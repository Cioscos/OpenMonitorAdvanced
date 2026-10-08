//! The object a score is shared or exported as (DZ4, spec section 8.5): what the user
//! previews is byte for byte what is sent. No id, device id, serial or sample leaves.

use serde::Serialize;

use super::board::{validate_submission, ErrorCode, MAX_SUBMIT_BYTES};
use super::file::{KernelRate, ScoreFile};

/// What the score file does not hold: read at preview, send and export time.
#[derive(Debug, Clone, PartialEq)]
pub struct HostFacts {
    pub ram_gb: u32,
    pub os_build: String,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ShareScores {
    Cpu {
        single: Option<u32>,
        multi: Option<u32>,
    },
    Gpu {
        compute: Option<u32>,
        graphics: Option<u32>,
    },
    Disk {
        points: Option<u32>,
        #[serde(rename = "readMBs")]
        read_mbs: Option<f64>,
        #[serde(rename = "writeMBs")]
        write_mbs: Option<f64>,
    },
}

#[derive(Serialize)]
struct Hardware<'a> {
    model: &'a str,
    #[serde(rename = "ramGB")]
    ram_gb: u32,
    #[serde(rename = "osBuild")]
    os_build: &'a str,
}

#[derive(Serialize)]
struct Share<'a> {
    format: u32,
    #[serde(rename = "appVersion")]
    app_version: &'a str,
    category: &'a str,
    #[serde(rename = "scoreVersion")]
    score_version: &'a str,
    valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    overclock: Option<bool>,
    scores: ShareScores,
    kernels: &'a [KernelRate],
    hardware: Hardware<'a>,
    flags: &'a [String],
}

fn build(file: &ScoreFile, facts: &HostFacts, overclock: Option<bool>) -> Vec<u8> {
    let s = &file.scores;
    let scores = match file.category.as_str() {
        "gpu" => ShareScores::Gpu {
            compute: s.compute,
            graphics: s.graphics,
        },
        "disk" => ShareScores::Disk {
            points: s.points,
            read_mbs: s.read_mbs,
            write_mbs: s.write_mbs,
        },
        _ => ShareScores::Cpu {
            single: s.single,
            multi: s.multi,
        },
    };
    let share = Share {
        format: 1,
        app_version: &file.app_version,
        category: &file.category,
        score_version: &file.score_version,
        valid: file.valid,
        overclock,
        scores,
        kernels: &file.kernels,
        hardware: Hardware {
            model: &file.device.model,
            ram_gb: facts.ram_gb,
            os_build: &facts.os_build,
        },
        flags: &file.flags,
    };
    serde_json::to_vec_pretty(&share).expect("plain data serializes")
}

/// The export (section 8.5): the submission without `overclock`; not validated.
pub fn export_bytes(file: &ScoreFile, facts: &HostFacts) -> Vec<u8> {
    build(file, facts, None)
}

/// The submission, checked with the server's rules and size limit.
pub fn submission_bytes(
    file: &ScoreFile,
    facts: &HostFacts,
    overclock: bool,
) -> Result<Vec<u8>, ErrorCode> {
    let bytes = build(file, facts, Some(overclock));
    if bytes.len() > MAX_SUBMIT_BYTES {
        return Err(ErrorCode::BodyTooLarge);
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| ErrorCode::BadSchema)?;
    validate_submission(&value)?;
    Ok(bytes)
}

