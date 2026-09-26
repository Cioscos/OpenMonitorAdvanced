//! Drops values outside the physically plausible range (spec §8).

use std::collections::{HashMap, HashSet};

use crate::model::Unit;

/// Minimum time between two "discarding implausible value" lines for the
/// same sensor.
pub const DISCARD_LOG_INTERVAL_MS: u64 = 60_000;

/// Rate limit for the debug line written when a value is discarded. The
/// release log filter keeps `oma_core=debug`, so without it a sensor stuck on
/// an implausible reading would write one line per tick for as long as the
/// app runs.
#[derive(Debug, Default)]
pub struct DiscardLog {
    last_logged_ms: HashMap<String, u64>,
}

impl DiscardLog {
    /// True when a discard of `sensor_id` may be logged at `now_ms`
    /// (monotonic): the first time, then at most once per
    /// `DISCARD_LOG_INTERVAL_MS`.
    pub fn should_log(&mut self, sensor_id: &str, now_ms: u64) -> bool {
        match self.last_logged_ms.get_mut(sensor_id) {
            Some(last) if now_ms.saturating_sub(*last) < DISCARD_LOG_INTERVAL_MS => false,
            Some(last) => {
                *last = now_ms;
                true
            }
            None => {
                self.last_logged_ms.insert(sensor_id.to_owned(), now_ms);
                true
            }
        }
    }

    /// Forgets sensors that are no longer in the schema.
    pub fn retain(&mut self, ids: &[String]) {
        let keep: HashSet<&str> = ids.iter().map(String::as_str).collect();
        self.last_logged_ms
            .retain(|id, _| keep.contains(id.as_str()));
    }
}

/// Plausible value or `None`. It does not log: the engine logs discards per
/// sensor through `DiscardLog`.
pub fn sanitize(unit: Unit, value: Option<f64>) -> Option<f64> {
    let v = value?;
    if !v.is_finite() {
        return None;
    }
    let plausible = match unit {
        Unit::Celsius => (-50.0..=150.0).contains(&v),
        Unit::Percent => (0.0..=100.0).contains(&v),
        Unit::Megahertz => (0.0..=20_000.0).contains(&v),
        Unit::Volt => (-20.0..=20.0).contains(&v),
        Unit::Boolean => v == 0.0 || v == 1.0,
        Unit::PcieGeneration => (1.0..=7.0).contains(&v) && v.fract() == 0.0,
        Unit::Lanes => (1.0..=32.0).contains(&v) && v.fract() == 0.0,
        Unit::Hours => (0.0..=10_000_000.0).contains(&v) && v.fract() == 0.0,
        Unit::Count => (0.0..=1_000_000_000.0).contains(&v) && v.fract() == 0.0,
        _ => v >= 0.0,
    };
    plausible.then_some(v)
}

