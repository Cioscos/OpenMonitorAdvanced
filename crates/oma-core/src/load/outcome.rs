//! Session outcome and the precedence between facts (plan DA13).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::stability::MIN_STABILITY;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    Marginal,
    Errors,
    Crashed,
    /// The GPU was reset by Windows or the driver (DG8).
    DeviceLost,
    Hung,
    /// A disk test filled the volume (DC10).
    StoppedDiskFull,
    SystemCrash,
    StoppedUser,
    StoppedThermal,
    Suspended,
    FailedToStart,
    /// Completed, but the throughput was not steady (DG7).
    LowStability,
}

/// The `failed_to_start` reason of a session that completed without running anything.
pub const NOTHING_RAN: &str = "performance.start.nothing_ran";

/// What happened during a session; `decide` picks the outcome.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutcomeFacts {
    pub failed_to_start: Option<String>,
    pub system_crash: bool,
    pub app_closed: bool,
    pub crashed: bool,
    /// The `GetDeviceRemovedReason` HRESULT; 0 when only the exit code told us.
    pub device_lost: Option<u32>,
    pub hung: bool,
    pub errors: u64,
    pub error_cores: BTreeSet<u32>,
    /// Some counted error has no core (a reference disagreement): no «core N» verdict.
    pub coreless_errors: bool,
    /// Disk runs: `oma-load` reported `disk_full`.
    pub disk_full: bool,
    pub thermal_stop: Option<f64>,
    pub suspended: bool,
    pub user_stop: bool,
    pub whea_corrected: u64,
    pub completed: bool,
    /// Throughput stability of a GPU run, 0-1 (DG7).
    pub stability: Option<f64>,
    /// Completed with no check, or with every phase skipped.
    pub nothing_ran: bool,
}

/// The text key (table T3 of the plan) with its parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictKey {
    pub key: &'static str,
    pub params: BTreeMap<String, String>,
}

fn key(key: &'static str) -> VerdictKey {
    VerdictKey {
        key,
        params: BTreeMap::new(),
    }
}

