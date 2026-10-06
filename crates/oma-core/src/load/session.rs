//! Saved stress session and crash journal formats (spec §8.1, §8.3), and the
//! rules for their file names.

use std::collections::BTreeMap;

use oma_ipc::load::{CoreState, KernelId, Plan};
use serde::{Deserialize, Serialize};

use super::outcome::Outcome;
use super::plan::{Component, Objective, Preset, StartRequest};

pub const FORMAT: u32 = 1;
/// Errors kept in a session; the rest are counted in `errors_dropped`.
pub const MAX_ERRORS: usize = 200;
/// Sessions kept in the history folder.
pub const KEEP_SESSIONS: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("file format {0} is newer than this version understands")]
    Future(u32),
    #[error("invalid file: {0}")]
    Invalid(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub format: u32,
    pub id: String,
    /// RFC 3339, UTC.
    pub started_at: String,
    pub ended_at: Option<String>,
    pub component: Component,
    pub device: String,
    pub objective: Objective,
    pub preset: Preset,
    pub request: StartRequest,
    pub plan: Plan,
    pub outcome: Option<Outcome>,
    pub outcome_detail: Option<OutcomeDetail>,
    pub phases: Vec<PhaseResult>,
    pub cores: Vec<CoreResult>,
    /// At most `MAX_ERRORS`.
    pub errors: Vec<ErrorRecord>,
    pub errors_dropped: u64,
    /// Oldest events dropped once the list is full.
    #[serde(default)]
    pub events_dropped: u64,
    pub whea: WheaCounts,
    pub stats: Stats,
    pub samples: Vec<Sample>,
    pub events: Vec<SessionEvent>,
    pub app_version: String,
    pub load_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeDetail {
    /// The T3 key, e.g. `errors_core`.
    pub verdict: String,
    pub params: BTreeMap<String, String>,
    pub phase: Option<u32>,
    pub kernel: Option<KernelId>,
    pub core: Option<u32>,
    pub temp_c: Option<f64>,
    pub clock_mhz: Option<f64>,
    pub at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseResult {
    pub index: u32,
    pub kernel: KernelId,
    pub outcome: String,
    pub duration_ms: u64,
    pub checks: u64,
    pub errors: u64,
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreResult {
    pub core: u32,
    pub state: CoreState,
    pub first_error: Option<ErrorRecord>,
}

/// A `ComputeError` with the moment, temperature and clock it happened at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorRecord {
    // ComputeError's fields are single words, so camelCase and snake_case coincide.
    #[serde(flatten)]
    pub error: oma_ipc::load::ComputeError,
    pub at_ms: u64,
    pub temp_c: Option<f64>,
    pub clock_mhz: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WheaCounts {
    pub by_id: BTreeMap<u32, u64>,
    pub by_apic: BTreeMap<u32, u64>,
    pub unreadable: bool,
    pub last_record: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub temp_max_c: Option<f64>,
    pub temp_avg_c: Option<f64>,
    pub power_max_w: Option<f64>,
    pub power_avg_w: Option<f64>,
    pub clock_max_mhz: Option<f64>,
    pub clock_avg_mhz: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub t_ms: u64,
    pub temp_c: Option<f64>,
    pub power_w: Option<f64>,
    pub clock_mhz: Option<f64>,
}

/// The UI translates `code` with `performance.event.<code>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub at_ms: u64,
    pub code: String,
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub format: u32,
    pub session_id: String,
    pub plan_summary: String,
    pub phase_index: u32,
    pub kernel: Option<KernelId>,
    pub core: Option<u32>,
    /// RFC 3339, UTC.
    pub updated_at: String,
    pub clean_end: bool,
}

#[derive(Deserialize)]
struct FormatProbe {
    format: u32,
}

fn check_format(bytes: &[u8]) -> Result<(), FormatError> {
    let p: FormatProbe = serde_json::from_slice(bytes)?;
    if p.format > FORMAT {
        return Err(FormatError::Future(p.format));
    }
    Ok(())
}

pub fn parse_session(bytes: &[u8]) -> Result<Session, FormatError> {
    check_format(bytes)?;
    Ok(serde_json::from_slice(bytes)?)
}

pub fn parse_journal(bytes: &[u8]) -> Result<Journal, FormatError> {
    check_format(bytes)?;
    Ok(serde_json::from_slice(bytes)?)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub started_at: String,
    pub component: Component,
    pub objective: Objective,
    pub preset: Preset,
    pub duration_ms: u64,
    pub outcome: Option<Outcome>,
    pub verdict: Option<String>,
    pub params: BTreeMap<String, String>,
}

pub fn summary(s: &Session) -> SessionSummary {
    SessionSummary {
        id: s.id.clone(),
        started_at: s.started_at.clone(),
        component: s.component,
        objective: s.objective,
        preset: s.preset,
        // ponytail: sum of phase times, no RFC 3339 parsing; add chrono-free parsing if gaps matter.
        duration_ms: s.phases.iter().map(|p| p.duration_ms).sum(),
        outcome: s.outcome,
        verdict: s.outcome_detail.as_ref().map(|d| d.verdict.clone()),
        params: s
            .outcome_detail
            .as_ref()
            .map(|d| d.params.clone())
            .unwrap_or_default(),
    }
}

/// `AAAAMMGG-HHMMSS-<id>.json` from `2026-10-06T14:03:09Z`.
pub fn session_file_name(started_utc: &str, id: &str) -> String {
    let d: String = started_utc
        .chars()
        .take(19)
        .filter(char::is_ascii_digit)
        .collect();
    format!(
        "{}-{}-{id}.json",
        d.get(..8).unwrap_or(""),
        d.get(8..14).unwrap_or("")
    )
}

/// Only a lowercase uuid (8-4-4-4-12 hex digits).
pub fn is_session_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

/// Only `AAAAMMGG-HHMMSS-<uuid>.json`.
pub fn is_session_file_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".json") else {
        return false;
    };
    let b = stem.as_bytes();
    b.len() == 15 + 1 + 36
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'-'
        && b[9..15].iter().all(u8::is_ascii_digit)
        && b[15] == b'-'
        && is_session_id(&stem[16..])
}