pub fn sanitize_sensor(sensor: &crate::model::Sensor, value: Option<f64>) -> Option<f64> {
    if sensor.unit == Unit::Percent
        && sensor.kind == crate::model::SensorKind::Percent
        && sensor.device_id.starts_with("gpu/")
        && sensor.id.ends_with("/percent/power-limit")
    {
        return value.filter(|v| v.is_finite() && *v >= 0.0);
    }
    // NVMe "Percentage Used" (spec §M4) can legitimately exceed 100.
    if sensor.unit == Unit::Percent
        && sensor.device_id.starts_with("storage/")
        && sensor.id.ends_with("/percent/wear")
    {
        return value.filter(|v| v.is_finite() && (0.0..=255.0).contains(v));
    }
    sanitize(sensor.unit, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_stays_none() {
        assert_eq!(sanitize(Unit::Celsius, None), None);
    }

    #[test]
    fn rejects_non_finite() {
        assert_eq!(sanitize(Unit::Watt, Some(f64::NAN)), None);
        assert_eq!(sanitize(Unit::Watt, Some(f64::INFINITY)), None);
    }

    #[test]
    fn temperature_must_be_physically_plausible() {
        assert_eq!(sanitize(Unit::Celsius, Some(-60.0)), None);
        assert_eq!(sanitize(Unit::Celsius, Some(45.0)), Some(45.0));
        assert_eq!(sanitize(Unit::Celsius, Some(151.0)), None);
    }

    #[test]
    fn percent_must_stay_within_0_and_100() {
        assert_eq!(sanitize(Unit::Percent, Some(100.0)), Some(100.0));
        assert_eq!(sanitize(Unit::Percent, Some(100.5)), None);
        assert_eq!(sanitize(Unit::Percent, Some(-0.1)), None);
    }

    #[test]
    fn negative_voltage_rails_are_allowed() {
        assert_eq!(sanitize(Unit::Volt, Some(-12.1)), Some(-12.1));
        assert_eq!(sanitize(Unit::Volt, Some(25.0)), None);
    }

    #[test]
    fn counters_and_rates_cannot_be_negative() {
        assert_eq!(sanitize(Unit::Bytes, Some(-1.0)), None);
        assert_eq!(sanitize(Unit::BytesPerSecond, Some(0.0)), Some(0.0));
    }

    #[test]
    fn discard_log_allows_one_line_per_sensor_per_minute() {
        let mut log = DiscardLog::default();
        assert!(log.should_log("a", 1_000));
        assert!(!log.should_log("a", 2_000));
        assert!(!log.should_log("a", 60_999));
        // Another sensor has its own budget.
        assert!(log.should_log("b", 2_000));
        assert!(log.should_log("a", 61_000));
        assert!(!log.should_log("a", 120_999));
        assert!(log.should_log("a", 121_000));
    }

    #[test]
    fn discard_log_forgets_removed_sensors() {
        let mut log = DiscardLog::default();
        assert!(log.should_log("a", 0));
        assert!(log.should_log("b", 0));
        log.retain(&["b".to_owned()]);
        assert!(log.should_log("a", 1_000));
        assert!(!log.should_log("b", 1_000));
    }

    #[test]
    fn pcie_generation_is_a_whole_number_from_1_to_7() {
        assert_eq!(sanitize(Unit::PcieGeneration, Some(1.0)), Some(1.0));
        assert_eq!(sanitize(Unit::PcieGeneration, Some(4.0)), Some(4.0));
        assert_eq!(sanitize(Unit::PcieGeneration, Some(7.0)), Some(7.0));
        assert_eq!(sanitize(Unit::PcieGeneration, Some(0.0)), None);
        assert_eq!(sanitize(Unit::PcieGeneration, Some(8.0)), None);
        assert_eq!(sanitize(Unit::PcieGeneration, Some(3.5)), None);
    }

    #[test]
    fn lanes_range_from_1_to_32() {
        assert_eq!(sanitize(Unit::Lanes, Some(1.0)), Some(1.0));
        assert_eq!(sanitize(Unit::Lanes, Some(16.0)), Some(16.0));
        assert_eq!(sanitize(Unit::Lanes, Some(32.0)), Some(32.0));
        assert_eq!(sanitize(Unit::Lanes, Some(0.0)), None);
        assert_eq!(sanitize(Unit::Lanes, Some(64.0)), None);
        assert_eq!(
            sanitize(Unit::Lanes, Some(8.5)),
            None,
            "a lane count must be a whole number"
        );
    }

    #[test]
    fn counters_must_be_whole_and_in_range() {
        assert_eq!(sanitize(Unit::Hours, Some(12345.0)), Some(12345.0));
        assert_eq!(sanitize(Unit::Hours, Some(1.5)), None);
        assert_eq!(sanitize(Unit::Hours, Some(-1.0)), None);
        assert_eq!(sanitize(Unit::Count, Some(1e10)), None);
    }

    #[test]
    fn nvme_wear_may_exceed_one_hundred() {
        use crate::model::{Label, Sensor, SensorKind, Source};
        let wear = Sensor::new(
            "storage/x",
            SensorKind::Percent,
            "wear",
            Unit::Percent,
            Label::new("storage.wear"),
            Source::Lhm,
        );
        assert_eq!(sanitize_sensor(&wear, Some(120.0)), Some(120.0));
        let other = Sensor::new(
            "storage/x",
            SensorKind::Percent,
            "active",
            Unit::Percent,
            Label::new("storage.active"),
            Source::Lhm,
        );
        assert_eq!(sanitize_sensor(&other, Some(120.0)), None);
    }

    #[test]
    fn power_ratio_can_exceed_100_but_utilization_cannot() {
        use crate::model::{Label, Sensor, SensorKind, Source};
        let power = Sensor::new(
            "gpu/test",
            SensorKind::Percent,
            "power-limit",
            Unit::Percent,
            Label::new("gpu.power.limitPercent"),
            Source::Nvml,
        );
        assert_eq!(sanitize_sensor(&power, Some(125.0)), Some(125.0));
        assert_eq!(sanitize_sensor(&power, Some(-1.0)), None);
        assert_eq!(sanitize_sensor(&power, Some(f64::INFINITY)), None);
        assert_eq!(sanitize_sensor(&power, Some(f64::NAN)), None);
        for kind in [SensorKind::Load, SensorKind::Fan] {
            let sensor = Sensor::new(
                "gpu/test",
                kind,
                "percent",
                Unit::Percent,
                Label::new("gpu.load.core"),
                Source::Pdh,
            );
            assert_eq!(sanitize_sensor(&sensor, Some(125.0)), None);
            assert_eq!(sanitize_sensor(&sensor, Some(100.0)), Some(100.0));
        }
    }
}
