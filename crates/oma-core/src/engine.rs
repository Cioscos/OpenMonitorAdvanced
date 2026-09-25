//! Parallel provider sampling with one shared deadline, schema revisions and history.
use std::time::{Duration, Instant};

use crate::history::History;
use crate::model::{Schema, Snapshot};
use crate::provider::{Inventory, Provider};
use crate::sanitize::sanitize_sensor;
use crate::stats::Stats;
use crate::worker::Worker;

pub fn backoff_ms(failures: u32) -> u64 {
    (5_000u64 << failures.saturating_sub(1).min(4)).min(60_000)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TickOutput {
    pub snapshot: Snapshot,
    pub schema: Option<Schema>,
}

struct Slot {
    worker: Worker,
    inventory: Inventory,
    last: Vec<Option<f64>>,
    /// Set once a pending request has already missed one deadline: the next
    /// miss in a row means the provider is still hung, so its values are
    /// cleared instead of being republished forever (spec §4.1/§8).
    timed_out: bool,
}

pub struct Engine {
    slots: Vec<Slot>,
    schema: Schema,
    history: History,
    /// Min/max/average since start, same retention rule as `history`.
    stats: Stats,
    /// Unix time of the first tick: when monitoring started (not the window).
    started_at_ms: Option<u64>,
    seq: u64,
}

impl Engine {
    pub fn new(providers: Vec<Box<dyn Provider>>, history_capacity: usize) -> Self {
        Self {
            slots: providers
                .into_iter()
                .map(|p| Slot {
                    worker: Worker::spawn(p),
                    inventory: Inventory::default(),
                    last: Vec::new(),
                    timed_out: false,
                })
                .collect(),
            schema: Schema::default(),
            history: History::new(history_capacity),
            stats: Stats::new(),
            started_at_ms: None,
            seq: 0,
        }
    }
    pub fn schema(&self) -> &Schema {
        &self.schema
    }
    pub fn history(&self) -> &History {
        &self.history
    }
    pub fn sequence(&self) -> u64 {
        self.seq
    }
    pub fn stats(&self) -> &Stats {
        &self.stats
    }
    pub fn stats_mut(&mut self) -> &mut Stats {
        &mut self.stats
    }
    /// `timestamp_ms` of the first tick; `None` before it.
    pub fn started_at_ms(&self) -> Option<u64> {
        self.started_at_ms
    }

    /// `timestamp_ms` is Unix time for display; `monotonic_ms` drives retry deadlines.
    pub fn tick(&mut self, timestamp_ms: u64, monotonic_ms: u64) -> TickOutput {
        // One budget for the entire cycle, not N sequential provider timeouts.
        let deadline = Instant::now() + Duration::from_millis(200);
        self.started_at_ms.get_or_insert(timestamp_ms);
        for slot in &mut self.slots {
            slot.worker.start(monotonic_ms);
        }
        let mut changed = self.schema.revision == 0;
        for slot in &mut self.slots {
            match slot.worker.finish(deadline) {
                Some(sample) => {
                    changed |= slot.inventory != sample.inventory;
                    slot.inventory = sample.inventory;
                    slot.last = sample.values;
                    slot.timed_out = false;
                }
                // A timeout retains the last values for one cycle only (§4.1); a
                // request still hung on the next tick means the provider is
                // degraded, so its sensors go back to unavailable (§8). No extra
                // worker/request is spawned while busy.
                None if slot.timed_out => {
                    slot.last = vec![None; slot.last.len()];
                }
                None => slot.timed_out = true,
            }
        }
        if changed {
            let retained: Vec<String> = self
                .schema
                .sensors
                .iter()
                .filter(|old| {
                    self.slots
                        .iter()
                        .flat_map(|slot| &slot.inventory.sensors)
                        .any(|new| {
                            old.id == new.id && old.source == new.source && old.unit == new.unit
                        })
                })
                .map(|sensor| sensor.id.clone())
                .collect();
            // Series and statistics survive only for sensors whose id, source
            // and unit are unchanged; the second call adds the new sensors.
            self.history.set_sensors(&retained);
            self.stats.set_sensors(&retained);
            self.schema = Schema {
                revision: self.schema.revision + 1,
                devices: self
                    .slots
                    .iter()
                    .flat_map(|s| s.inventory.devices.iter().cloned())
                    .collect(),
                sensors: self
                    .slots
                    .iter()
                    .flat_map(|s| s.inventory.sensors.iter().cloned())
                    .collect(),
            };
            let ids: Vec<String> = self.schema.sensors.iter().map(|s| s.id.clone()).collect();
            self.history.set_sensors(&ids);
            self.stats.set_sensors(&ids);
        }
        let values: Vec<_> = self
            .slots
            .iter()
            .flat_map(|s| s.last.iter().copied())
            .zip(&self.schema.sensors)
            .map(|(value, sensor)| sanitize_sensor(sensor, value))
            .collect();
        self.history.push(timestamp_ms, &values);
        self.stats.push(&values);
        self.seq += 1;
        TickOutput {
            snapshot: Snapshot {
                revision: self.schema.revision,
                seq: self.seq,
                timestamp_ms,
                values,
            },
            schema: changed.then(|| self.schema.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
    use crate::provider::ProviderError;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    type PollResult = Result<Vec<Option<f64>>, ProviderError>;

    #[derive(Default)]
    struct Script {
        inventory: Inventory,
        discover_errors: VecDeque<ProviderError>,
        polls: VecDeque<PollResult>,
        discover_calls: usize,
        poll_calls: usize,
    }

    struct Fake {
        name: &'static str,
        script: Arc<Mutex<Script>>,
    }

    impl Provider for Fake {
        fn name(&self) -> &'static str {
            self.name
        }

        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            let mut s = self.script.lock().unwrap();
            s.discover_calls += 1;
            match s.discover_errors.pop_front() {
                Some(e) => Err(e),
                None => Ok(s.inventory.clone()),
            }
        }

        fn poll(&mut self) -> PollResult {
            let mut s = self.script.lock().unwrap();
            s.poll_calls += 1;
            let n = s.inventory.sensors.len();
            s.polls
                .pop_front()
                .unwrap_or_else(|| Ok(vec![Some(1.0); n]))
        }
    }

    fn inventory(device: &str, sensors: &[&str]) -> Inventory {
        Inventory {
            devices: vec![Device {
                id: device.into(),
                kind: DeviceKind::Cpu,
                name: device.into(),
                vendor: None,
                properties: Default::default(),
            }],
            sensors: sensors
                .iter()
                .map(|n| {
                    Sensor::new(
                        device,
                        SensorKind::Load,
                        n,
                        Unit::Percent,
                        Label::new("test"),
                        Source::Mock,
                    )
                })
                .collect(),
        }
    }

    fn fake(name: &'static str, inv: Inventory) -> (Box<dyn Provider>, Arc<Mutex<Script>>) {
        let script = Arc::new(Mutex::new(Script {
            inventory: inv,
            ..Default::default()
        }));
        (
            Box::new(Fake {
                name,
                script: script.clone(),
            }),
            script,
        )
    }

    fn failed() -> ProviderError {
        ProviderError::Failed("boom".into())
    }

    #[test]
    fn first_tick_discovers_and_publishes_schema() {
        let (p, _) = fake("a", inventory("dev/a", &["x", "y"]));
        let mut e = Engine::new(vec![p], 10);
        let out = e.tick(1_000, 1_000);
        let schema = out.schema.expect("schema on first tick");
        assert_eq!(schema.revision, 1);
        assert_eq!(schema.sensors.len(), 2);
        assert_eq!(out.snapshot.revision, 1);
        assert_eq!(out.snapshot.seq, 1);
        assert_eq!(out.snapshot.values, vec![Some(1.0), Some(1.0)]);
        assert!(e.tick(2_000, 2_000).schema.is_none());
    }

    #[test]
    fn engine_without_providers_publishes_an_empty_schema() {
        let mut e = Engine::new(Vec::new(), 10);
        let out = e.tick(0, 0);
        assert_eq!(out.schema.map(|s| s.revision), Some(1));
        assert!(out.snapshot.values.is_empty());
    }

    #[test]
    fn implausible_values_become_none() {
        let (p, script) = fake("a", inventory("dev/a", &["x", "y"]));
        script
            .lock()
            .unwrap()
            .polls
            .push_back(Ok(vec![Some(150.0), Some(f64::NAN)]));
        let mut e = Engine::new(vec![p], 10);
        assert_eq!(e.tick(0, 0).snapshot.values, vec![None, None]);
    }

    #[test]
    fn failing_provider_backs_off_and_recovers() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        script.lock().unwrap().polls.push_back(Err(failed()));
        let mut e = Engine::new(vec![p], 10);
        assert_eq!(e.tick(0, 0).snapshot.values, vec![None]);
        assert_eq!(e.tick(1_000, 1_000).snapshot.values, vec![None]);
        assert_eq!(script.lock().unwrap().discover_calls, 1);
        assert_eq!(e.tick(5_000, 5_000).snapshot.values, vec![Some(1.0)]);
        assert_eq!(script.lock().unwrap().discover_calls, 2);
    }

    #[test]
    fn repeated_failures_grow_the_backoff() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        script
            .lock()
            .unwrap()
            .discover_errors
            .extend([failed(), failed()]);
        let mut e = Engine::new(vec![p], 10);
        e.tick(0, 0);
        e.tick(5_000, 5_000);
        e.tick(14_999, 14_999);
        assert_eq!(script.lock().unwrap().discover_calls, 2);
        let out = e.tick(15_000, 15_000);
        assert_eq!(script.lock().unwrap().discover_calls, 3);
        assert_eq!(out.schema.map(|s| s.revision), Some(2));
        assert_eq!(out.snapshot.values, vec![Some(1.0)]);
    }

    #[test]
    fn rediscover_rebuilds_schema_and_keeps_history_by_id() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        let mut e = Engine::new(vec![p], 10);
        e.tick(1_000, 1_000);
        {
            let mut s = script.lock().unwrap();
            s.polls.push_back(Err(ProviderError::Rediscover));
            s.inventory = inventory("dev/a", &["x", "z"]);
        }
        let out = e.tick(2_000, 2_000);
        assert!(out.schema.is_none());
        assert_eq!(out.snapshot.values, vec![None]);
        let out = e.tick(3_000, 3_000);
        assert_eq!(out.schema.map(|s| s.revision), Some(2));
        assert_eq!(out.snapshot.values, vec![Some(1.0), Some(1.0)]);
        let w = e
            .history()
            .window(&["dev/a/load/x".into(), "dev/a/load/z".into()], 0);
        assert_eq!(w.timestamps_ms, vec![1_000, 2_000, 3_000]);
        assert_eq!(w.series[0], vec![Some(1.0), None, Some(1.0)]);
        assert_eq!(w.series[1], vec![None, None, Some(1.0)]);
    }

    #[test]
    fn rediscover_storm_is_backed_off() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        script
            .lock()
            .unwrap()
            .polls
            .extend((0..6).map(|_| Err(ProviderError::Rediscover)));
        let discovers = || script.lock().unwrap().discover_calls;
        let mut e = Engine::new(vec![p], 10);
        // Three rediscoveries in a row are free: discover runs on every tick.
        for t in [0, 1_000, 2_000] {
            assert_eq!(e.tick(t, t).snapshot.values, vec![None]);
        }
        assert_eq!(discovers(), 3);
        // The 4th Rediscover (at 3 s) arms backoff_ms(1) = 5 s: nothing runs before 8 s.
        for t in [3_000, 4_000, 5_000, 6_000, 7_000] {
            assert_eq!(e.tick(t, t).snapshot.values, vec![None]);
        }
        assert_eq!(discovers(), 4);
        // The 5th (at 8 s) arms backoff_ms(2) = 10 s: next attempt at 18 s.
        e.tick(8_000, 8_000);
        assert_eq!(discovers(), 5);
        for t in (9_000..18_000).step_by(1_000) {
            assert_eq!(e.tick(t, t).snapshot.values, vec![None]);
        }
        assert_eq!(discovers(), 5);
        // The 6th (at 18 s) arms backoff_ms(3) = 20 s: next attempt at 38 s.
        e.tick(18_000, 18_000);
        e.tick(37_999, 37_999);
        assert_eq!(discovers(), 6);
        // The script is exhausted: the retry at 38 s succeeds.
        assert_eq!(e.tick(38_000, 38_000).snapshot.values, vec![Some(1.0)]);
        assert_eq!(discovers(), 7);
    }

    #[test]
    fn successful_poll_resets_the_rediscover_streak() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        let rediscover = || Err(ProviderError::Rediscover);
        script.lock().unwrap().polls.extend([
            rediscover(),
            rediscover(),
            rediscover(),
            Ok(vec![Some(2.0)]),
            rediscover(),
            rediscover(),
            rediscover(),
        ]);
        let mut e = Engine::new(vec![p], 10);
        for t in [0, 1_000, 2_000] {
            assert_eq!(e.tick(t, t).snapshot.values, vec![None]);
        }
        assert_eq!(e.tick(3_000, 3_000).snapshot.values, vec![Some(2.0)]);
        // After the successful poll the next three Rediscover are free again.
        for t in [4_000, 5_000, 6_000] {
            assert_eq!(e.tick(t, t).snapshot.values, vec![None]);
        }
        assert_eq!(e.tick(7_000, 7_000).snapshot.values, vec![Some(1.0)]);
        assert_eq!(script.lock().unwrap().discover_calls, 7);
    }

    #[test]
    fn wrong_value_count_degrades_provider() {
        let (p, script) = fake("a", inventory("dev/a", &["x", "y"]));
        script.lock().unwrap().polls.push_back(Ok(vec![Some(1.0)]));
        let mut e = Engine::new(vec![p], 10);
        assert_eq!(e.tick(0, 0).snapshot.values, vec![None, None]);
        e.tick(1_000, 1_000);
        assert_eq!(script.lock().unwrap().poll_calls, 1);
    }

    #[test]
    fn failure_of_one_provider_does_not_affect_another() {
        let (a, script_a) = fake("a", inventory("dev/a", &["x"]));
        let (b, _) = fake("b", inventory("dev/b", &["y"]));
        script_a.lock().unwrap().polls.push_back(Err(failed()));
        let mut e = Engine::new(vec![a, b], 10);
        assert_eq!(e.tick(0, 0).snapshot.values, vec![None, Some(1.0)]);
    }

    #[test]
    fn successful_rediscovery_does_not_reset_poll_backoff() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        script
            .lock()
            .unwrap()
            .polls
            .extend([Err(failed()), Err(failed())]);
        let mut e = Engine::new(vec![p], 10);
        e.tick(100_000, 0);
        e.tick(1_000, 5_000); // Wall clock moves backwards; retry still occurs.
        e.tick(2_000, 14_999);
        assert_eq!(script.lock().unwrap().poll_calls, 2);
        assert_eq!(e.tick(3_000, 15_000).snapshot.values, vec![Some(1.0)]);
    }

    struct Blocked(std::sync::mpsc::Receiver<()>);
    impl Provider for Blocked {
        fn name(&self) -> &'static str {
            "blocked"
        }
        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            let _ = self.0.recv();
            Ok(Inventory::default())
        }
        fn poll(&mut self) -> PollResult {
            Ok(Vec::new())
        }
    }
    #[test]
    fn blocked_discovery_does_not_block_other_providers_or_drop() {
        let (release, wait) = std::sync::mpsc::channel();
        let (fast, _) = fake("fast", inventory("dev/fast", &["x"]));
        let mut e = Engine::new(vec![Box::new(Blocked(wait)), fast], 10);
        let start = Instant::now();
        assert_eq!(e.tick(0, 0).snapshot.values, vec![Some(1.0)]);
        assert!(start.elapsed() < Duration::from_secs(1));
        let start = Instant::now();
        drop(e);
        assert!(start.elapsed() < Duration::from_millis(100));
        drop(release);
    }

    struct SlowPoll {
        wait: std::sync::mpsc::Receiver<()>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Provider for SlowPoll {
        fn name(&self) -> &'static str {
            "slow-poll"
        }
        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            Ok(inventory("dev/slow", &["x"]))
        }
        fn poll(&mut self) -> PollResult {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
                let _ = self.wait.recv();
            }
            Ok(vec![Some(42.0)])
        }
    }
    #[test]
    fn timed_out_poll_reuses_last_value_without_queuing_more_work() {
        let (release, wait) = std::sync::mpsc::channel();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let slow = SlowPoll {
            wait,
            calls: calls.clone(),
        };
        let (fast, script) = fake("fast", inventory("dev/fast", &["x"]));
        let mut e = Engine::new(vec![Box::new(slow), fast], 10);
        assert_eq!(e.tick(0, 0).snapshot.values, vec![Some(42.0), Some(1.0)]);
        script.lock().unwrap().polls.push_back(Ok(vec![Some(2.0)]));
        assert_eq!(
            e.tick(1_000, 1_000).snapshot.values,
            vec![Some(42.0), Some(2.0)]
        );
        // A second consecutive miss on the same request means the provider is
        // still hung: its values must go back to None, not stay frozen forever.
        assert_eq!(e.tick(2_000, 2_000).snapshot.values, vec![None, Some(1.0)]);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        drop(e);
        drop(release);
    }

    #[test]
    fn source_change_resets_only_the_changed_series() {
        let (provider, script) = fake("gpu", inventory("dev/a", &["x", "y"]));
        let mut engine = Engine::new(vec![provider], 10);
        engine.tick(1_000, 1_000);
        {
            let mut script = script.lock().unwrap();
            script.inventory.sensors[0].source = Source::Nvml;
            script.polls.push_back(Err(ProviderError::Rediscover));
        }
        engine.tick(2_000, 2_000);
        engine.tick(3_000, 3_000);
        let history = engine
            .history()
            .window(&["dev/a/load/x".into(), "dev/a/load/y".into()], 0);
        assert_eq!(history.series[0], vec![None, None, Some(1.0)]);
        assert_eq!(history.series[1], vec![Some(1.0), None, Some(1.0)]);
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn stats_cover_every_tick_since_start() {
        let (p, script) = fake("a", inventory("dev/a", &["x", "y"]));
        script.lock().unwrap().polls.extend([
            Ok(vec![Some(10.0), None]),
            Ok(vec![Some(30.0), Some(150.0)]),
            Ok(vec![Some(20.0), Some(5.0)]),
        ]);
        let mut e = Engine::new(vec![p], 10);
        for t in [1_000, 2_000, 3_000] {
            e.tick(t, t);
        }
        let got = e.stats().get(&ids(&["dev/a/load/x", "dev/a/load/y"]));
        let x = got[0].expect("x has samples");
        assert_eq!((x.min, x.max, x.avg, x.count), (10.0, 30.0, 20.0, 3));
        // 150 % is implausible and sanitized away before it reaches the stats.
        let y = got[1].expect("y has samples");
        assert_eq!((y.min, y.max, y.avg, y.count), (5.0, 5.0, 5.0, 1));
    }

    #[test]
    fn stats_follow_the_history_retention_rule() {
        let (p, script) = fake("a", inventory("dev/a", &["x", "y"]));
        let mut e = Engine::new(vec![p], 10);
        e.tick(1_000, 1_000);
        {
            let mut s = script.lock().unwrap();
            s.inventory.sensors[0].source = Source::Nvml;
            s.inventory.sensors.push(Sensor::new(
                "dev/a",
                SensorKind::Load,
                "z",
                Unit::Percent,
                Label::new("test"),
                Source::Mock,
            ));
            s.polls.push_back(Err(ProviderError::Rediscover));
        }
        e.tick(2_000, 2_000);
        e.tick(3_000, 3_000);
        let got = e
            .stats()
            .get(&ids(&["dev/a/load/x", "dev/a/load/y", "dev/a/load/z"]));
        // x changed source: its statistics restart with the new schema.
        assert_eq!(got[0].map(|s| s.count), Some(1));
        // y is unchanged: the sample of the first tick is still counted.
        assert_eq!(got[1].map(|s| s.count), Some(2));
        assert_eq!(got[2].map(|s| s.count), Some(1));
    }

    #[test]
    fn stats_reset_through_the_engine() {
        let (p, _) = fake("a", inventory("dev/a", &["x", "y"]));
        let mut e = Engine::new(vec![p], 10);
        e.tick(1_000, 1_000);
        e.stats_mut().reset(&ids(&["dev/a/load/x"]));
        let got = e.stats().get(&ids(&["dev/a/load/x", "dev/a/load/y"]));
        assert_eq!(got[0], None);
        assert_eq!(got[1].map(|s| s.count), Some(1));
        e.tick(2_000, 2_000);
        let got = e.stats().get(&ids(&["dev/a/load/x"]));
        assert_eq!(got[0].map(|s| s.count), Some(1));
    }

    #[test]
    fn started_at_is_the_timestamp_of_the_first_tick() {
        let mut e = Engine::new(Vec::new(), 10);
        assert_eq!(e.started_at_ms(), None);
        e.tick(5_000, 0);
        e.tick(6_000, 1_000);
        assert_eq!(e.started_at_ms(), Some(5_000));
    }

    #[test]
    fn backoff_doubles_up_to_one_minute() {
        assert_eq!(
            [1, 2, 3, 4, 5, 10].map(backoff_ms),
            [5_000, 10_000, 20_000, 40_000, 60_000, 60_000]
        );
    }
}