/// The oldest names beyond `keep` (names sort chronologically).
pub fn prune(names: &[String], keep: usize) -> Vec<String> {
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    sorted.into_iter().skip(keep).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::load::{ComputeError, DataSize, ErrorKind, Isa, LoadMode, Placement};

    const ID: &str = "0b9f6c1e-7d2a-4c53-9a1e-3f5d8e2b7a10";

    fn session() -> Session {
        let error = ComputeError {
            phase: 1,
            kernel: KernelId::K2,
            isa: Isa::Avx2,
            kind: ErrorKind::Mismatch,
            logical: Some(6),
            core: Some(3),
            iteration: 7,
            expected: 1,
            actual: 2,
            seed: 9,
        };
        Session {
            format: FORMAT,
            id: ID.into(),
            started_at: "2026-10-06T14:03:09Z".into(),
            ended_at: None,
            component: Component::Cpu,
            device: "AMD Ryzen".into(),
            objective: Objective::Overclock,
            preset: Preset::Standard,
            request: StartRequest {
                component: Component::Cpu,
                objective: Objective::Overclock,
                preset: Preset::Standard,
                custom: None,
                retry_core: None,
            },
            plan: Plan {
                seed: 1,
                ram_bytes: 0,
                phases: vec![oma_ipc::load::Phase {
                    kernel: KernelId::K2,
                    alt_kernel: None,
                    isa: Isa::Avx2,
                    size: DataSize::L2,
                    mode: LoadMode::Steady,
                    placement: Placement::AllLogical,
                    duration_s: 60,
                    per_core_s: None,
                    both_smt: false,
                    cores: None,
                    patterns: vec![],
                    stop_on_error: false,
                }],
            },
            outcome: Some(Outcome::Errors),
            outcome_detail: Some(OutcomeDetail {
                verdict: "errors_core".into(),
                params: [("core".to_string(), "3".to_string())].into(),
                phase: Some(1),
                kernel: Some(KernelId::K2),
                core: Some(3),
                temp_c: Some(80.5),
                clock_mhz: None,
                at_ms: Some(1234),
            }),
            phases: vec![PhaseResult {
                index: 0,
                kernel: KernelId::K2,
                outcome: "errors".into(),
                duration_ms: 5000,
                checks: 10,
                errors: 1,
                skipped: None,
            }],
            cores: vec![CoreResult {
                core: 3,
                state: CoreState::Failed,
                first_error: Some(ErrorRecord {
                    error,
                    at_ms: 1234,
                    temp_c: None,
                    clock_mhz: Some(4500.0),
                }),
            }],
            errors: vec![],
            errors_dropped: 0,
            events_dropped: 0,
            whea: WheaCounts::default(),
            stats: Stats::default(),
            samples: vec![Sample {
                t_ms: 0,
                temp_c: Some(50.0),
                power_w: None,
                clock_mhz: None,
            }],
            events: vec![],
            app_version: "0.5.0".into(),
            load_version: None,
        }
    }

    #[test]
    fn session_round_trips_and_ignores_unknown_fields() {
        let s = session();
        let mut v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["startedAt"], "2026-10-06T14:03:09Z");
        assert!(v["endedAt"].is_null());
        v["somethingNew"] = serde_json::json!(1);
        let back = parse_session(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(summary(&s).duration_ms, 5000);
        assert_eq!(summary(&s).verdict.as_deref(), Some("errors_core"));
    }

    #[test]
    fn journal_round_trips() {
        let j = Journal {
            format: 1,
            session_id: ID.into(),
            plan_summary: "cpu overclock".into(),
            phase_index: 2,
            kernel: Some(KernelId::K1),
            core: None,
            updated_at: "2026-10-06T14:03:09Z".into(),
            clean_end: false,
        };
        assert_eq!(parse_journal(&serde_json::to_vec(&j).unwrap()).unwrap(), j);
    }

    #[test]
    fn future_format_is_rejected() {
        let mut v = serde_json::to_value(session()).unwrap();
        v["format"] = serde_json::json!(2);
        let bytes = serde_json::to_vec(&v).unwrap();
        assert!(matches!(parse_session(&bytes), Err(FormatError::Future(2))));
        assert!(matches!(
            parse_journal(br#"{"format":9}"#),
            Err(FormatError::Future(9))
        ));
    }

    #[test]
    fn truncated_json_is_an_error() {
        let bytes = serde_json::to_vec(&session()).unwrap();
        assert!(parse_session(&bytes[..bytes.len() / 2]).is_err());
        assert!(parse_journal(b"").is_err());
    }

    #[test]
    fn file_name_form() {
        let n = session_file_name("2026-10-06T14:03:09Z", ID);
        assert_eq!(n, format!("20261006-140309-{ID}.json"));
        assert!(is_session_file_name(&n));
    }

    #[test]
    fn ids_outside_the_session_form_are_rejected() {
        assert!(is_session_id(ID));
        for bad in [
            "..\\x",
            "a/b",
            "0B9F6C1E-7D2A-4C53-9A1E-3F5D8E2B7A10",
            "",
            "0b9f6c1e7d2a4c539a1e3f5d8e2b7a10",
        ] {
            assert!(!is_session_id(bad), "{bad}");
        }
        for bad in [
            format!("20261006-140309-{ID}"),
            format!("..\\20261006-140309-{ID}.json"),
            format!("x0261006-140309-{ID}.json"),
            format!("20261006-140309-{}.json", ID.to_uppercase()),
            "a/b.json".to_string(),
        ] {
            assert!(!is_session_file_name(&bad), "{bad}");
        }
    }

    #[test]
    fn prune_keeps_the_newest_500() {
        let names: Vec<String> = (0..503)
            .map(|i| format!("2026{:04}-000000-{ID}.json", i + 1))
            .collect();
        let gone = prune(&names, KEEP_SESSIONS);
        assert_eq!(gone.len(), 3);
        assert!(gone.iter().all(|n| names[..3].contains(n)));
        assert!(prune(&names[..10], KEEP_SESSIONS).is_empty());
    }
}
