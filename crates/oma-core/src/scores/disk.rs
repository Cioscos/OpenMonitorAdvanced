//! Disk benchmark (DC5, DC13): the tests of the two profiles, the plan, the fixed-scale
//! reference rates and points.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use oma_ipc::load::{
    DataSize, DiskJob, DiskTarget, Isa, KernelId, LoadMode, Phase, Placement, Plan,
};
use serde::{Deserialize, Serialize};

use super::file::ScoreFile;
use super::plan::{BenchMode, BenchStep};
use super::score::{geomean, round4, BaselineError};
use super::workloads::BenchKernel;
use crate::load::disk_reserve;

pub const DISK_SCORE_VERSION: &str = "disk-1";
pub const DISK_POINTS: f64 = 1000.0;
/// The benchmark file (1 GiB).
pub const DISK_BENCH_FILE: u64 = 1 << 30;

const GIB: u64 = 1 << 30;
const KIB: u32 = 1 << 10;
/// Seconds of every `disk_bench` phase, and the pause before it.
const BENCH_PHASE_S: u32 = 5;
const BENCH_PAUSE_MS: u32 = 5000;
/// Bookkeeping duration of the fill phase: `oma-load` ignores it.
const FILL_S: u32 = 900;

/// B1 is the scored profile; B2 (NVMe-style tests) has no points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskProfile {
    B1,
    B2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskTest {
    pub id: BenchKernel,
    pub block_bytes: u32,
    pub queue: u16,
    pub threads: u16,
    pub random: bool,
}

const fn test(id: BenchKernel, block_kib: u32, queue: u16, threads: u16, random: bool) -> DiskTest {
    DiskTest {
        id,
        block_bytes: block_kib * KIB,
        queue,
        threads,
        random,
    }
}

pub const B1_TESTS: [DiskTest; 4] = [
    test(BenchKernel::Seq1mQ8t1, 1024, 8, 1, false),
    test(BenchKernel::Seq1mQ1t1, 1024, 1, 1, false),
    test(BenchKernel::Rnd4kQ32t1, 4, 32, 1, true),
    test(BenchKernel::Rnd4kQ1t1, 4, 1, 1, true),
];

pub const B2_TESTS: [DiskTest; 4] = [
    test(BenchKernel::Seq1mQ8t1, 1024, 8, 1, false),
    test(BenchKernel::Seq128kQ32t1, 128, 32, 1, false),
    test(BenchKernel::Rnd4kQ32t16, 4, 32, 16, true),
    test(BenchKernel::Rnd4kQ1t1, 4, 1, 1, true),
];

pub fn disk_tests(profile: DiskProfile) -> &'static [DiskTest; 4] {
    match profile {
        DiskProfile::B1 => &B1_TESTS,
        DiskProfile::B2 => &B2_TESTS,
    }
}

fn job(t: &DiskTest, read: bool, write_cap_bytes: Option<u64>) -> DiskJob {
    DiskJob {
        block_bytes: t.block_bytes,
        seq_block_bytes: t.block_bytes,
        random_percent: if t.random { 100 } else { 0 },
        read_percent: if read { 100 } else { 0 },
        queue: t.queue,
        threads: t.threads,
        write_cap_bytes,
        cycles: None,
        rate_limit_bps: None,
    }
}

fn phase(kernel: KernelId, duration_s: u32, pause_before_ms: u32, disk: DiskJob) -> Phase {
    Phase {
        kernel,
        alt_kernel: None,
        isa: Isa::Sse2,
        size: DataSize::Auto,
        mode: LoadMode::Steady,
        placement: Placement::AllLogical,
        duration_s,
        per_core_s: None,
        both_smt: false,
        cores: None,
        patterns: vec![],
        stop_on_error: true,
        iterations: None,
        pause_before_ms,
        windows: None,
        disk: Some(disk),
    }
}

