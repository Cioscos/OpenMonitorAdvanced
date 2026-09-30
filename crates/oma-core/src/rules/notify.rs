//! Toast cooldown (spec §3.5, R3): a pure function of the monotonic clock.

use std::collections::HashMap;

use super::Level;

/// Silence after a toast attempt for the same rule, sensor and level.
pub const TOAST_COOLDOWN_MS: u64 = 300_000;

/// When each rule/sensor/level last attempted a toast, on the monotonic
/// clock. Lives for the whole session, apart from the rule engine, so it
/// survives rule changes and reconnections.
#[derive(Debug, Default)]
pub struct Cooldown {
    last: HashMap<(String, String, Level), u64>,
}

impl Cooldown {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a level entry may raise a toast at `now_ms`, and if so records
    /// the attempt (a failed toast still counts). A refused entry is dropped,
    /// not deferred: only a later entry can be admitted.
    pub fn admit(&mut self, rule_id: &str, sensor_id: &str, level: Level, now_ms: u64) -> bool {
        // Toasts are rare: allocating the key on each attempt is fine.
        let key = (rule_id.to_owned(), sensor_id.to_owned(), level);
        if let Some(&last) = self.last.get(&key) {
            if now_ms.saturating_sub(last) < TOAST_COOLDOWN_MS {
                return false;
            }
        }
        self.last.insert(key, now_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Quality;
    use crate::model::{Device, DeviceKind, Label, Schema, Sensor, SensorKind, Source, Unit};
    use crate::rules::{default_rules, LevelSpec, RuleEngine, Threshold};

    const GPU0: &str = "gpu/0/temperature/core";
    const MIN: u64 = 60_000;

    #[test]
    fn cooldown_suppresses_reentry_within_five_minutes() {
        let mut cooldown = Cooldown::new();
        assert!(cooldown.admit("gpu-temp", GPU0, Level::Crit, 0));
        assert!(!cooldown.admit("gpu-temp", GPU0, Level::Crit, 1));
        assert!(!cooldown.admit("gpu-temp", GPU0, Level::Crit, TOAST_COOLDOWN_MS - 1));
    }

    #[test]
    fn cooldown_allows_after_five_minutes() {
        let mut cooldown = Cooldown::new();
        assert!(cooldown.admit("gpu-temp", GPU0, Level::Crit, 10));
        assert!(cooldown.admit("gpu-temp", GPU0, Level::Crit, 10 + TOAST_COOLDOWN_MS));
        // The admitted attempt restarts the cooldown.
        assert!(!cooldown.admit("gpu-temp", GPU0, Level::Crit, 11 + TOAST_COOLDOWN_MS));
    }

    #[test]
    fn cooldown_is_per_level() {
        let mut cooldown = Cooldown::new();
        assert!(cooldown.admit("gpu-temp", GPU0, Level::Crit, 0));
        assert!(cooldown.admit("gpu-temp", GPU0, Level::Warn, 1));
        assert!(cooldown.admit("gpu-temp", "gpu/1/temperature/core", Level::Crit, 2));
        assert!(cooldown.admit("gpu-hotspot", GPU0, Level::Crit, 3));
        assert!(!cooldown.admit("gpu-temp", GPU0, Level::Crit, 4));
        assert!(!cooldown.admit("gpu-temp", GPU0, Level::Warn, 5));
    }

    #[test]
    fn no_deferred_toast_at_expiry() {
        // A refused entry is not queued for later.
        let mut cooldown = Cooldown::new();
        assert!(cooldown.admit("gpu-temp", GPU0, Level::Crit, 0));
        assert!(!cooldown.admit("gpu-temp", GPU0, Level::Crit, 4 * MIN));

        // The notifier loop over the engine: crit at 0 (toast), recovery,
        // crit again at 4 min (refused), still crit past 5 min (no entry,
        // so no toast), recovery and a new entry at 5 min 20 s (toast).
        let schema = Schema {
            revision: 1,
            devices: vec![Device {
                id: "gpu/0".into(),
                kind: DeviceKind::Gpu,
                name: "GPU 0".into(),
                vendor: None,
                properties: Default::default(),
            }],
            sensors: vec![Sensor::new(
                "gpu/0",
                SensorKind::Temperature,
                "core",
                Unit::Celsius,
                Label::new("gpu.core"),
                Source::Mock,
            )],
        };
        let mut rule = default_rules()
            .into_iter()
            .find(|r| r.id == "gpu-temp")
            .unwrap();
        let instant = |value: f64| LevelSpec {
            threshold: Some(Threshold::Fixed { fixed: value }),
            duration_s: 0,
        };
        rule.warn = Some(instant(83.0));
        rule.crit = Some(instant(90.0));
        rule.hysteresis.duration_s = 0;
        let mut engine = RuleEngine::new();
        engine.set_interval_ms(1000);
        engine.set_rules(vec![rule]);

        let mut cooldown = Cooldown::new();
        let mut entries = Vec::new();
        let mut toasts = Vec::new();
        for s in 0..=400u64 {
            let value = match s {
                0..=9 => 95.0,
                10..=239 => 50.0,
                240..=309 => 95.0,
                310..=319 => 50.0,
                _ => 95.0,
            };
            let ms = s * 1000;
            let out = engine.evaluate(&schema, s == 0, &[Some(value)], &[Quality::Fresh], ms, ms);
            for entry in out.entries {
                entries.push(ms);
                if entry.notify && cooldown_admit(&mut cooldown, &entry, ms) {
                    toasts.push(ms);
                }
            }
        }
        assert_eq!(entries, [0, 240_000, 320_000]);
        assert_eq!(toasts, [0, 320_000]);
    }

    fn cooldown_admit(cooldown: &mut Cooldown, entry: &crate::rules::LevelEntry, ms: u64) -> bool {
        cooldown.admit(&entry.rule_id, &entry.sensor_id, entry.level, ms)
    }
}
