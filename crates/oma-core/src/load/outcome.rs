//! Session outcome and the precedence between facts (plan DA13).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    Marginal,
    Errors,
    Crashed,
    Hung,
    SystemCrash,
    StoppedUser,
    StoppedThermal,
    Suspended,
    FailedToStart,
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
    pub hung: bool,
    pub errors: u64,
    pub error_cores: BTreeSet<u32>,
    /// Some counted error has no core (a reference disagreement): no «core N» verdict.
    pub coreless_errors: bool,
    pub thermal_stop: Option<f64>,
    pub suspended: bool,
    pub user_stop: bool,
    pub whea_corrected: u64,
    pub completed: bool,
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

/// Precedence: failed_to_start > system_crash > crashed > hung > errors >
/// stopped_thermal > suspended > stopped_user > marginal > passed.
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
        let setters: [(Outcome, Setter); 9] = [
            (Outcome::FailedToStart, |f| {
                f.failed_to_start = Some("x".into())
            }),
            (Outcome::SystemCrash, |f| f.system_crash = true),
            (Outcome::Crashed, |f| f.crashed = true),
            (Outcome::Hung, |f| f.hung = true),
            (Outcome::Errors, |f| f.errors = 1),
            (Outcome::StoppedThermal, |f| f.thermal_stop = Some(96.0)),
            (Outcome::Suspended, |f| f.suspended = true),
            (Outcome::StoppedUser, |f| f.user_stop = true),
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