/// Fill, then every test read (warm-up + 3 measures), then written the same way: 33 phases.
/// The writes are capped (DC5): SEQ 1 GiB warm-up and 4 GiB measures, RND 1 GiB and 2 GiB.
pub fn disk_bench_plan(
    dir: String,
    volume_bytes: u64,
    compressible: bool,
    profile: DiskProfile,
    seed: u64,
) -> (Plan, Vec<BenchStep>) {
    let fill = test(BenchKernel::DiskFill, 1024, 4, 1, false);
    let mut phases = vec![phase(
        KernelId::DiskFill,
        FILL_S,
        0,
        job(&fill, false, None),
    )];
    let mut steps = vec![BenchStep {
        kernel: BenchKernel::DiskFill,
        mode: BenchMode::Write,
        rep: 0,
    }];
    for (mode, read) in [(BenchMode::Read, true), (BenchMode::Write, false)] {
        for t in disk_tests(profile) {
            for rep in 0..=3u8 {
                let cap = (!read).then_some(match (t.random, rep) {
                    (_, 0) => GIB,
                    (false, _) => 4 * GIB,
                    (true, _) => 2 * GIB,
                });
                phases.push(phase(
                    KernelId::DiskBench,
                    BENCH_PHASE_S,
                    BENCH_PAUSE_MS,
                    job(t, read, cap),
                ));
                steps.push(BenchStep {
                    kernel: t.id,
                    mode,
                    rep,
                });
            }
        }
    }
    let plan = Plan {
        seed,
        ram_bytes: 0,
        phases,
        gpu: None,
        disk: Some(DiskTarget {
            dir,
            file_bytes: DISK_BENCH_FILE,
            compressible,
            reserve_bytes: disk_reserve(volume_bytes),
        }),
    };
    (plan, steps)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiskBaseline {
    pub version: String,
    pub provisional: bool,
    /// MB/s (10^6 bytes per second), like the score file's `mbs`.
    pub read: BTreeMap<BenchKernel, f64>,
    pub write: BTreeMap<BenchKernel, f64>,
}

const DISK_BASELINE_JSON: &str = include_str!("disk-1-baseline.json");

fn checked(b: DiskBaseline) -> Result<DiskBaseline, BaselineError> {
    let complete = |m: &BTreeMap<BenchKernel, f64>| {
        B1_TESTS
            .iter()
            .all(|t| m.get(&t.id).is_some_and(|v| v.is_finite() && *v > 0.0))
    };
    if b.version != DISK_SCORE_VERSION {
        Err(BaselineError(format!(
            "version {} is not {DISK_SCORE_VERSION}",
            b.version
        )))
    } else if !complete(&b.read) || !complete(&b.write) {
        Err(BaselineError("a test is missing or not positive".into()))
    } else {
        Ok(b)
    }
}

fn parse_disk_baseline(text: &str) -> Result<DiskBaseline, BaselineError> {
    checked(serde_json::from_str(text).map_err(|e| BaselineError(e.to_string()))?)
}

/// The reference rates of the disk fixed scale (compiled in).
pub fn disk_baseline() -> &'static DiskBaseline {
    static BASELINE: OnceLock<DiskBaseline> = OnceLock::new();
    BASELINE.get_or_init(|| {
        parse_disk_baseline(DISK_BASELINE_JSON).expect("disk-1-baseline.json is valid")
    })
}

/// `round(1000 x geometric mean(rate / reference))` over the 8 B1 measures (MB/s); `None`
/// when one is missing, which is always the case for B2 (DC5).
pub fn disk_points(
    read: &BTreeMap<BenchKernel, f64>,
    write: &BTreeMap<BenchKernel, f64>,
    baseline: &DiskBaseline,
) -> Option<u32> {
    let ratios = [(read, &baseline.read), (write, &baseline.write)]
        .into_iter()
        .flat_map(|(m, r)| {
            B1_TESTS
                .iter()
                .map(move |t| Some(*m.get(&t.id)? / r[&t.id]))
        });
    let p = (DISK_POINTS * geomean(ratios)?).round();
    (p.is_finite() && p <= f64::from(u32::MAX)).then_some(p as u32)
}

