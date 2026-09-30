//! Keeps the rule engine's rules in step with `rules` in the settings store,
//! and paces the health clock sent to an open window.

use std::sync::{Arc, Mutex, PoisonError};

use oma_core::engine::Engine;
use oma_core::rules::{effective_rules, HealthClock, Rule, RulesSettings};

use crate::settings::SettingsStore;

/// The health report, sent when it changes (spec §3.5).
pub const EVENT_HEALTH: &str = "oma:health";
/// How long the overall level has lasted; see [`ClockPacer`].
pub const EVENT_HEALTH_CLOCK: &str = "oma:health-clock";

/// Gives the engine the effective rules of the stored settings now, before
/// the sampler starts, and again on every change of `rules`.
///
/// Like `interval::follow_interval`, the listener runs on whichever thread
/// changes the store and applies changes in store order; it may wait for the
/// engine lock, which a tick holds for up to about 200 ms. The store lock is
/// never held while the engine is called.
pub fn install_rules(store: &Arc<SettingsStore>, engine: Arc<Mutex<Engine>>) {
    follow_rules(store, move |rules| {
        engine
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_rules(rules);
    });
}

/// Calls `apply_rules` with the effective rules (disabled ones included,
/// since the rules settings list them) once right after subscribing, and
/// then only when the effective rules change: an override equal to the
/// built-in value changes `rules` but not what the engine evaluates. The
/// rules seen last also serialize the listener with that catch-up.
fn follow_rules(
    store: &Arc<SettingsStore>,
    apply_rules: impl Fn(Vec<Rule>) + Send + Sync + 'static,
) {
    // The rules settings seen last (every settings change reaches the
    // listener, so most return here) and the effective rules applied.
    let applied: Mutex<Option<(RulesSettings, Vec<Rule>)>> = Mutex::new(None);
    let apply = move |rules: &RulesSettings| {
        let mut applied = applied.lock().unwrap_or_else(PoisonError::into_inner);
        if applied.as_ref().is_some_and(|(seen, _)| seen == rules) {
            return;
        }
        let effective = effective_rules(rules);
        if applied.as_ref().is_some_and(|(_, last)| *last == effective) {
            *applied = Some((rules.clone(), effective));
            return;
        }
        apply_rules(effective.clone());
        *applied = Some((rules.clone(), effective));
    };
    let apply = Arc::new(apply);
    let listener = Arc::clone(&apply);
    store.subscribe(Box::new(move |settings, _| listener(&settings.rules)));
    // `snapshot` releases the store lock before the engine is called.
    apply(&store.snapshot().rules);
}

/// Decides which health clocks go to the window: every new revision at once
/// (the UI accepts only the current one), otherwise at most one a second of
/// the level's monotonic duration.
#[derive(Debug, Default)]
pub struct ClockPacer {
    last: Option<HealthClock>,
}

impl ClockPacer {
    const PERIOD_MS: u64 = 1_000;