/// Why this score cannot be shared, if so (`provisional` before `shared`).
pub fn share_block(file: &ScoreFile) -> Option<&'static str> {
    if file.provisional {
        Some("provisional")
    } else if file.shared {
        Some("shared")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scores::{parse_score, BenchKernel, Device, ScoreSample, Scores};
    use serde_json::Value;

    fn facts() -> HostFacts {
        HostFacts {
            ram_gb: 32,
            os_build: "26300".into(),
        }
    }

    fn base(category: &str, version: &str, scores: Scores) -> ScoreFile {
        ScoreFile {
            format: 1,
            id: "0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d".into(),
            at: "2026-10-07T10:00:00Z".into(),
            category: category.into(),
            score_version: version.into(),
            provisional: false,
            isa: None,
            shader_digest: None,
            scores,
            kernels: vec![KernelRate {
                id: BenchKernel::Ntt,
                unit: "Mop/s".into(),
                single: Some(428.2),
                multi: None,
                value: None,
                spread: None,
                read: None,
                write: None,
            }],
            device: Device {
                model: "AMD Ryzen 7 7800X3D".into(),
                device_id: Some("cpu-0".into()),
                ..Device::default()
            },
            flags: vec!["battery".into()],
            valid: true,
            scaling: None,
            samples: vec![ScoreSample {
                t_ms: 1,
                temp_c: None,
                power_w: None,
                clock_mhz: None,
            }],
            app_version: "0.5.0".into(),
            load_version: None,
            disk_profile: None,
            shared: false,
        }
    }

    fn cpu() -> ScoreFile {
        base(
            "cpu",
            "cpu-1",
            Scores {
                single: Some(1491),
                multi: Some(1495),
                ..Scores::default()
            },
        )
    }

    fn disk(points: Option<u32>) -> ScoreFile {
        base(
            "disk",
            "disk-1",
            Scores {
                points,
                read_mbs: Some(7012.5),
                write_mbs: Some(5990.0),
                ..Scores::default()
            },
        )
    }

    fn json(b: &[u8]) -> Value {
        serde_json::from_slice(b).unwrap()
    }

    #[test]
    fn cpu_submission_passes_the_shared_rules() {
        let bytes = submission_bytes(&cpu(), &facts(), true).unwrap();
        let v = json(&bytes);
        assert_eq!(v["hardware"]["ramGB"], 32);
        assert_eq!(v["hardware"]["osBuild"], "26300");
        assert_eq!(v["hardware"]["model"], "AMD Ryzen 7 7800X3D");
        assert_eq!(v["scores"]["single"], 1491);
        assert_eq!(v["scores"]["multi"], 1495);
        assert_eq!(v["overclock"], true);
        let text = String::from_utf8(bytes).unwrap();
        for k in ["deviceId", "samples", "0b9c5a2e", "cpu-0"] {
            assert!(!text.contains(k), "{k} leaks");
        }
    }

    #[test]
    fn disk_submission_has_points_and_throughput() {
        let v = json(&submission_bytes(&disk(Some(1003)), &facts(), false).unwrap());
        assert_eq!(v["scores"]["points"], 1003);
        assert_eq!(v["scores"]["readMBs"], 7012.5);
        assert_eq!(v["scores"]["writeMBs"], 5990.0);
    }

    #[test]
    fn gpu_submission_validates() {
        let f = base(
            "gpu",
            "gpu-1",
            Scores {
                compute: Some(1480),
                graphics: Some(1520),
                ..Scores::default()
            },
        );
        let v = json(&submission_bytes(&f, &facts(), false).unwrap());
        assert_eq!(v["scores"]["graphics"], 1520);
    }

    #[test]
    fn b2_disk_score_is_bad_schema() {
        assert_eq!(
            submission_bytes(&disk(None), &facts(), false),
            Err(ErrorCode::BadSchema)
        );
    }

    #[test]
    fn export_has_no_overclock() {
        let text = String::from_utf8(export_bytes(&cpu(), &facts())).unwrap();
        assert!(!text.contains("overclock"));
        assert!(json(text.as_bytes())["scores"]["single"].is_number());
    }

    #[test]
    fn submission_field_order_is_fixed() {
        let text = String::from_utf8(submission_bytes(&cpu(), &facts(), false).unwrap()).unwrap();
        assert!(
            text.starts_with("{\n  \"format\": 1,\n  \"appVersion\""),
            "{text}"
        );
        let pos = |k: &str| text.find(&format!("\"{k}\":")).unwrap();
        let order = [
            "category",
            "scoreVersion",
            "valid",
            "overclock",
            "scores",
            "kernels",
            "hardware",
            "flags",
        ];
        assert!(order.windows(2).all(|w| pos(w[0]) < pos(w[1])));
    }

    #[test]
    fn oversized_submission_is_body_too_large() {
        let mut f = cpu();
        f.flags = vec!["x".repeat(MAX_SUBMIT_BYTES)];
        assert_eq!(
            submission_bytes(&f, &facts(), false),
            Err(ErrorCode::BodyTooLarge)
        );
    }

    #[test]
    fn share_block_reports_provisional_and_shared() {
        let mut f = cpu();
        assert_eq!(share_block(&f), None);
        f.shared = true;
        assert_eq!(share_block(&f), Some("shared"));
        f.provisional = true;
        assert_eq!(share_block(&f), Some("provisional"));
    }

    #[test]
    fn shared_mark_round_trips() {
        let mut f = cpu();
        f.shared = true;
        let back = parse_score(&serde_json::to_vec(&f).unwrap()).unwrap();
        assert!(back.shared);
    }
}