/// The baseline calibrated from a B1 score file: the best MB/s of each test to 4
/// significant digits, `provisional: false`. Refuses B2, a run that is invalid, flagged,
/// of another score version or incomplete.
pub fn disk_calibration_from(score: &ScoreFile) -> Result<DiskBaseline, BaselineError> {
    if !score.valid {
        return Err(BaselineError("the run is not valid".into()));
    }
    if !score.flags.is_empty() {
        return Err(BaselineError(format!(
            "the run has flags: {}",
            score.flags.join(", ")
        )));
    }
    if score.score_version != DISK_SCORE_VERSION {
        return Err(BaselineError(format!(
            "score version {} is not {DISK_SCORE_VERSION}",
            score.score_version
        )));
    }
    if score.disk_profile != Some(DiskProfile::B1) {
        return Err(BaselineError("only a B1 run calibrates the scale".into()));
    }
    let best = |f: fn(&super::file::KernelRate) -> Option<f64>| {
        B1_TESTS
            .iter()
            .filter_map(|t| {
                let k = score.kernels.iter().find(|k| k.id == t.id)?;
                Some((t.id, round4(f(k)?)))
            })
            .collect()
    };
    checked(DiskBaseline {
        version: DISK_SCORE_VERSION.into(),
        provisional: false,
        read: best(|k| k.read.as_ref().map(|r| r.mbs)),
        write: best(|k| k.write.as_ref().map(|r| r.mbs)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scores::{Device, DiskRate, KernelRate, Scores, FORMAT};
    use oma_ipc::load::{LoadMessage, RunRequest};

    fn plan_of(profile: DiskProfile) -> (Plan, Vec<BenchStep>) {
        disk_bench_plan(r"C:\Temp\oma".into(), 500 * GIB, false, profile, 7)
    }

    fn job_of(p: &Phase) -> &DiskJob {
        p.disk.as_ref().unwrap()
    }

    #[test]
    fn b1_plan_has_the_fill_then_reads_then_writes() {
        let (plan, steps) = plan_of(DiskProfile::B1);
        assert_eq!((plan.phases.len(), steps.len()), (33, 33));
        assert_eq!(plan.seed, 7);
        assert_eq!(plan.phases[0].kernel, KernelId::DiskFill);
        assert_eq!(plan.phases[0].duration_s, 900);
        let fill = job_of(&plan.phases[0]);
        assert_eq!(
            (
                fill.block_bytes,
                fill.queue,
                fill.threads,
                fill.random_percent
            ),
            (1 << 20, 4, 1, 0)
        );
        assert_eq!(
            steps[0],
            BenchStep {
                kernel: BenchKernel::DiskFill,
                mode: BenchMode::Write,
                rep: 0
            }
        );
        assert_eq!(
            steps[1],
            BenchStep {
                kernel: BenchKernel::Seq1mQ8t1,
                mode: BenchMode::Read,
                rep: 0
            }
        );
        let j = job_of(&plan.phases[1]);
        assert_eq!(
            (j.block_bytes, j.queue, j.threads, j.read_percent),
            (1 << 20, 8, 1, 100)
        );
        assert!(steps[1..17].iter().all(|s| s.mode == BenchMode::Read));
        assert!(steps[17..].iter().all(|s| s.mode == BenchMode::Write));
        assert_eq!(
            (steps[17].kernel, steps[17].rep),
            (BenchKernel::Seq1mQ8t1, 0)
        );
        assert_eq!(job_of(&plan.phases[17]).read_percent, 0);
        assert_eq!(steps[5].kernel, BenchKernel::Seq1mQ1t1);
        assert_eq!(steps[13].kernel, BenchKernel::Rnd4kQ1t1);
        let rnd = job_of(&plan.phases[9]);
        assert_eq!(
            (rnd.block_bytes, rnd.random_percent, rnd.queue),
            (4096, 100, 32)
        );
        let t = plan.disk.as_ref().unwrap();
        assert_eq!(
            (t.dir.as_str(), t.file_bytes, t.compressible),
            (r"C:\Temp\oma", DISK_BENCH_FILE, false)
        );
        assert_eq!(t.reserve_bytes, disk_reserve(500 * GIB));
    }

    #[test]
    fn bench_write_caps_follow_the_table() {
        for profile in [DiskProfile::B1, DiskProfile::B2] {
            let (plan, steps) = plan_of(profile);
            let total: u64 = plan
                .phases
                .iter()
                .map(|p| job_of(p).write_cap_bytes.unwrap_or(0))
                .sum();
            assert_eq!(total, 40 * GIB);
            for (p, s) in plan.phases.iter().zip(&steps).skip(1) {
                let cap = job_of(p).write_cap_bytes;
                if s.mode == BenchMode::Read {
                    assert_eq!(cap, None);
                } else if s.rep == 0 {
                    assert_eq!(cap, Some(GIB));
                } else if job_of(p).random_percent == 0 {
                    assert_eq!(cap, Some(4 * GIB));
                } else {
                    assert_eq!(cap, Some(2 * GIB));
                }
            }
        }
    }

    #[test]
    fn every_bench_phase_waits_5_s_and_lasts_5_s() {
        let (plan, _) = plan_of(DiskProfile::B1);
        for p in &plan.phases[1..] {
            assert_eq!(p.kernel, KernelId::DiskBench);
            assert_eq!((p.duration_s, p.pause_before_ms), (5, 5000));
        }
    }

    #[test]
    fn disk_bench_plan_validates() {
        for profile in [DiskProfile::B1, DiskProfile::B2] {
            let (plan, _) = plan_of(profile);
            LoadMessage::Run(RunRequest { plan }).validate().unwrap();
        }
    }

    #[test]
    fn b2_uses_the_nvme_tests() {
        let (plan, steps) = plan_of(DiskProfile::B2);
        assert!(steps[1..5]
            .iter()
            .all(|s| s.kernel == BenchKernel::Seq1mQ8t1));
        assert_eq!(steps[5].kernel, BenchKernel::Seq128kQ32t1);
        assert_eq!(steps[9].kernel, BenchKernel::Rnd4kQ32t16);
        let j = job_of(&plan.phases[9]);
        assert_eq!((j.queue, j.threads, j.block_bytes), (32, 16, 4096));
        let j = job_of(&plan.phases[5]);
        assert_eq!(
            (j.block_bytes, j.seq_block_bytes, j.random_percent),
            (131_072, 131_072, 0)
        );
    }

    #[test]
    fn disk_points_at_the_baseline_are_1000() {
        let b = disk_baseline();
        assert_eq!(disk_points(&b.read, &b.write, b), Some(1000));
    }

    #[test]
    fn doubling_every_rate_doubles_the_points() {
        let b = disk_baseline();
        let twice = |m: &BTreeMap<BenchKernel, f64>| m.iter().map(|(k, v)| (*k, v * 2.0)).collect();
        assert_eq!(
            disk_points(&twice(&b.read), &twice(&b.write), b),
            Some(2000)
        );
    }

    #[test]
    fn b2_and_partial_runs_have_no_points() {
        let b = disk_baseline();
        let b2: BTreeMap<_, _> = B2_TESTS.iter().map(|t| (t.id, 1000.0)).collect();
        assert_eq!(disk_points(&b2, &b2, b), None);
        let mut partial = b.read.clone();
        partial.remove(&BenchKernel::Rnd4kQ1t1);
        assert_eq!(disk_points(&partial, &b.write, b), None);
        partial.insert(BenchKernel::Rnd4kQ1t1, f64::NAN);
        assert_eq!(disk_points(&partial, &b.write, b), None);
    }

    #[test]
    fn disk_baseline_parses_with_four_tests_per_direction() {
        let b = disk_baseline();
        assert_eq!((b.version.as_str(), b.provisional), ("disk-1", false));
        assert_eq!((b.read.len(), b.write.len()), (4, 4));
        assert_eq!(b.read[&BenchKernel::Seq1mQ8t1], 7342.0);
        assert_eq!(b.write[&BenchKernel::Rnd4kQ1t1], 223.9);
        let other = DISK_BASELINE_JSON.replace("\"disk-1\"", "\"disk-2\"");
        assert!(parse_disk_baseline(&other).is_err());
        let missing = DISK_BASELINE_JSON.replace("rnd4k_q1t1", "seq128k_q32t1");
        assert!(parse_disk_baseline(&missing).is_err());
    }

    fn calibration_score() -> ScoreFile {
        let b = disk_baseline();
        let rate = |mbs: f64| {
            Some(DiskRate {
                mbs,
                iops: 1000.0,
                mean_lat_us: 100.0,
                p99_lat_us: 300.0,
            })
        };
        ScoreFile {
            format: FORMAT,
            id: "0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d".into(),
            at: "2026-10-08T10:00:00Z".into(),
            category: "disk".into(),
            score_version: DISK_SCORE_VERSION.into(),
            provisional: true,
            isa: None,
            shader_digest: None,
            scores: Scores {
                read_mbs: Some(7000.0),
                write_mbs: Some(6000.0),
                points: Some(1000),
                ..Scores::default()
            },
            kernels: B1_TESTS
                .iter()
                .map(|t| KernelRate {
                    id: t.id,
                    unit: "MB/s".into(),
                    single: None,
                    multi: None,
                    value: None,
                    spread: None,
                    read: rate(b.read[&t.id] * 1.000_04),
                    write: rate(b.write[&t.id] * 1.000_04),
                })
                .collect(),
            device: Device {
                model: "Samsung SSD".into(),
                device_id: Some("disk-0".into()),
                kind: Some("nvme".into()),
                ..Device::default()
            },
            flags: vec![],
            valid: true,
            scaling: None,
            samples: vec![],
            app_version: "0.6.0".into(),
            load_version: Some("0.6.0".into()),
            disk_profile: Some(DiskProfile::B1),
        }
    }

    #[test]
    fn disk_calibration_takes_the_best_rates_of_a_clean_b1_run() {
        let c = disk_calibration_from(&calibration_score()).unwrap();
        assert_eq!((c.version.as_str(), c.provisional), ("disk-1", false));
        let b = disk_baseline();
        assert_eq!(c.read, b.read);
        assert_eq!(c.write, b.write);
    }

    #[test]
    fn disk_calibration_refuses_b2_flagged_and_partial_runs() {
        let mut s = calibration_score();
        s.disk_profile = Some(DiskProfile::B2);
        assert!(disk_calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.disk_profile = None;
        assert!(disk_calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.valid = false;
        assert!(disk_calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.flags = vec!["throttling".into()];
        assert!(disk_calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.score_version = "cpu-1".into();
        assert!(disk_calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.kernels[2].write = None;
        assert!(disk_calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.kernels.remove(1);
        assert!(disk_calibration_from(&s).is_err());
    }
}
