//! The saved score file (DB11, spec section 8.2).

use oma_ipc::load::Isa;
use serde::{Deserialize, Serialize};

use super::disk::DiskProfile;
use super::workloads::BenchKernel;
use crate::load::{session_file_name, FormatError};

/// Score file format, independent of the stress-session one.
pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scores {
    pub single: Option<u32>,
    pub multi: Option<u32>,
    /// GPU groups (DH11); `null` in a CPU file, absent before M8b2.
    #[serde(default)]
    pub compute: Option<u32>,
    #[serde(default)]
    pub graphics: Option<u32>,
    /// Disk groups (DC13): SEQ1M Q8T1 in MB/s, and the points (B1 only).
    #[serde(default, rename = "readMBs")]
    pub read_mbs: Option<f64>,
    #[serde(default, rename = "writeMBs")]
    pub write_mbs: Option<f64>,
    #[serde(default)]
    pub points: Option<u32>,
}

/// The best measure of a disk test in one direction (DC13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskRate {
    /// 10^6 bytes per second.
    pub mbs: f64,
    pub iops: f64,
    pub mean_lat_us: f64,
    pub p99_lat_us: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelRate {
    pub id: BenchKernel,
    pub unit: String,
    /// True units.
    pub single: Option<f64>,
    pub multi: Option<f64>,
    /// GPU load: the median of the windows (true units) and its spread.
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub spread: Option<f64>,
    /// Disk test: the best read and write measures (`unit` is `MB/s`).
    #[serde(default)]
    pub read: Option<DiskRate>,
    #[serde(default)]
    pub write: Option<DiskRate>,
}

/// A CPU (`cores`, `logical`) or a GPU (`cores` and `logical` 0, and the GPU fields).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub model: String,
    pub cores: u32,
    pub logical: u32,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub vendor_id: Option<u32>,
    #[serde(default)]
    pub dedicated_bytes: Option<u64>,
    #[serde(default)]
    pub integrated: Option<bool>,
    /// Disk: `nvme`, `sata_ssd`, `hdd`, `usb`, `virtual` or `other`.
    #[serde(default)]
    pub kind: Option<String>,
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
    /// `null` for a GPU score.
    pub isa: Option<Isa>,
    /// Shader bytecode digest of a GPU score (DH8).
    #[serde(default)]
    pub shader_digest: Option<String>,
    pub scores: Scores,
    pub kernels: Vec<KernelRate>,
    pub device: Device,
    pub flags: Vec<String>,
    pub valid: bool,
    pub scaling: Option<f64>,
    pub samples: Vec<ScoreSample>,
    pub app_version: String,
    pub load_version: Option<String>,
    /// Disk score: the test profile.
    #[serde(default)]
    pub disk_profile: Option<DiskProfile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreSummary {
    pub id: String,
    pub at: String,
    pub category: String,
    pub single: Option<u32>,
    pub multi: Option<u32>,
    pub compute: Option<u32>,
    pub graphics: Option<u32>,
    #[serde(default, rename = "readMBs")]
    pub read_mbs: Option<f64>,
    #[serde(default, rename = "writeMBs")]
    pub write_mbs: Option<f64>,
    #[serde(default)]
    pub points: Option<u32>,
    pub device_id: Option<String>,
    pub valid: bool,
    pub flags: Vec<String>,
    pub provisional: bool,
}

/// Rejects any format other than `FORMAT` (`FormatError::Future` carries the found value).
pub fn parse_score(bytes: &[u8]) -> Result<ScoreFile, FormatError> {
    #[derive(Deserialize)]
    struct Probe {
        format: u32,
    }
    let probe: Probe = serde_json::from_slice(bytes)?;
    if probe.format != FORMAT {
        return Err(FormatError::Future(probe.format));
    }
    Ok(serde_json::from_slice(bytes)?)
}