/// Precedence: failed_to_start > system_crash > crashed > device_lost > hung > errors >
/// stopped_disk_full > stopped_thermal > suspended > stopped_user > low_stability > marginal > passed.
pub fn decide(f: &OutcomeFacts) -> (Outcome, VerdictKey) {
    if let Some(reason) = &f.failed_to_start {
        let mut k = key("failed_to_start");
        k.params.insert("reason".into(), reason.clone());
        return (Outcome::FailedToStart, k);
    }
    // A session that ran nothing has no verdict to give, unless its references disagreed.
    if f.nothing_ran && f.errors == 0 {
        let mut k = key("failed_to_start");
        k.params.insert("reason".into(), NOTHING_RAN.into());
        return (Outcome::FailedToStart, k);
    }
    if f.system_crash {
        return (Outcome::SystemCrash, key("system_crash"));
    }
    if f.crashed || f.app_closed {
        let k = if f.app_closed {
            "crashed_app"
        } else {
            "crashed"
        };
        return (Outcome::Crashed, key(k));
    }
    if let Some(code) = f.device_lost {
        let mut k = key("device_lost");
        if code != 0 {
            k.params.insert("code".into(), format!("0x{code:08X}"));
        }
        return (Outcome::DeviceLost, k);
    }
    if f.hung {
        return (Outcome::Hung, key("hung"));
    }
    if f.errors > 0 {
        let mut it = f.error_cores.iter();
        return match (it.next(), it.next()) {
            (Some(core), None) if !f.coreless_errors => {
                let mut k = key("errors_core");
                k.params.insert("core".into(), core.to_string());
                (Outcome::Errors, k)
            }
            _ => (Outcome::Errors, key("errors")),
        };
    }
    if f.disk_full {
        return (Outcome::StoppedDiskFull, key("stopped_disk_full"));
    }
    if let Some(t) = f.thermal_stop {
        let mut k = key("stopped_thermal");
        k.params.insert("temp".into(), format!("{t:.0}"));
        return (Outcome::StoppedThermal, k);
    }
    if f.suspended {
        return (Outcome::Suspended, key("suspended"));
    }
    if f.user_stop {
        return (Outcome::StoppedUser, key("stopped_user"));
    }
    if let Some(s) = f.stability.filter(|s| f.completed && *s < MIN_STABILITY) {
        let mut k = key("low_stability");
        k.params
            .insert("stability".into(), format!("{:.1}", s * 100.0));
        return (Outcome::LowStability, k);
    }
    if f.whea_corrected > 0 {
        return (Outcome::Marginal, key("marginal"));
    }
    (Outcome::Passed, key("passed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    type Setter = fn(&mut OutcomeFacts);

    fn base() -> OutcomeFacts {
        OutcomeFacts {
            completed: true,
            ..Default::default()
        }
    }

    #[test]
    fn precedence_follows_the_plan_table() {
        let setters: [(Outcome, Setter); 12] = [
            (Outcome::FailedToStart, |f| {
                f.failed_to_start = Some("x".into())
            }),
            (Outcome::SystemCrash, |f| f.system_crash = true),
            (Outcome::Crashed, |f| f.crashed = true),
            (Outcome::DeviceLost, |f| f.device_lost = Some(1)),
            (Outcome::Hung, |f| f.hung = true),
            (Outcome::Errors, |f| f.errors = 1),
            (Outcome::StoppedDiskFull, |f| f.disk_full = true),
            (Outcome::StoppedThermal, |f| f.thermal_stop = Some(96.0)),
            (Outcome::Suspended, |f| f.suspended = true),
            (Outcome::StoppedUser, |f| f.user_stop = true),
            (Outcome::LowStability, |f| f.stability = Some(0.5)),
            (Outcome::Marginal, |f| f.whea_corrected = 2),
        ];
        // Every pair: the earlier entry wins.
        for (i, (hi, set_hi)) in setters.iter().enumerate() {
            for (lo, set_lo) in &setters[i + 1..] {
                let mut f = base();
                set_hi(&mut f);
                set_lo(&mut f);
                assert_eq!(decide(&f).0, *hi, "{hi:?} over {lo:?}");
            }
            let mut f = base();
            set_hi(&mut f);
            assert_eq!(decide(&f).0, *hi);
        }
        assert_eq!(decide(&base()).0, Outcome::Passed);
    }

    #[test]
    fn stopped_disk_full_ranks_after_errors_and_before_thermal() {
        let mut f = base();
        f.disk_full = true;
        f.thermal_stop = Some(80.0);
        f.user_stop = true;
        let (o, k) = decide(&f);
        assert_eq!((o, k.key), (Outcome::StoppedDiskFull, "stopped_disk_full"));
        f.errors = 1;
        assert_eq!(decide(&f).0, Outcome::Errors);
        f.errors = 0;
        f.hung = true;
        assert_eq!(decide(&f).0, Outcome::Hung);
    }

    #[test]
    fn single_error_core_is_unstable_core_n() {
        let mut f = base();
        f.errors = 3;
        f.error_cores = [4].into();
        let (o, k) = decide(&f);
        assert_eq!(o, Outcome::Errors);
        assert_eq!(k.key, "errors_core");
        assert_eq!(k.params["core"], "4");
    }

    #[test]
    fn several_cores_is_errors_found() {
        let mut f = base();
        f.errors = 3;
        f.error_cores = [1, 4].into();
        assert_eq!(decide(&f).1.key, "errors");
        f.error_cores.clear();
        assert_eq!(decide(&f).1.key, "errors");
    }

    #[test]
    fn a_core_less_error_is_plain_errors() {
        let mut f = base();
        f.errors = 2;
        f.error_cores = [4].into();
        f.coreless_errors = true;
        assert_eq!(decide(&f).1.key, "errors");
    }

    #[test]
    fn nothing_ran_is_failed_to_start() {
        let mut f = base();
        f.nothing_ran = true;
        let (o, k) = decide(&f);
        assert_eq!(o, Outcome::FailedToStart);
        assert_eq!(k.params["reason"], NOTHING_RAN);
        // A disagreement of the reference is still an error.
        f.errors = 1;
        assert_eq!(decide(&f).0, Outcome::Errors);
    }

    #[test]
    fn stopped_after_errors_is_errors() {
        let mut f = base();
        f.user_stop = true;
        f.errors = 1;
        assert_eq!(decide(&f).0, Outcome::Errors);
    }

    #[test]
    fn whea_corrected_without_errors_is_marginal() {
        let mut f = base();
        f.whea_corrected = 1;
        assert_eq!(decide(&f).0, Outcome::Marginal);
    }

    #[test]
    fn device_lost_beats_hung_and_errors() {
        let mut f = base();
        f.device_lost = Some(0x887A_0006);
        f.hung = true;
        f.errors = 3;
        let (o, k) = decide(&f);
        assert_eq!(o, Outcome::DeviceLost);
        assert_eq!(k.key, "device_lost");
        assert_eq!(k.params["code"], "0x887A0006");
        // An exit code alone carries no HRESULT.
        f.device_lost = Some(0);
        assert!(decide(&f).1.params.is_empty());
    }

    #[test]
    fn crashed_beats_device_lost() {
        let mut f = base();
        f.device_lost = Some(1);
        f.crashed = true;
        assert_eq!(decide(&f).0, Outcome::Crashed);
    }

    #[test]
    fn low_stability_only_when_completed() {
        let mut f = base();
        f.stability = Some(0.96);
        assert_eq!(decide(&f).0, Outcome::LowStability);
        f.stability = Some(0.97);
        assert_eq!(decide(&f).0, Outcome::Passed);
        f.stability = Some(0.5);
        f.completed = false;
        assert_eq!(decide(&f).0, Outcome::Passed);
    }

    #[test]
    fn stopped_user_beats_low_stability() {
        let mut f = base();
        f.stability = Some(0.5);
        f.user_stop = true;
        assert_eq!(decide(&f).0, Outcome::StoppedUser);
        // Low stability beats marginal.
        let mut f = base();
        f.stability = Some(0.5);
        f.whea_corrected = 1;
        assert_eq!(decide(&f).0, Outcome::LowStability);
    }

    #[test]
    fn low_stability_key_carries_the_percent() {
        let mut f = base();
        f.stability = Some(0.953);
        let (o, k) = decide(&f);
        assert_eq!(o, Outcome::LowStability);
        assert_eq!(k.key, "low_stability");
        assert_eq!(k.params["stability"], "95.3");
    }

    #[test]
    fn crash_keys_and_params() {
        let mut f = base();
        f.crashed = true;
        f.app_closed = true;
        assert_eq!(decide(&f).1.key, "crashed_app");
        f.app_closed = false;
        assert_eq!(decide(&f).1.key, "crashed");
        let mut f = base();
        f.app_closed = true;
        assert_eq!(decide(&f).0, Outcome::Crashed);
        let mut f = base();
        f.thermal_stop = Some(96.4);
        assert_eq!(decide(&f).1.params["temp"], "96");
    }
}
