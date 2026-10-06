//! The `performance` section: stress test settings (CPU and RAM).

use std::ops::RangeInclusive;

use serde_json::{json, Value};

/// Accepted values of `performance.cpuStopC`, in degrees Celsius.
pub const CPU_STOP_C: RangeInclusive<u32> = 60..=110;
/// Accepted values of `performance.ramSharePercent`.
pub const RAM_SHARE_PERCENT: RangeInclusive<u32> = 10..=90;

#[derive(Debug, Clone, PartialEq)]
pub struct PerformanceSettings {
    /// Whether a test stops when the CPU gets too hot.
    pub thermal_stop: bool,
    /// Stop threshold; `None` = automatic (Tjmax - 5, else 95); within [`CPU_STOP_C`].
    pub cpu_stop_c: Option<u32>,
    /// Overrides the profile's stop-on-error; `None` = as the profile says.
    pub stop_on_first_error: Option<bool>,
    /// Share of the available RAM a test may use; within [`RAM_SHARE_PERCENT`].
    pub ram_share_percent: u32,
    /// Whether the risk notice was already acknowledged.
    pub risk_notice_seen: bool,
}

impl Default for PerformanceSettings {
    fn default() -> Self {
        Self {
            thermal_stop: true,
            cpu_stop_c: None,
            stop_on_first_error: None,
            ram_share_percent: 70,
            risk_notice_seen: false,
        }
    }
}

impl PerformanceSettings {
    /// The JSON spelling; every key is always present.
    pub(super) fn encode(&self) -> Value {
        json!({
            "thermalStop": self.thermal_stop,
            "cpuStopC": self.cpu_stop_c,
            "stopOnFirstError": self.stop_on_first_error,
            "ramSharePercent": self.ram_share_percent,
            "riskNoticeSeen": self.risk_notice_seen,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_defaults_match_the_spec() {
        let p = PerformanceSettings::default();
        assert_eq!(
            p.encode(),
            json!({"thermalStop": true, "cpuStopC": null, "stopOnFirstError": null,
                   "ramSharePercent": 70, "riskNoticeSeen": false})
        );
        assert!(RAM_SHARE_PERCENT.contains(&p.ram_share_percent));
    }
}
