//! Parallel provider sampling with one shared deadline, schema revisions and history.
use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::history::History;
use crate::model::{Device, Schema, Snapshot};
use crate::provider::{Inventory, Provider};
use crate::rules::{HealthClock, HealthReport, LevelEntry, Rule, RuleEngine, RuleStatus};
use crate::sanitize::{sanitize_sensor, DiscardLog};
use crate::stats::Stats;
use crate::worker::Worker;

pub fn backoff_ms(failures: u32) -> u64 {
    (5_000u64 << failures.saturating_sub(1).min(4)).min(60_000)
}

pub use crate::provider::Quality;

#[derive(Debug, Clone, PartialEq)]
pub struct TickOutput {
    pub snapshot: Snapshot,
    pub schema: Option<Schema>,
    /// One entry per `snapshot.values`, same order.
    pub quality: Vec<Quality>,
    /// The health report, when this tick changed it; always on the first tick.
    pub health: Option<HealthReport>,
    /// Instances that entered a more severe level in this tick.
    pub entries: Vec<LevelEntry>,
    /// The tick's `monotonic_ms`, the clock of the rules' timers (and of the
    /// toast cooldown).
    pub monotonic_ms: u64,
}

struct Slot {
    worker: Worker,
    inventory: Inventory,
    last: Vec<Option<f64>>,
    /// One entry per value of `last`: whether it is a new measurement (the
    /// provider reported a repeat, suspended it, or the slot just
    /// republished it after a missed deadline).
    quality: Vec<Quality>,
    /// Set once a pending request has already missed one deadline: the next
    /// miss in a row means the provider is still hung, so its values are
    /// cleared instead of being republished forever (spec §4.1/§8).
    timed_out: bool,
    /// One entry per sensor of `inventory`, recomputed on every schema
    /// rebuild: `false` when a provider earlier in the list already claimed
    /// that sensor id (spec §M4 merge rule). `tick` uses it to line up poll
    /// values with the merged schema even on ticks where the schema itself
    /// does not change.
    keep: Vec<bool>,
}

/// Merges devices with the same id across providers, in provider order: the
/// first provider to expose an id wins its `name`, `vendor` and `kind`, and
/// its properties take precedence over later duplicates' (spec §M4).
fn merge_devices(slots: &[Slot]) -> Vec<Device> {
    let mut order: Vec<String> = Vec::new();
    let mut merged: std::collections::HashMap<String, Device> = std::collections::HashMap::new();
    for slot in slots {
        for device in &slot.inventory.devices {
            match merged.get_mut(&device.id) {
                Some(existing) => {
                    for (key, value) in &device.properties {
                        existing
                            .properties
                            .entry(key.clone())
                            .or_insert_with(|| value.clone());
                    }
                }
                None => {
                    order.push(device.id.clone());
                    merged.insert(device.id.clone(), device.clone());
                }
            }
        }
    }
    order
        .into_iter()
        .map(|id| merged.remove(&id).expect("id was just inserted"))
        .collect()
}

