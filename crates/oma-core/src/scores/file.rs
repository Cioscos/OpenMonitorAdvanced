//! The saved score file (DB11, spec section 8.2).

use oma_ipc::load::Isa;
use serde::{Deserialize, Serialize};

use super::workloads::BenchKernel;
use crate::load::{check_format, session_file_name, FormatError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scores {
    pub single: Option<u32>,
    pub multi: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelRate {
    pub id: BenchKernel,
    pub unit: String,
    /// True units.
    pub single: Option<f64>,
    pub multi: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub model: String,
    pub cores: u32,
    pub logical: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreSample {
    pub t_ms: u64,
    pub temp_c: Option<f64>,
    pub power_w: Option<f64>,
    pub clock_mhz: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreFile {
    pub format: u32,
    pub id: String,
    /// RFC 3339, UTC.
    pub at: String,
    pub category: String,
    pub score_version: String,
    pub provisional: bool,
    pub isa: Isa,
    pub scores: Scores,
    pub kernels: Vec<KernelRate>,
    pub device: Device,
    pub flags: Vec<String>,
    pub valid: bool,
    pub scaling: Option<f64>,
    pub samples: Vec<ScoreSample>,
    pub app_version: String,
    pub load_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreSummary {
    pub id: String,
    pub at: String,
    pub single: Option<u32>,
    pub multi: Option<u32>,
    pub valid: bool,
    pub flags: Vec<String>,
    pub provisional: bool,
}

/// Rejects a file of a newer format.
pub fn parse_score(bytes: &[u8]) -> Result<ScoreFile, FormatError> {
    check_format(bytes)?;
    Ok(serde_json::from_slice(bytes)?)
}

pub fn summary(s: &ScoreFile) -> ScoreSummary {
    ScoreSummary {
        id: s.id.clone(),
        at: s.at.clone(),
        single: s.scores.single,
        multi: s.scores.multi,
        valid: s.valid,
        flags: s.flags.clone(),
        provisional: s.provisional,
    }
}

/// `AAAAMMGG-HHMMSS-<id>.json`, the same form as the stress sessions.
pub fn score_file_name(at_utc: &str, id: &str) -> String {
    session_file_name(at_utc, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::FORMAT;

    fn sample() -> ScoreFile {
        ScoreFile {
            format: FORMAT,
            id: "0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d".into(),
            at: "2026-10-07T10:00:00Z".into(),
            category: "cpu".into(),
            score_version: "cpu-1".into(),
            provisional: true,
            isa: Isa::Avx512,
            scores: Scores {
                single: Some(1500),
                multi: None,
            },
            kernels: vec![KernelRate {
                id: BenchKernel::Ntt,
                unit: "Mop/s".into(),
                single: Some(428.2),
                multi: None,
            }],
            device: Device {
                model: "CPU".into(),
                cores: 8,
                logical: 16,
            },
            flags: vec!["battery".into()],
            valid: true,
            scaling: Some(0.75),
            samples: vec![ScoreSample {
                t_ms: 5000,
                temp_c: Some(60.0),
                power_w: None,
                clock_mhz: Some(5000.0),
            }],
            app_version: "0.5.0".into(),
            load_version: None,
        }
    }

    #[test]
    fn score_file_round_trips_and_rejects_a_future_format() {
        let s = sample();
        let bytes = serde_json::to_vec(&s).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("\"scoreVersion\"") && text.contains("\"tMs\""));
        assert_eq!(parse_score(&bytes).unwrap(), s);
        let future = text.replace("\"format\":1", "\"format\":2");
        assert!(matches!(
            parse_score(future.as_bytes()),
            Err(FormatError::Future(2))
        ));
        assert!(parse_score(b"{\"format\":1").is_err());
        assert_eq!(
            score_file_name(&s.at, &s.id),
            "20261007-100000-0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d.json"
        );
        let m = summary(&s);
        assert_eq!(
            (m.id.as_str(), m.single, m.multi),
            (s.id.as_str(), Some(1500), None)
        );
    }
}
