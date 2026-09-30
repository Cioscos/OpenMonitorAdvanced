//! Keeps the sampling interval in step with `general.intervalMs` in the
//! settings store.

use std::sync::{Arc, Mutex, PoisonError};

use oma_core::engine::Engine;
use oma_core::sampler::{history_capacity, sample_interval, IntervalHandle};

use crate::settings::SettingsStore;

/// What tells the service link about a new interval (milliseconds).
pub type LinkSink = Box<dyn Fn(u32) + Send + Sync>;

/// Applies every change of `general.intervalMs` to the running app, in this
/// order: the history is resized to one hour of samples at the new pace and
/// the rules are given the interval for their suspend detection, the sampler
/// is given the new interval (and woken), and the link is told. The value the
/// sampler was started with is taken as the current one and given to the
/// rules at once, and the store is checked once right after subscribing, so
/// a change made before the listener existed is not lost.
///
/// The listener runs on whichever thread changes the store, the settings
/// writer included, so it must be quick and must never wait for the store:
/// it locks the engine briefly (a tick holds it for at most about 200 ms),
/// stores an atomic and pushes on the link's channel. Only a change of the
/// interval does anything; every other setting returns at once.
pub fn follow_interval(
    store: &Arc<SettingsStore>,
    engine: Arc<Mutex<Engine>>,
    interval: IntervalHandle,
    link: LinkSink,
) {
    // The rules start at 1 s; the sampler may already run at another pace.
    engine
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .set_interval_ms(interval.get().as_millis() as u64);
    // The interval applied last; also serializes the listener with the
    // catch-up below, so changes are applied in store order.
    let applied = Mutex::new(interval.get().as_millis() as u32);
    let apply = move |ms: u32| {
        let mut applied = applied.lock().unwrap_or_else(PoisonError::into_inner);
        if *applied == ms {
            return;
        }
        let Ok(new) = sample_interval(u64::from(ms)) else {
            tracing::warn!(ms, "ignoring a sampling interval outside 500..=5000 ms");
            return;
        };
        *applied = ms;
        {
            let mut engine = engine.lock().unwrap_or_else(PoisonError::into_inner);
            engine.set_history_capacity(history_capacity(new));
            engine.set_interval_ms(u64::from(ms));
        }
        interval.set(new);
        link(ms);
    };
    let apply = Arc::new(apply);
    let listener = Arc::clone(&apply);
    store.subscribe(Box::new(move |settings, _| {
        listener(settings.general.interval_ms)
    }));
    apply(store.settings().general.interval_ms);
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use oma_core::engine::Engine;
    use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
    use oma_core::provider::{Inventory, Provider, ProviderError};
    use oma_core::rules::{default_rules, OverallLevel};
    use oma_core::sampler::IntervalHandle;

    use super::*;
    use crate::settings::fake_fs::{open_fast, test_path, wait_until, FakeFs};
    use crate::settings::SettingsStore;

    struct Rig {
        store: Arc<SettingsStore>,
        engine: Arc<Mutex<Engine>>,
        interval: IntervalHandle,
        link: Arc<Mutex<Vec<u32>>>,
    }

    fn rig_with(fs: &Arc<FakeFs>, initial: Duration) -> Rig {
        let store = Arc::new(open_fast(fs));
        let engine = Arc::new(Mutex::new(Engine::new(Vec::new(), 3_600)));
        let interval = IntervalHandle::new(initial);
        let link = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&link);
        follow_interval(
            &store,
            Arc::clone(&engine),
            interval.clone(),
            Box::new(move |ms| sink.lock().unwrap().push(ms)),
        );
        Rig {
            store,
            engine,
            interval,
            link,
        }
    }

    fn capacity(rig: &Rig) -> usize {
        rig.engine.lock().unwrap().history().capacity()
    }

    fn sent(rig: &Rig) -> Vec<u32> {
        rig.link.lock().unwrap().clone()
    }

    #[test]
    fn a_change_of_the_interval_reaches_history_sampler_and_link() {
        let rig = rig_with(&FakeFs::new(), Duration::from_millis(1_000));
        rig.store
            .update(&serde_json::json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        wait_until("the new interval", || sent(&rig) == [2000]);
        assert_eq!(rig.interval.get(), Duration::from_millis(2_000));
        assert_eq!(capacity(&rig), 1_800);
    }

    #[test]
    fn other_settings_and_repeated_values_do_nothing() {
        let rig = rig_with(&FakeFs::new(), Duration::from_millis(1_000));
        rig.store
            .update(&serde_json::json!({"sources": {"antiCheat": true}}))
            .unwrap();
        rig.store
            .update(&serde_json::json!({"general": {"intervalMs": 1000}}))
            .unwrap();
        rig.store
            .update(&serde_json::json!({"general": {"intervalMs": 500}}))
            .unwrap();
        rig.store
            .update(&serde_json::json!({"general": {"intervalMs": 500}}))
            .unwrap();
        wait_until("the last interval", || sent(&rig) == [500]);
        assert_eq!(capacity(&rig), 7_200);
    }

    #[test]
    fn a_value_stored_before_the_listener_is_caught_up() {
        let fs = FakeFs::new().with_file(
            &test_path(),
            br#"{"version":1,"general":{"intervalMs":2000}}"#,
        );
        // The sampler was started with the default, the store already says 2000.
        let rig = rig_with(&fs, Duration::from_millis(1_000));
        assert_eq!(rig.interval.get(), Duration::from_millis(2_000));
        assert_eq!(capacity(&rig), 1_800);
        assert_eq!(sent(&rig), vec![2000]);
        assert_eq!(rig.store.settings().general.interval_ms, 2_000);
    }

    /// Memory used at 99 %, the `ram-used` rule's critical level.
    struct FullMemory;

    impl Provider for FullMemory {
        fn name(&self) -> &'static str {
            "full-memory"
        }

        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            Ok(Inventory {
                devices: vec![Device {
                    id: "memory/0".into(),
                    kind: DeviceKind::Memory,
                    name: "RAM".into(),
                    vendor: None,
                    properties: Default::default(),
                }],
                sensors: vec![Sensor::new(
                    "memory/0",
                    SensorKind::Load,
                    "used",
                    Unit::Percent,
                    Label::new("memory.load"),
                    Source::Mock,
                )],
            })
        }

        fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
            Ok(vec![Some(99.0)])
        }
    }

    /// Ticks 5.5 s apart for 33 s: `ram-used` (critical after 30 s) enters
    /// `crit` only if the rules' suspend bound, max(3 x interval, 5 s), is
    /// above the gap, i.e. only at an interval of 2 s or more.
    fn crit_after_gapped_ticks(engine: &Arc<Mutex<Engine>>) -> bool {
        let mut engine = engine.lock().unwrap();
        let ram = default_rules()
            .into_iter()
            .filter(|rule| rule.id == "ram-used")
            .collect();
        engine.set_rules(ram);
        for step in 0..=6u64 {
            let t = step * 5_500;
            engine.tick(1_700_000_000_000 + t, t);
        }
        engine.health().level == OverallLevel::Crit
    }

    fn rules_rig(fs: &Arc<FakeFs>, initial: Duration) -> (Arc<SettingsStore>, Arc<Mutex<Engine>>) {
        let store = Arc::new(open_fast(fs));
        let engine = Arc::new(Mutex::new(Engine::new(vec![Box::new(FullMemory)], 3_600)));
        follow_interval(
            &store,
            Arc::clone(&engine),
            IntervalHandle::new(initial),
            Box::new(|_| {}),
        );
        (store, engine)
    }

    #[test]
    fn interval_change_reaches_rule_engine() {
        // The default 1 s: every 5.5 s gap looks like a suspend.
        let (_, engine) = rules_rig(&FakeFs::new(), Duration::from_millis(1_000));
        assert!(!crit_after_gapped_ticks(&engine));

        // Started at 2 s from the stored setting: the engine is told at once.
        let fs = FakeFs::new().with_file(
            &test_path(),
            br#"{"version":1,"general":{"intervalMs":2000}}"#,
        );
        let (_, engine) = rules_rig(&fs, Duration::from_millis(2_000));
        assert!(crit_after_gapped_ticks(&engine));

        // Changed to 2 s later on.
        let (store, engine) = rules_rig(&FakeFs::new(), Duration::from_millis(1_000));
        store
            .update(&serde_json::json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        assert!(crit_after_gapped_ticks(&engine));
    }

    #[test]
    fn a_matching_start_sends_nothing() {
        let rig = rig_with(&FakeFs::new(), Duration::from_millis(1_000));
        assert!(sent(&rig).is_empty());
        assert_eq!(rig.interval.get(), Duration::from_millis(1_000));
    }
}
