//! Drops values outside the physically plausible range (spec §8).

use crate::model::Unit;

pub fn sanitize(unit: Unit, value: Option<f64>) -> Option<f64> {
    let v = value?;
    if !v.is_finite() {
        tracing::debug!(?unit, v, "discarding non-finite value");
        return None;
    }
    let plausible = match unit {
        Unit::Celsius => (-50.0..=150.0).contains(&v),
        Unit::Percent => (0.0..=100.0).contains(&v),
        Unit::Megahertz => (0.0..=20_000.0).contains(&v),
        Unit::Volt => (-20.0..=20.0).contains(&v),
        Unit::Boolean => v == 0.0 || v == 1.0,
        _ => v >= 0.0,
    };
    if plausible {
        Some(v)
    } else {
        tracing::debug!(?unit, v, "discarding implausible value");
        None
    }
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
}