pub fn summary(s: &ScoreFile) -> ScoreSummary {
    ScoreSummary {
        id: s.id.clone(),
        at: s.at.clone(),
        category: s.category.clone(),
        single: s.scores.single,
        multi: s.scores.multi,
        compute: s.scores.compute,
        graphics: s.scores.graphics,
        read_mbs: s.scores.read_mbs,
        write_mbs: s.scores.write_mbs,
        points: s.scores.points,
        device_id: s.device.device_id.clone(),
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
    use crate::scores::DiskProfile;

    fn sample() -> ScoreFile {
        ScoreFile {
            format: FORMAT,
            id: "0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d".into(),
            at: "2026-10-07T10:00:00Z".into(),
            category: "cpu".into(),
            score_version: "cpu-1".into(),
            provisional: true,
            isa: Some(Isa::Avx512),
            shader_digest: None,
            scores: Scores {
                single: Some(1500),
                multi: None,
                ..Scores::default()
            },
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
                model: "CPU".into(),
                cores: 8,
                logical: 16,
                ..Device::default()
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
            disk_profile: None,
        }
    }

    #[test]
    fn score_file_round_trips_and_rejects_a_future_format() {
        let s = sample();
        let bytes = serde_json::to_vec(&s).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("\"scoreVersion\"") && text.contains("\"tMs\""));
        assert_eq!(parse_score(&bytes).unwrap(), s);
        let with_format =
            |n: u32| text.replace(&format!("\"format\":{FORMAT}"), &format!("\"format\":{n}"));
        let future = with_format(FORMAT + 1);
        assert!(matches!(
            parse_score(future.as_bytes()),
            Err(FormatError::Future(2))
        ));
        assert!(matches!(
            parse_score(with_format(0).as_bytes()),
            Err(FormatError::Future(0))
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

    /// A CPU score as M8a2 wrote it, before the GPU fields.
    const M8A2_CPU: &str = r#"{"format":1,"id":"0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d","at":"2026-10-07T10:00:00Z","category":"cpu","scoreVersion":"cpu-1","provisional":true,"isa":"avx512","scores":{"single":1500,"multi":18000},"kernels":[{"id":"ntt","unit":"Mop/s","single":428.2,"multi":5138.0}],"device":{"model":"AMD Ryzen 7 7800X3D","cores":8,"logical":16},"flags":[],"valid":true,"scaling":0.75,"samples":[{"tMs":5000,"tempC":60.0,"powerW":null,"clockMhz":5000.0}],"appVersion":"0.5.0","loadVersion":"0.5.0"}"#;

    #[test]
    fn cpu_score_files_from_m8a2_still_parse() {
        let s = parse_score(M8A2_CPU.as_bytes()).unwrap();
        assert_eq!(s.isa, Some(Isa::Avx512));
        assert_eq!((s.scores.single, s.scores.multi), (Some(1500), Some(18000)));
        assert_eq!((s.scores.compute, s.scores.graphics), (None, None));
        assert_eq!((s.kernels[0].value, s.kernels[0].spread), (None, None));
        assert_eq!(s.kernels[0].single, Some(428.2));
        assert_eq!(s.device.device_id, None);
        assert_eq!((s.device.vendor_id, s.device.integrated), (None, None));
        assert_eq!(s.shader_digest, None);
        let m = summary(&s);
        assert_eq!(
            (m.category.as_str(), m.compute, m.device_id),
            ("cpu", None, None)
        );
    }

    #[test]
    fn cpu_score_writes_the_gpu_fields_as_null() {
        let text = serde_json::to_string(&sample()).unwrap();
        for key in [
            "\"compute\":null",
            "\"graphics\":null",
            "\"value\":null",
            "\"spread\":null",
            "\"deviceId\":null",
            "\"vendorId\":null",
            "\"dedicatedBytes\":null",
            "\"integrated\":null",
            "\"shaderDigest\":null",
            "\"isa\":\"avx512\"",
        ] {
            assert!(text.contains(key), "{key} in {text}");
        }
    }

    #[test]
    fn gpu_score_file_round_trips() {
        let s = ScoreFile {
            category: "gpu".into(),
            score_version: "gpu-1".into(),
            isa: None,
            shader_digest: Some("0123456789abcdef".into()),
            scores: Scores {
                single: None,
                multi: None,
                compute: Some(1480),
                graphics: Some(1520),
                ..Scores::default()
            },
            kernels: vec![KernelRate {
                id: BenchKernel::Fma,
                unit: "TFLOPS".into(),
                single: None,
                multi: None,
                value: Some(47.18),
                spread: Some(0.012),
                read: None,
                write: None,
            }],
            device: Device {
                model: "NVIDIA GeForce RTX 4080".into(),
                cores: 0,
                logical: 0,
                device_id: Some("gpu-pci-0100".into()),
                vendor_id: Some(0x10de),
                dedicated_bytes: Some(17_171_480_576),
                integrated: Some(false),
                kind: None,
            },
            scaling: None,
            ..sample()
        };
        let text = serde_json::to_string(&s).unwrap();
        for key in [
            "\"isa\":null",
            "\"shaderDigest\":\"0123456789abcdef\"",
            "\"compute\":1480",
            "\"id\":\"fma\"",
            "\"deviceId\":\"gpu-pci-0100\"",
            "\"vendorId\":4318",
            "\"dedicatedBytes\":17171480576",
            "\"integrated\":false",
        ] {
            assert!(text.contains(key), "{key} in {text}");
        }
        assert_eq!(parse_score(text.as_bytes()).unwrap(), s);
        let m = summary(&s);
        assert_eq!(
            (
                m.category.as_str(),
                m.compute,
                m.graphics,
                m.device_id.as_deref()
            ),
            ("gpu", Some(1480), Some(1520), Some("gpu-pci-0100"))
        );
    }

    /// A GPU score as M8b2 wrote it, before the disk fields.
    const M8B2_GPU: &str = r#"{"format":1,"id":"0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d","at":"2026-10-07T10:00:00Z","category":"gpu","scoreVersion":"gpu-1","provisional":false,"isa":null,"shaderDigest":"0123456789abcdef","scores":{"single":null,"multi":null,"compute":1480,"graphics":1520},"kernels":[{"id":"fma","unit":"TFLOPS","single":null,"multi":null,"value":47.18,"spread":0.012}],"device":{"model":"NVIDIA GeForce RTX 4080","cores":0,"logical":0,"deviceId":"gpu-pci-0100","vendorId":4318,"dedicatedBytes":17171480576,"integrated":false},"flags":[],"valid":true,"scaling":null,"samples":[],"appVersion":"0.5.0","loadVersion":"0.5.0"}"#;

    #[test]
    fn gpu_and_cpu_score_files_still_parse() {
        let g = parse_score(M8B2_GPU.as_bytes()).unwrap();
        assert_eq!(g.scores.compute, Some(1480));
        assert_eq!(
            (g.scores.read_mbs, g.scores.write_mbs, g.scores.points),
            (None, None, None)
        );
        assert_eq!((&g.kernels[0].read, &g.kernels[0].write), (&None, &None));
        assert_eq!((g.device.kind, g.disk_profile), (None, None));
        let c = parse_score(M8A2_CPU.as_bytes()).unwrap();
        assert_eq!((c.scores.points, c.disk_profile), (None, None));
        let m = summary(&c);
        assert_eq!((m.read_mbs, m.write_mbs, m.points), (None, None, None));
    }

    #[test]
    fn disk_score_file_round_trips() {
        let rate = |mbs| DiskRate {
            mbs,
            iops: mbs * 1e6 / 1_048_576.0,
            mean_lat_us: 140.5,
            p99_lat_us: 310.0,
        };
        let s = ScoreFile {
            category: "disk".into(),
            score_version: "disk-1".into(),
            isa: None,
            scores: Scores {
                read_mbs: Some(7012.5),
                write_mbs: Some(5990.0),
                points: Some(1003),
                ..Scores::default()
            },
            kernels: vec![KernelRate {
                id: BenchKernel::Seq1mQ8t1,
                unit: "MB/s".into(),
                single: None,
                multi: None,
                value: None,
                spread: None,
                read: Some(rate(7012.5)),
                write: Some(rate(5990.0)),
            }],
            device: Device {
                model: "Samsung SSD 990 PRO".into(),
                device_id: Some("disk-0".into()),
                kind: Some("nvme".into()),
                ..Device::default()
            },
            scaling: None,
            disk_profile: Some(DiskProfile::B1),
            ..sample()
        };
        let text = serde_json::to_string(&s).unwrap();
        for key in [
            "\"category\":\"disk\"",
            "\"readMBs\":7012.5",
            "\"writeMBs\":5990.0",
            "\"points\":1003",
            "\"id\":\"seq1m_q8t1\"",
            "\"unit\":\"MB/s\"",
            "\"meanLatUs\":140.5",
            "\"p99LatUs\":310.0",
            "\"kind\":\"nvme\"",
            "\"diskProfile\":\"b1\"",
            "\"isa\":null",
        ] {
            assert!(text.contains(key), "{key} in {text}");
        }
        assert_eq!(parse_score(text.as_bytes()).unwrap(), s);
        let m = summary(&s);
        assert_eq!(
            (m.category.as_str(), m.read_mbs, m.write_mbs, m.points),
            ("disk", Some(7012.5), Some(5990.0), Some(1003))
        );
        assert!(serde_json::to_string(&m)
            .unwrap()
            .contains("\"readMBs\":7012.5"));
    }
}