pub struct Engine {
    slots: Vec<Slot>,
    schema: Schema,
    history: History,
    /// Min/max/average since start, same retention rule as `history`.
    stats: Stats,
    /// The tick's values as `history` and `stats` take them: a suspended
    /// value is a last reading, not a measurement, so it is absent here.
    /// Kept between ticks to reuse its buffer.
    measured: Vec<Option<f64>>,
    /// Unix time of the first tick: when monitoring started (not the window).
    started_at_ms: Option<u64>,
    /// Rate limit of the "discarding implausible value" debug line.
    discards: DiscardLog,
    rules: RuleEngine,
    /// `monotonic_ms` of the latest tick, the epoch of the rules' timers.
    last_monotonic_ms: u64,
    seq: u64,
    /// Snapshot and quality of the latest tick, for the sensor report.
    latest: Option<(Snapshot, Vec<Quality>)>,
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
                    quality: Vec::new(),
                    timed_out: false,
                    keep: Vec::new(),
                })
                .collect(),
            schema: Schema::default(),
            history: History::new(history_capacity),
            stats: Stats::new(),
            measured: Vec::new(),
            started_at_ms: None,
            discards: DiscardLog::default(),
            rules: RuleEngine::new(),
            last_monotonic_ms: 0,
            seq: 0,
            latest: None,
        }
    }
    pub fn schema(&self) -> &Schema {
        &self.schema
    }
    pub fn history(&self) -> &History {
        &self.history
    }
    /// Snapshot and quality of the latest tick; `None` before the first.
    pub fn latest(&self) -> Option<(&Snapshot, &[Quality])> {
        self.latest
            .as_ref()
            .map(|(snapshot, quality)| (snapshot, quality.as_slice()))
    }
    pub fn sequence(&self) -> u64 {
        self.seq
    }
    /// Resizes the history to the new sampling interval (one hour of
    /// samples). The statistics are not affected.
    pub fn set_history_capacity(&mut self, capacity: usize) {
        self.history.set_capacity(capacity);
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

    /// Replaces the alert rules; they take effect on the next tick.
    pub fn set_rules(&mut self, rules: Vec<Rule>) {
        self.rules.set_rules(rules);
    }
    /// The sampling interval, for the rules' suspend detection.
    pub fn set_interval_ms(&mut self, interval_ms: u64) {
        self.rules.set_interval_ms(interval_ms);
    }
    /// The latest health report.
    pub fn health(&self) -> &HealthReport {
        self.rules.report()
    }
    /// How long the overall level has lasted, at the latest tick.
    pub fn health_clock(&self) -> HealthClock {
        self.rules.clock(self.last_monotonic_ms)
    }
    /// Every rule with its instances, for the rules settings.
    pub fn rule_status(&self) -> Vec<RuleStatus> {
        self.rules.status()
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
                    slot.quality = sample.quality;
                    slot.timed_out = false;
                }
                // A timeout retains the last values for one cycle only (§4.1); a
                // request still hung on the next tick means the provider is
                // degraded, so its sensors go back to unavailable (§8). No extra
                // worker/request is spawned while busy.
                None if slot.timed_out => {
                    slot.last = vec![None; slot.last.len()];
                    slot.quality = vec![Quality::Fresh; slot.last.len()];
                }
                None => {
                    slot.timed_out = true;
                    for quality in &mut slot.quality {
                        if *quality != Quality::Suspended {
                            *quality = Quality::Held;
                        }
                    }
                }
            }
        }
        if changed {
            // A sensor id already claimed by an earlier provider is dropped,
            // together with its value (spec §M4): `keep[i]` tells `i` apart.
            let mut seen_ids: HashSet<String> = HashSet::new();
            let mut new_sensors = Vec::new();
            for slot in &mut self.slots {
                let keep: Vec<bool> = slot
                    .inventory
                    .sensors
                    .iter()
                    .map(|sensor| seen_ids.insert(sensor.id.clone()))
                    .collect();
                for (sensor, &kept) in slot.inventory.sensors.iter().zip(&keep) {
                    if kept {
                        new_sensors.push(sensor.clone());
                    }
                }
                slot.keep = keep;
            }
            // Series and statistics survive only for sensors whose id, source
            // and unit are unchanged among the sensors that win the new
            // merge; comparing against the raw (pre-merge) inventories would
            // wrongly keep history when the old winner becomes a discarded
            // duplicate. The second call adds the new sensors.
            let retained: Vec<String> = self
                .schema
                .sensors
                .iter()
                .filter(|old| {
                    new_sensors.iter().any(|new| {
                        old.id == new.id && old.source == new.source && old.unit == new.unit
                    })
                })
                .map(|sensor| sensor.id.clone())
                .collect();
            self.history.set_sensors(&retained);
            self.stats.set_sensors(&retained);
            self.schema = Schema {
                revision: self.schema.revision + 1,
                devices: merge_devices(&self.slots),
                sensors: new_sensors,
            };
            let ids: Vec<String> = self.schema.sensors.iter().map(|s| s.id.clone()).collect();
            self.history.set_sensors(&ids);
            self.stats.set_sensors(&ids);
            self.discards.retain(&ids);
        }
        let discards = &mut self.discards;
        let measured = &mut self.measured;
        measured.clear();
        let mut values = Vec::with_capacity(self.schema.sensors.len());
        let mut quality = Vec::with_capacity(self.schema.sensors.len());
        let kept_values = self.slots.iter().flat_map(|slot| {
            slot.keep
                .iter()
                .enumerate()
                .filter(|&(_, &kept)| kept)
                .map(move |(i, _)| {
                    (
                        slot.last.get(i).copied().flatten(),
                        slot.quality.get(i).copied().unwrap_or(Quality::Fresh),
                    )
                })
        });
        for ((value, slot_quality), sensor) in kept_values.zip(&self.schema.sensors) {
            let clean = sanitize_sensor(sensor, value);
            if let (Some(raw), None) = (value, clean) {
                if discards.should_log(&sensor.id, monotonic_ms) {
                    tracing::debug!(
                        sensor = %sensor.id,
                        unit = ?sensor.unit,
                        value = raw,
                        "discarding implausible value"
                    );
                }
            }
            values.push(clean);
            measured.push(clean.filter(|_| slot_quality != Quality::Suspended));
            quality.push(match slot_quality {
                Quality::Suspended => Quality::Suspended,
                Quality::Held if clean.is_some() => Quality::Held,
                _ => Quality::Fresh,
            });
        }
        self.history.push(timestamp_ms, &self.measured);
        self.stats.push(&self.measured);
        self.seq += 1;
        self.last_monotonic_ms = monotonic_ms;
        // The rules see the sanitized values; a new merged schema is flagged
        // even when its revision number repeats (a reconnecting service).
        let evaluation = self.rules.evaluate(
            &self.schema,
            changed,
            &values,
            &quality,
            monotonic_ms,
            timestamp_ms,
        );
        let snapshot = Snapshot {
            revision: self.schema.revision,
            seq: self.seq,
            timestamp_ms,
            values,
        };
        // Reuses the buffers of the previous tick.
        match &mut self.latest {
            Some((last, last_quality)) => {
                last.revision = snapshot.revision;
                last.seq = snapshot.seq;
                last.timestamp_ms = snapshot.timestamp_ms;
                last.values.clone_from(&snapshot.values);
                last_quality.clone_from(&quality);
            }
            None => self.latest = Some((snapshot.clone(), quality.clone())),
        }
        TickOutput {
            snapshot,
            schema: changed.then(|| self.schema.clone()),
            quality,
            health: evaluation.report,
            entries: evaluation.entries,
            monotonic_ms,
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
        /// What `Provider::repeated` answers after each poll.
        repeated: bool,
        /// What `Provider::quality` answers after each poll.
        quality: Option<Vec<Quality>>,
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

        fn repeated(&self) -> bool {
            self.script.lock().unwrap().repeated
        }

        fn quality(&self) -> Option<Vec<Quality>> {
            self.script.lock().unwrap().quality.clone()
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
    fn latest_returns_last_tick_snapshot() {
        let (p, _) = fake("a", inventory("dev/a", &["x", "y"]));
        let mut e = Engine::new(vec![p], 10);
        assert!(e.latest().is_none());
        e.tick(1_000, 1_000);
        let out = e.tick(2_000, 2_000);
        let (snapshot, quality) = e.latest().expect("latest after a tick");
        assert_eq!(snapshot, &out.snapshot);
        assert_eq!(snapshot.revision, out.snapshot.revision);
        assert_eq!(snapshot.seq, 2);
        assert_eq!(quality, out.quality.as_slice());
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

    /// One `cpu/0` temperature sensor, `package`, in °C.
    fn temperature_inventory() -> Inventory {
        Inventory {
            devices: vec![Device {
                id: "cpu/0".into(),
                kind: DeviceKind::Cpu,
                name: "cpu".into(),
                vendor: None,
                properties: Default::default(),
            }],
            sensors: vec![Sensor::new(
                "cpu/0",
                SensorKind::Temperature,
                "package",
                Unit::Celsius,
                Label::new("test"),
                Source::Mock,
            )],
        }
    }

    /// Warns at 80 °C at once.
    fn hot_rule() -> crate::rules::Rule {
        use crate::rules::{Condition, Hysteresis, LevelSpec, Notify, Rule, Target, Threshold};
        Rule {
            id: "custom-00000000-0000-4000-8000-000000000001".into(),
            target: Target::Sensor {
                sensor: "cpu/0/temperature/package".into(),
            },
            unit: Unit::Celsius,
            condition: Condition::Above,
            warn: Some(LevelSpec {
                threshold: Some(Threshold::Fixed { fixed: 80.0 }),
                duration_s: 0,
            }),
            crit: None,
            hysteresis: Hysteresis::default(),
            enabled: true,
            notify: Notify::default(),
        }
    }

    #[test]
    fn tick_reports_health_on_first_tick_then_only_on_change() {
        let (p, script) = fake("a", temperature_inventory());
        script.lock().unwrap().polls.extend([
            Ok(vec![Some(50.0)]),
            Ok(vec![Some(51.0)]),
            Ok(vec![Some(90.0)]),
            Ok(vec![Some(90.0)]),
        ]);
        let mut e = Engine::new(vec![p], 10);
        e.set_rules(vec![hot_rule()]);
        let first = e.tick(1_000, 1_000);
        let report = first.health.expect("health on the first tick");
        assert_eq!(report.revision, 1);
        assert!(first.entries.is_empty());
        assert_eq!(e.health(), &report);
        // A different value that changes nothing on screen: no new report.
        let second = e.tick(2_000, 2_000);
        assert!(second.health.is_none());
        // The alarm shows up: a new report and an entry.
        let third = e.tick(3_000, 3_000);
        let report = third.health.expect("health changes with the alarm");
        assert_eq!(report.revision, 2);
        assert_eq!(report.alerts.len(), 1);
        assert_eq!(third.entries.len(), 1);
        assert_eq!(third.entries[0].level, crate::rules::Level::Warn);
        // Stays in alarm at the same reading: no report, no entry.
        let fourth = e.tick(4_000, 4_000);
        assert!(fourth.health.is_none());
        assert!(fourth.entries.is_empty());
        assert_eq!(e.health().revision, 2);
    }

    #[test]
    fn rules_see_sanitized_values() {
        let (p, script) = fake("a", temperature_inventory());
        // 200 °C is implausible: the snapshot has no value and the rule none.
        script
            .lock()
            .unwrap()
            .polls
            .push_back(Ok(vec![Some(200.0)]));
        let mut e = Engine::new(vec![p], 10);
        e.set_rules(vec![hot_rule()]);
        let out = e.tick(1_000, 1_000);
        assert_eq!(out.snapshot.values, vec![None]);
        assert!(out.entries.is_empty());
        let report = out.health.expect("first tick");
        assert!(report.alerts.is_empty());
        assert_eq!(report.unavailable_targets.len(), 1);
    }

    #[test]
    fn set_rules_takes_effect_on_the_next_tick() {
        let (p, script) = fake("a", temperature_inventory());
        script.lock().unwrap().polls.extend([
            Ok(vec![Some(90.0)]),
            Ok(vec![Some(90.0)]),
            Ok(vec![Some(90.0)]),
        ]);
        let mut e = Engine::new(vec![p], 10);
        let out = e.tick(1_000, 1_000);
        assert!(out.health.is_some());
        assert!(out.entries.is_empty());
        assert!(e.rule_status().is_empty());
        e.set_rules(vec![hot_rule()]);
        // The new rule is listed at once, without instances.
        let status = e.rule_status();
        assert_eq!(status.len(), 1);
        assert!(status[0].instances.is_empty());
        let out = e.tick(2_000, 2_000);
        assert_eq!(out.entries.len(), 1);
        assert_eq!(
            out.health
                .expect("the rules changed the report")
                .alerts
                .len(),
            1
        );
        assert_eq!(e.rule_status()[0].instances.len(), 1);
        assert!(e.tick(3_000, 3_000).entries.is_empty());
    }

    #[test]
    fn health_clock_uses_the_monotonic_time_of_the_latest_tick() {
        let (p, _) = fake("a", temperature_inventory());
        let mut e = Engine::new(vec![p], 10);
        e.tick(1_000_000, 10_000);
        e.tick(1_001_000, 14_000);
        let clock = e.health_clock();
        assert_eq!(clock.revision, e.health().revision);
        assert_eq!(clock.level_elapsed_ms, 4_000);
    }

    #[test]
    fn tick_output_carries_the_rules_clock() {
        // The toast cooldown runs on the same monotonic clock as the rules.
        let (p, _) = fake("a", temperature_inventory());
        let mut e = Engine::new(vec![p], 10);
        assert_eq!(e.tick(1_000_000, 10_000).monotonic_ms, 10_000);
        assert_eq!(e.tick(1_001_000, 14_000).monotonic_ms, 14_000);
    }

    #[test]
    fn rule_state_survives_a_schema_change() {
        let (p, script) = fake("a", temperature_inventory());
        let mut e = Engine::new(vec![p], 10);
        e.set_rules(vec![hot_rule()]);
        script.lock().unwrap().polls.push_back(Ok(vec![Some(90.0)]));
        assert_eq!(e.tick(1_000, 1_000).entries.len(), 1);
        {
            let mut s = script.lock().unwrap();
            s.polls.push_back(Err(ProviderError::Rediscover));
            let mut inventory = temperature_inventory();
            inventory.sensors.push(Sensor::new(
                "cpu/0",
                SensorKind::Temperature,
                "tdie",
                Unit::Celsius,
                Label::new("test"),
                Source::Mock,
            ));
            s.inventory = inventory;
        }
        e.tick(2_000, 2_000);
        script
            .lock()
            .unwrap()
            .polls
            .push_back(Ok(vec![Some(90.0), Some(40.0)]));
        let out = e.tick(3_000, 3_000);
        assert!(out.schema.is_some());
        // Still in alarm, no second entry for the same instance.
        assert!(out.entries.is_empty());
        assert_eq!(e.health().alerts.len(), 1);
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
    fn discarded_values_are_logged_at_most_once_a_minute_per_sensor() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        script
            .lock()
            .unwrap()
            .polls
            .extend([Ok(vec![Some(150.0)]), Ok(vec![Some(150.0)])]);
        let mut e = Engine::new(vec![p], 10);
        e.tick(0, 0);
        // The engine used this sensor's budget at 0 ms (monotonic time)...
        assert!(!e.discards.should_log("dev/a/load/x", 1_000));
        e.tick(1_000, 1_000);
        // ...and a new discard within the minute did not renew it.
        assert!(e.discards.should_log("dev/a/load/x", 60_000));
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
    fn a_new_history_capacity_resizes_the_history_and_leaves_the_stats_alone() {
        let (p, _) = fake("a", inventory("dev/a", &["x"]));
        let mut e = Engine::new(vec![p], 10);
        for t in 1..=6u64 {
            e.tick(t * 1_000, t * 1_000);
        }
        e.set_history_capacity(3);
        let id = ids(&["dev/a/load/x"]);
        assert_eq!(
            e.history().window(&id, 0).timestamps_ms,
            vec![4_000, 5_000, 6_000]
        );
        assert_eq!(e.stats().get(&id)[0].map(|s| s.count), Some(6));
        e.set_history_capacity(20);
        e.tick(7_000, 7_000);
        assert_eq!(e.history().len(), 4);
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
    fn duplicate_sensor_id_keeps_the_first_provider() {
        let (a, script_a) = fake("a", inventory("dev/a", &["x"]));
        let b_inv = Inventory {
            devices: vec![Device {
                id: "dev/a".into(),
                kind: DeviceKind::Cpu,
                name: "dev/a".into(),
                vendor: None,
                properties: Default::default(),
            }],
            sensors: vec![
                Sensor::new(
                    "dev/a",
                    SensorKind::Load,
                    "x",
                    Unit::Percent,
                    Label::new("test"),
                    Source::Lhm,
                ),
                Sensor::new(
                    "dev/a",
                    SensorKind::Load,
                    "y",
                    Unit::Percent,
                    Label::new("test"),
                    Source::Lhm,
                ),
            ],
        };
        let (b, script_b) = fake("b", b_inv);
        script_a
            .lock()
            .unwrap()
            .polls
            .push_back(Ok(vec![Some(1.0)]));
        script_b
            .lock()
            .unwrap()
            .polls
            .push_back(Ok(vec![Some(9.0), Some(2.0)]));
        let mut e = Engine::new(vec![a, b], 10);
        let out = e.tick(1_000, 1_000);
        let schema = out.schema.expect("schema on first tick");
        assert_eq!(schema.sensors.len(), 2);
        assert_eq!(schema.sensors[0].source, Source::Mock);
        assert_eq!(out.snapshot.values, vec![Some(1.0), Some(2.0)]);
    }

    #[test]
    fn fresh_poll_marks_values_fresh() {
        let (p, _) = fake("a", inventory("dev/a", &["x", "y"]));
        let mut e = Engine::new(vec![p], 10);
        let out = e.tick(0, 0);
        assert_eq!(out.quality, vec![Quality::Fresh, Quality::Fresh]);
        assert_eq!(out.quality.len(), out.snapshot.values.len());
    }

    #[test]
    fn timed_out_slot_marks_republished_values_held() {
        let (release, wait) = std::sync::mpsc::channel();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let slow = SlowPoll {
            wait,
            calls: calls.clone(),
        };
        let (fast, _) = fake("fast", inventory("dev/fast", &["x"]));
        let mut e = Engine::new(vec![Box::new(slow), fast], 10);
        let out = e.tick(0, 0);
        assert_eq!(out.quality, vec![Quality::Fresh, Quality::Fresh]);
        // First miss: the last value is republished and is no measurement.
        let out = e.tick(1_000, 1_000);
        assert_eq!(out.snapshot.values, vec![Some(42.0), Some(1.0)]);
        assert_eq!(out.quality, vec![Quality::Held, Quality::Fresh]);
        // Second miss: the value is gone, and an absent value is not "held".
        let out = e.tick(2_000, 2_000);
        assert_eq!(out.snapshot.values, vec![None, Some(1.0)]);
        assert_eq!(out.quality, vec![Quality::Fresh, Quality::Fresh]);
        drop(e);
        drop(release);
    }

    #[test]
    fn per_sensor_quality_marks_only_the_flagged_value() {
        let (p, script) = fake("a", inventory("d", &["a", "b"]));
        {
            let mut s = script.lock().unwrap();
            s.polls.push_back(Ok(vec![Some(1.0), Some(2.0)]));
            s.quality = Some(vec![Quality::Fresh, Quality::Held]);
        }
        let mut e = Engine::new(vec![p], 10);
        let out = e.tick(0, 0);
        assert_eq!(out.snapshot.values, vec![Some(1.0), Some(2.0)]);
        assert_eq!(out.quality, vec![Quality::Fresh, Quality::Held]);
    }

    #[test]
    fn suspended_is_reported_even_without_a_value() {
        let (p, script) = fake("a", inventory("d", &["a"]));
        {
            let mut s = script.lock().unwrap();
            s.polls.push_back(Ok(vec![None]));
            s.quality = Some(vec![Quality::Suspended]);
        }
        let mut e = Engine::new(vec![p], 10);
        let out = e.tick(0, 0);
        assert_eq!(out.snapshot.values, vec![None]);
        assert_eq!(out.quality, vec![Quality::Suspended]);
    }

    /// Ticks once per `(values, quality)` pair, one second apart from 1 s.
    fn tick_with_quality(
        e: &mut Engine,
        script: &Arc<Mutex<Script>>,
        ticks: &[(&[Option<f64>], &[Quality])],
    ) -> Vec<TickOutput> {
        (1u64..)
            .zip(ticks)
            .map(|(second, (values, quality))| {
                {
                    let mut s = script.lock().unwrap();
                    s.polls.push_back(Ok(values.to_vec()));
                    s.quality = Some(quality.to_vec());
                }
                e.tick(second * 1_000, second * 1_000)
            })
            .collect()
    }

    #[test]
    fn a_suspended_value_enters_the_history_as_absent() {
        use Quality::{Fresh, Suspended};
        let (p, script) = fake("a", inventory("d", &["a"]));
        let mut e = Engine::new(vec![p], 10);
        let outs = tick_with_quality(
            &mut e,
            &script,
            &[
                (&[Some(40.0)], &[Fresh]),
                (&[Some(40.0)], &[Suspended]),
                // Suspended before any reading: absent as well.
                (&[None], &[Suspended]),
            ],
        );
        // The current value is still the last reading, with its quality.
        assert_eq!(outs[1].snapshot.values, vec![Some(40.0)]);
        assert_eq!(outs[1].quality, vec![Suspended]);
        let w = e.history().window(&ids(&["d/load/a"]), 0);
        assert_eq!(w.timestamps_ms, vec![1_000, 2_000, 3_000]);
        assert_eq!(w.series, vec![vec![Some(40.0), None, None]]);
    }

    #[test]
    fn a_held_value_enters_the_history_unchanged() {
        use Quality::{Fresh, Held};
        let (p, script) = fake("a", inventory("d", &["a"]));
        let mut e = Engine::new(vec![p], 10);
        let outs = tick_with_quality(
            &mut e,
            &script,
            &[(&[Some(40.0)], &[Fresh]), (&[Some(40.0)], &[Held])],
        );
        assert_eq!(outs[1].quality, vec![Held]);
        let id = ids(&["d/load/a"]);
        assert_eq!(
            e.history().window(&id, 0).series,
            vec![vec![Some(40.0), Some(40.0)]]
        );
        assert_eq!(e.stats().get(&id)[0].map(|s| s.count), Some(2));
    }

    #[test]
    fn statistics_ignore_suspended_ticks() {
        use Quality::{Fresh, Suspended};
        let (p, script) = fake("a", inventory("d", &["a"]));
        let mut e = Engine::new(vec![p], 10);
        // The suspended ticks carry a value that would move every statistic.
        tick_with_quality(
            &mut e,
            &script,
            &[
                (&[Some(10.0)], &[Fresh]),
                (&[Some(30.0)], &[Fresh]),
                (&[Some(90.0)], &[Suspended]),
                (&[Some(90.0)], &[Suspended]),
                (&[Some(20.0)], &[Fresh]),
            ],
        );
        let stats = e.stats().get(&ids(&["d/load/a"]))[0].expect("three measurements");
        // `max` is also the peak KPI of the device pages.
        assert_eq!(
            (stats.min, stats.max, stats.avg, stats.count),
            (10.0, 30.0, 20.0, 3)
        );
    }

    #[test]
    fn the_history_window_has_a_gap_while_a_sensor_is_suspended() {
        use Quality::{Fresh, Suspended};
        let (p, script) = fake("a", inventory("d", &["a", "b"]));
        let mut e = Engine::new(vec![p], 10);
        tick_with_quality(
            &mut e,
            &script,
            &[
                (&[Some(1.0), Some(35.0)], &[Fresh, Fresh]),
                (&[Some(2.0), Some(35.0)], &[Fresh, Suspended]),
                (&[Some(3.0), Some(35.0)], &[Fresh, Suspended]),
                (&[Some(4.0), Some(36.0)], &[Fresh, Fresh]),
            ],
        );
        let w = e.history().window(&ids(&["d/load/a", "d/load/b"]), 0);
        // Every tick keeps its row: only the suspended sensor has the gap.
        assert_eq!(w.timestamps_ms, vec![1_000, 2_000, 3_000, 4_000]);
        assert_eq!(
            w.series,
            vec![
                vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)],
                vec![Some(35.0), None, None, Some(36.0)],
            ]
        );
    }

    #[test]
    fn a_quality_vector_of_the_wrong_length_falls_back_to_repeated() {
        let (p, script) = fake("a", inventory("d", &["a", "b"]));
        {
            let mut s = script.lock().unwrap();
            s.polls.push_back(Ok(vec![Some(1.0), Some(2.0)]));
            s.quality = Some(vec![Quality::Held]);
        }
        let mut e = Engine::new(vec![p], 10);
        assert_eq!(e.tick(0, 0).quality, vec![Quality::Fresh, Quality::Fresh]);
        // With `repeated` the fallback is Held for every value.
        script.lock().unwrap().repeated = true;
        assert_eq!(
            e.tick(1_000, 1_000).quality,
            vec![Quality::Held, Quality::Held]
        );
    }

    /// Two sensors; the first poll answers `[1, 2]` with `[Fresh, Suspended]`,
    /// later polls block until released.
    struct SlowQuality {
        wait: std::sync::mpsc::Receiver<()>,
        calls: usize,
    }
    impl Provider for SlowQuality {
        fn name(&self) -> &'static str {
            "slow-quality"
        }
        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            Ok(inventory("dev/slow", &["x", "y"]))
        }
        fn poll(&mut self) -> PollResult {
            self.calls += 1;
            if self.calls > 1 {
                let _ = self.wait.recv();
            }
            Ok(vec![Some(1.0), Some(2.0)])
        }
        fn quality(&self) -> Option<Vec<Quality>> {
            Some(vec![Quality::Fresh, Quality::Suspended])
        }
    }

    #[test]
    fn a_timed_out_slot_holds_its_values_and_keeps_suspended() {
        let (release, wait) = std::sync::mpsc::channel();
        let mut e = Engine::new(vec![Box::new(SlowQuality { wait, calls: 0 })], 10);
        let out = e.tick(0, 0);
        assert_eq!(out.quality, vec![Quality::Fresh, Quality::Suspended]);
        let out = e.tick(1_000, 1_000);
        assert_eq!(out.snapshot.values, vec![Some(1.0), Some(2.0)]);
        assert_eq!(out.quality, vec![Quality::Held, Quality::Suspended]);
        drop(e);
        drop(release);
    }

    #[test]
    fn a_second_timeout_clears_values_and_suspended_quality() {
        let (release, wait) = std::sync::mpsc::channel();
        let mut e = Engine::new(vec![Box::new(SlowQuality { wait, calls: 0 })], 10);
        e.tick(0, 0);
        e.tick(1_000, 1_000);
        let out = e.tick(2_000, 2_000);
        assert_eq!(out.snapshot.values, vec![None, None]);
        assert_eq!(out.quality, vec![Quality::Fresh, Quality::Fresh]);
        drop(e);
        drop(release);
    }

    #[test]
    fn merged_quality_follows_the_winning_sensor_indices() {
        // "a" and "b" both expose dev/a/load/x: "b" loses it, so only its
        // second sensor (with its own quality) reaches the output.
        let (a, script_a) = fake("a", inventory("dev/a", &["x"]));
        let (b, script_b) = fake("b", inventory("dev/a", &["x", "y"]));
        {
            let mut s = script_a.lock().unwrap();
            s.polls.push_back(Ok(vec![Some(1.0)]));
            s.quality = Some(vec![Quality::Fresh]);
        }
        {
            let mut s = script_b.lock().unwrap();
            s.polls.push_back(Ok(vec![Some(9.0), Some(2.0)]));
            s.quality = Some(vec![Quality::Suspended, Quality::Held]);
        }
        let mut e = Engine::new(vec![a, b], 10);
        let out = e.tick(0, 0);
        assert_eq!(out.snapshot.values, vec![Some(1.0), Some(2.0)]);
        assert_eq!(out.quality, vec![Quality::Fresh, Quality::Held]);
    }

    #[test]
    fn repeated_provider_marks_its_values_held() {
        let (a, script_a) = fake("a", inventory("dev/a", &["x", "y"]));
        let (b, _) = fake("b", inventory("dev/b", &["z"]));
        script_a
            .lock()
            .unwrap()
            .polls
            .push_back(Ok(vec![Some(3.0), None]));
        script_a.lock().unwrap().repeated = true;
        let mut e = Engine::new(vec![a, b], 10);
        let out = e.tick(0, 0);
        assert_eq!(out.snapshot.values, vec![Some(3.0), None, Some(1.0)]);
        assert_eq!(
            out.quality,
            vec![Quality::Held, Quality::Fresh, Quality::Fresh]
        );
        script_a.lock().unwrap().repeated = false;
        let out = e.tick(1_000, 1_000);
        assert_eq!(out.quality, vec![Quality::Fresh; 3]);
    }

    #[test]
    fn implausible_value_is_fresh_even_from_a_repeated_provider() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        {
            let mut s = script.lock().unwrap();
            s.polls.push_back(Ok(vec![Some(150.0)]));
            s.repeated = true;
        }
        let mut e = Engine::new(vec![p], 10);
        let out = e.tick(0, 0);
        assert_eq!(out.snapshot.values, vec![None]);
        assert_eq!(out.quality, vec![Quality::Fresh]);
    }

    #[test]
    fn quality_is_aligned_after_a_schema_change() {
        // "a" (repeated) and "b" both expose dev/a/load/x: "a" wins it, so
        // "b" contributes only its second sensor, with its own quality.
        let (a, script_a) = fake("a", inventory("dev/a", &["x"]));
        let b_inv = Inventory {
            devices: inventory("dev/a", &["x"]).devices,
            sensors: ["x", "y"]
                .iter()
                .map(|n| {
                    Sensor::new(
                        "dev/a",
                        SensorKind::Load,
                        n,
                        Unit::Percent,
                        Label::new("test"),
                        Source::Lhm,
                    )
                })
                .collect(),
        };
        let (b, script_b) = fake("b", b_inv);
        script_a.lock().unwrap().repeated = true;
        script_b
            .lock()
            .unwrap()
            .polls
            .push_back(Ok(vec![Some(9.0), Some(2.0)]));
        let mut e = Engine::new(vec![a, b], 10);
        let out = e.tick(1_000, 1_000);
        assert!(out.schema.is_some());
        assert_eq!(out.snapshot.values, vec![Some(1.0), Some(2.0)]);
        assert_eq!(out.quality, vec![Quality::Held, Quality::Fresh]);
        // "a" disappears from the schema: "b" now owns both sensors and the
        // quality vector follows the new layout.
        {
            let mut s = script_a.lock().unwrap();
            s.inventory = Inventory::default();
            s.polls.push_back(Err(ProviderError::Rediscover));
        }
        e.tick(2_000, 2_000);
        let out = e.tick(3_000, 3_000);
        assert!(out.schema.is_some());
        assert_eq!(out.snapshot.values.len(), 2);
        assert_eq!(out.quality, vec![Quality::Fresh, Quality::Fresh]);
    }

    #[test]
    fn devices_with_the_same_id_are_merged() {
        let mut props_a = std::collections::BTreeMap::new();
        props_a.insert("k".to_string(), "1".to_string());
        let inv_a = Inventory {
            devices: vec![Device {
                id: "dev/a".into(),
                kind: DeviceKind::Cpu,
                name: "Core".into(),
                vendor: None,
                properties: props_a,
            }],
            sensors: vec![Sensor::new(
                "dev/a",
                SensorKind::Load,
                "x",
                Unit::Percent,
                Label::new("test"),
                Source::Mock,
            )],
        };
        let mut props_b = std::collections::BTreeMap::new();
        props_b.insert("k".to_string(), "2".to_string());
        props_b.insert("j".to_string(), "3".to_string());
        let inv_b = Inventory {
            devices: vec![Device {
                id: "dev/a".into(),
                kind: DeviceKind::Cpu,
                name: "LHM".into(),
                vendor: None,
                properties: props_b,
            }],
            sensors: vec![Sensor::new(
                "dev/a",
                SensorKind::Load,
                "y",
                Unit::Percent,
                Label::new("test"),
                Source::Lhm,
            )],
        };
        let (a, _) = fake("a", inv_a);
        let (b, _) = fake("b", inv_b);
        let mut e = Engine::new(vec![a, b], 10);
        let out = e.tick(1_000, 1_000);
        let schema = out.schema.expect("schema on first tick");
        assert_eq!(schema.devices.len(), 1);
        let device = &schema.devices[0];
        assert_eq!(device.name, "Core");
        assert_eq!(device.properties.get("k").map(String::as_str), Some("1"));
        assert_eq!(device.properties.get("j").map(String::as_str), Some("3"));
    }

    #[test]
    fn values_follow_the_kept_sensors_when_a_provider_polls_short() {
        let (a, script_a) = fake("a", inventory("dev/a", &["x"]));
        let b_inv = Inventory {
            devices: vec![Device {
                id: "dev/b".into(),
                kind: DeviceKind::Cpu,
                name: "dev/b".into(),
                vendor: None,
                properties: Default::default(),
            }],
            sensors: vec![
                Sensor::new(
                    "dev/a",
                    SensorKind::Load,
                    "x",
                    Unit::Percent,
                    Label::new("test"),
                    Source::Lhm,
                ),
                Sensor::new(
                    "dev/b",
                    SensorKind::Load,
                    "p",
                    Unit::Percent,
                    Label::new("test"),
                    Source::Lhm,
                ),
                Sensor::new(
                    "dev/b",
                    SensorKind::Load,
                    "q",
                    Unit::Percent,
                    Label::new("test"),
                    Source::Lhm,
                ),
            ],
        };
        let (b, script_b) = fake("b", b_inv);
        script_a
            .lock()
            .unwrap()
            .polls
            .push_back(Ok(vec![Some(1.0)]));
        // Only 1 value for 3 sensors: the worker turns this into a poll failure.
        script_b
            .lock()
            .unwrap()
            .polls
            .push_back(Ok(vec![Some(5.0)]));
        let mut e = Engine::new(vec![a, b], 10);
        let out = e.tick(1_000, 1_000);
        let schema = out.schema.expect("schema on first tick");
        assert_eq!(schema.sensors.len(), 3);
        assert_eq!(out.snapshot.values, vec![Some(1.0), None, None]);
    }

    #[test]
    fn svc_sensors_vanish_without_touching_core_history() {
        let (a, _) = fake("a", inventory("dev/a", &["x"]));
        let (b, script_b) = fake("b", inventory("dev/b", &["y"]));
        let mut e = Engine::new(vec![a, b], 10);
        e.tick(1_000, 1_000);
        e.tick(2_000, 2_000);
        script_b
            .lock()
            .unwrap()
            .polls
            .push_back(Err(ProviderError::Rediscover));
        e.tick(3_000, 3_000);
        script_b.lock().unwrap().inventory = Inventory::default();
        let out = e.tick(4_000, 4_000);
        let schema = out.schema.expect("schema changes when b vanishes");
        assert!(schema.sensors.iter().all(|s| s.id != "dev/b/load/y"));
        let w = e.history().window(&["dev/a/load/x".into()], 0);
        assert_eq!(w.timestamps_ms, vec![1_000, 2_000, 3_000, 4_000]);
        assert_eq!(
            w.series[0],
            vec![Some(1.0), Some(1.0), Some(1.0), Some(1.0)]
        );
    }

    #[test]
    fn winner_change_resets_history_and_stats() {
        let (a, script_a) = fake("a", Inventory::default());
        let b_inv = Inventory {
            devices: vec![Device {
                id: "dev/a".into(),
                kind: DeviceKind::Cpu,
                name: "B".into(),
                vendor: None,
                properties: Default::default(),
            }],
            sensors: vec![Sensor::new(
                "dev/a",
                SensorKind::Load,
                "x",
                Unit::Percent,
                Label::new("test"),
                Source::Lhm,
            )],
        };
        let (b, script_b) = fake("b", b_inv);
        script_b
            .lock()
            .unwrap()
            .polls
            .extend([Ok(vec![Some(1.0)]), Ok(vec![Some(2.0)])]);
        let mut e = Engine::new(vec![a, b], 10);
        e.tick(1_000, 1_000);
        e.tick(2_000, 2_000);
        {
            let mut s = script_a.lock().unwrap();
            s.inventory = inventory("dev/a", &["x"]);
            s.polls.push_back(Err(ProviderError::Rediscover));
        }
        e.tick(3_000, 3_000);
        script_a
            .lock()
            .unwrap()
            .polls
            .push_back(Ok(vec![Some(9.0)]));
        let out = e.tick(4_000, 4_000);
        let schema = out.schema.expect("schema changes when a claims the id");
        let winner = schema
            .sensors
            .iter()
            .find(|s| s.id == "dev/a/load/x")
            .expect("winner present");
        assert_eq!(winner.source, Source::Mock);
        let w = e.history().window(&["dev/a/load/x".into()], 0);
        assert_eq!(w.timestamps_ms, vec![1_000, 2_000, 3_000, 4_000]);
        assert_eq!(w.series[0], vec![None, None, None, Some(9.0)]);
        let stats = e.stats().get(&["dev/a/load/x".to_string()]);
        assert_eq!(stats[0].map(|s| s.count), Some(1));
    }

    #[test]
    fn backoff_doubles_up_to_one_minute() {
        assert_eq!(
            [1, 2, 3, 4, 5, 10].map(backoff_ms),
            [5_000, 10_000, 20_000, 40_000, 60_000, 60_000]
        );
    }
}
