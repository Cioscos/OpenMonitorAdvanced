//! The `performance` section: stress test settings (CPU and RAM).

use std::ops::RangeInclusive;

use serde_json::{json, Value};

/// Accepted values of `performance.cpuStopC`, in degrees Celsius.
pub const CPU_STOP_C: RangeInclusive<u32> = 60..=110;
/// Accepted values of `performance.gpuStopC`, in degrees Celsius.
pub const GPU_STOP_C: RangeInclusive<u32> = 60..=110;
/// Accepted values of `performance.diskStopC`, in degrees Celsius.
pub const DISK_STOP_C: RangeInclusive<u32> = 40..=90;
/// Longest accepted `performance.diskFolder`, in bytes.
pub const DISK_FOLDER_MAX: usize = 1024;
/// Accepted values of `performance.ramSharePercent`.
pub const RAM_SHARE_PERCENT: RangeInclusive<u32> = 10..=90;

#[derive(Debug, Clone, PartialEq)]
pub struct PerformanceSettings {
    /// Whether a test stops when the CPU gets too hot.
    pub thermal_stop: bool,
    /// Stop threshold; `None` = automatic (Tjmax - 5, else 95); within [`CPU_STOP_C`].
    pub cpu_stop_c: Option<u32>,
    /// GPU core stop threshold; within [`GPU_STOP_C`], 90 by default.
    pub gpu_stop_c: u32,
    /// Overrides the profile's stop-on-error; `None` = as the profile says.
    pub stop_on_first_error: Option<bool>,
    /// Share of the available RAM a test may use; within [`RAM_SHARE_PERCENT`].
    pub ram_share_percent: u32,
    /// Whether the risk notice was already acknowledged.
    pub risk_notice_seen: bool,
    /// Disk stop threshold; `None` = automatic (WCTEMP, else 70); within [`DISK_STOP_C`].
    pub disk_stop_c: Option<u32>,
    /// The folder of the last disk test, at most [`DISK_FOLDER_MAX`] bytes.
    pub disk_folder: Option<String>,
}

impl Default for PerformanceSettings {
    fn default() -> Self {
        Self {
            thermal_stop: true,
            cpu_stop_c: None,
            gpu_stop_c: 90,
            stop_on_first_error: None,
            ram_share_percent: 70,
            risk_notice_seen: false,
            disk_stop_c: None,
            disk_folder: None,
        }
    }
}

impl PerformanceSettings {
    /// The JSON spelling; every key is always present.
    pub(super) fn encode(&self) -> Value {
        json!({
            "thermalStop": self.thermal_stop,
            "cpuStopC": self.cpu_stop_c,
            "gpuStopC": self.gpu_stop_c,
            "stopOnFirstError": self.stop_on_first_error,
            "ramSharePercent": self.ram_share_percent,
            "riskNoticeSeen": self.risk_notice_seen,
            "diskStopC": self.disk_stop_c,
            "diskFolder": self.disk_folder,
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
            json!({"thermalStop": true, "cpuStopC": null, "gpuStopC": 90, "stopOnFirstError": null,
                   "ramSharePercent": 70, "riskNoticeSeen": false, "diskStopC": null,
                   "diskFolder": null})
        );
        assert!(RAM_SHARE_PERCENT.contains(&p.ram_share_percent));
    }
}