    /// Whether `clock` should be sent; if so it becomes the last one sent.
    pub fn take(&mut self, clock: HealthClock) -> bool {
        let due = self.last.is_none_or(|last| {
            last.revision != clock.revision
                || clock.level_elapsed_ms < last.level_elapsed_ms
                || clock.level_elapsed_ms - last.level_elapsed_ms >= Self::PERIOD_MS
        });
        if due {
            self.last = Some(clock);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use oma_core::engine::Engine;
    use oma_core::rules::{default_rules, effective_rules, HealthClock, Rule};
    use serde_json::json;

    use super::*;
    use crate::settings::fake_fs::{open_fast, test_path, wait_until, FakeFs};

    type Calls = Arc<Mutex<Vec<Vec<Rule>>>>;

    fn follow_into(store: &Arc<SettingsStore>) -> Calls {
        let calls: Calls = Arc::default();
        let sink = Arc::clone(&calls);
        follow_rules(store, move |rules| sink.lock().unwrap().push(rules));
        calls
    }

    fn enabled(rules: &[Rule], id: &str) -> Option<bool> {
        rules
            .iter()
            .find(|rule| rule.id == id)
            .map(|rule| rule.enabled)
    }

    fn ids(rules: &[Rule]) -> Vec<String> {
        rules.iter().map(|rule| rule.id.clone()).collect()
    }

    #[test]
    fn rules_follow_settings_changes() {
        let store = Arc::new(open_fast(&FakeFs::new()));
        let calls = follow_into(&store);
        assert_eq!(calls.lock().unwrap().len(), 1, "the initial catch-up");
        assert_eq!(enabled(&calls.lock().unwrap()[0], "gpu-temp"), Some(true));

        store
            .update(&json!({"rules": {"overrides": {"gpu-temp": {"enabled": false}}}}))
            .unwrap();
        wait_until("the disabled rule", || calls.lock().unwrap().len() == 2);
        let last = calls.lock().unwrap()[1].clone();
        // Disabled rules stay in the list (the rules settings show them), so
        // the engine expands no instance of it.
        assert_eq!(enabled(&last, "gpu-temp"), Some(false));
        assert_eq!(enabled(&last, "cpu-temp"), Some(true));
        assert_eq!(ids(&last), ids(&default_rules()));

        // An override equal to the built-in value changes the stored rules
        // settings but not the effective rules: nothing to apply.
        store
            .update(&json!({"rules": {"overrides": {"cpu-temp": {"enabled": true}}}}))
            .unwrap();
        assert_eq!(calls.lock().unwrap().len(), 2);

        // Other settings do not touch the rules.
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        store
            .update(&json!({"general": {"temperatureUnit": "f"}}))
            .unwrap();
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn initial_rules_are_installed_before_sampling() {
        let fs = FakeFs::new().with_file(
            &test_path(),
            br#"{"version":1,"rules":{"overrides":{"gpu-temp":{"enabled":false}}}}"#,
        );
        let store = Arc::new(open_fast(&fs));
        let engine = Arc::new(Mutex::new(Engine::new(Vec::new(), 3_600)));
        install_rules(&store, Arc::clone(&engine));
        // No tick yet and no waiting: the rules are already in the engine.
        let status = engine.lock().unwrap().rule_status();
        let installed: Vec<String> = status.iter().map(|s| s.rule_id.clone()).collect();
        assert_eq!(installed, ids(&default_rules()));
    }

    #[test]
    fn rapid_settings_updates_keep_latest_rules() {
        let store = Arc::new(open_fast(&FakeFs::new()));
        let calls = follow_into(&store);
        let writers: Vec<_> = (0..4)
            .map(|writer| {
                let store = Arc::clone(&store);
                std::thread::spawn(move || {
                    for step in 0..25 {
                        let off = (writer + step) % 2 == 0;
                        let _ = store.update(
                            &json!({"rules": {"overrides": {"gpu-temp": {"enabled": !off}}}}),
                        );
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let expected = effective_rules(&store.settings().rules);
        wait_until("the latest rules", || {
            calls.lock().unwrap().last() == Some(&expected)
        });
        // Only effective changes reach the engine: never the same list twice in a row.
        let calls = calls.lock().unwrap();
        assert!(calls.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn health_clock_uses_monotonic_duration() {
        let mut engine = Engine::new(Vec::new(), 3_600);
        let wall = 1_700_000_000_000;
        engine.tick(wall, 10_000);
        // The system clock goes back an hour; the monotonic one moves on 2.5 s.
        engine.tick(wall - 3_600_000, 12_500);
        let clock = engine.health_clock();
        assert_eq!(clock.level_elapsed_ms, 2_500);
        assert_eq!(clock.revision, engine.health().revision);
    }

    #[test]
    fn clock_pacer_sends_at_most_once_a_second_and_every_revision() {
        let clock = |revision, level_elapsed_ms| HealthClock {
            revision,
            level_elapsed_ms,
        };
        let mut pacer = ClockPacer::default();
        let sent: Vec<bool> = [
            clock(1, 0),
            clock(1, 500),
            clock(1, 999),
            clock(1, 1_000),
            clock(1, 1_500),
            clock(1, 2_100),
            // A new revision goes out at once, even within the second.
            clock(2, 2_200),
            clock(2, 2_300),
            // A new level starts again from zero.
            clock(3, 0),
            clock(3, 400),
        ]
        .into_iter()
        .map(|c| pacer.take(c))
        .collect();
        assert_eq!(
            sent,
            [true, false, false, true, false, true, true, false, true, false]
        );
    }
}
