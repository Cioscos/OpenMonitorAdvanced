//! Fixed-scale score (DB1, DB6): `round(1500 x geometric mean(rate / reference))`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use oma_ipc::load::WorkerDone;
use serde::{Deserialize, Serialize};

use super::file::ScoreFile;
use super::plan::BenchMode;
use super::workloads::{BenchKernel, Workload, WORKLOADS};

pub const SCALE_POINTS: f64 = 1500.0;
pub const SCORE_VERSION: &str = "cpu-1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub version: String,
    pub provisional: bool,
    /// True units (`Workload::unit`).
    pub single: BTreeMap<BenchKernel, f64>,
    pub multi: BTreeMap<BenchKernel, f64>,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid baseline: {0}")]
pub struct BaselineError(pub(super) String);

impl Baseline {
    /// The reference table of a CPU mode; `None` for the GPU groups.
    pub(super) fn table(&self, mode: BenchMode) -> Option<&BTreeMap<BenchKernel, f64>> {
        match mode {
            BenchMode::Single => Some(&self.single),
            BenchMode::Multi => Some(&self.multi),
            BenchMode::Compute | BenchMode::Graphics => None,
        }
    }
}

/// 4 significant digits, as the calibrated baselines store them.
pub(super) fn round4(x: f64) -> f64 {
    format!("{x:.3e}").parse().unwrap_or(x)
}

const BASELINE_JSON: &str = include_str!("cpu-1-baseline.json");

fn checked(b: Baseline) -> Result<Baseline, BaselineError> {
    let complete = |m: &BTreeMap<BenchKernel, f64>| {
        WORKLOADS
            .iter()
            .all(|w| m.get(&w.id).is_some_and(|v| v.is_finite() && *v > 0.0))
    };
    if b.version != SCORE_VERSION {
        Err(BaselineError(format!(
            "version {} is not {SCORE_VERSION}",
            b.version
        )))
    } else if !complete(&b.single) || !complete(&b.multi) {
        Err(BaselineError("a kernel is missing or not positive".into()))
    } else {
        Ok(b)
    }
}

fn parse_baseline(text: &str) -> Result<Baseline, BaselineError> {
    checked(serde_json::from_str(text).map_err(|e| BaselineError(e.to_string()))?)
}

/// The baseline calibrated from a score file (DB1): its medians to 4 significant digits,
/// `provisional: false`. Refuses a run that is invalid, flagged, of another score version
/// or without all six kernels in both modes.
pub fn calibration_from(score: &ScoreFile) -> Result<Baseline, BaselineError> {
    if !score.valid {
        return Err(BaselineError("the run is not valid".into()));
    }
    if !score.flags.is_empty() {
        return Err(BaselineError(format!(
            "the run has flags: {}",
            score.flags.join(", ")
        )));
    }
    if score.score_version != SCORE_VERSION {
        return Err(BaselineError(format!(
            "score version {} is not {SCORE_VERSION}",
            score.score_version
        )));
    }
    let medians = |f: fn(&super::file::KernelRate) -> Option<f64>| {
        score
            .kernels
            .iter()
            .filter_map(|k| f(k).map(|v| (k.id, round4(v))))
            .collect()
    };
    checked(Baseline {
        version: SCORE_VERSION.into(),
        provisional: false,
        single: medians(|k| k.single),
        multi: medians(|k| k.multi),
    })
}

/// The reference rates of the fixed scale (compiled in).
pub fn cpu_baseline() -> &'static Baseline {
    static BASELINE: OnceLock<Baseline> = OnceLock::new();
    BASELINE.get_or_init(|| parse_baseline(BASELINE_JSON).expect("cpu-1-baseline.json is valid"))
}

/// Iterations per second (all threads) to the workload's true unit.
pub fn per_second_to_units(w: &Workload, iterations_per_s: f64) -> f64 {
    let divisor = if w.unit.starts_with('G') { 1e9 } else { 1e6 };
    iterations_per_s * w.work_per_iteration / divisor
}

/// Rate of one repetition in true units: `checks` iterations of all threads in `work_ms`.
/// `None` when either is 0.
pub fn rate(w: &Workload, checks: u64, work_ms: u64) -> Option<f64> {
    if checks == 0 || work_ms == 0 {
        return None;
    }
    let r = per_second_to_units(w, checks as f64 / (work_ms as f64 / 1000.0));
    (r.is_finite() && r > 0.0).then_some(r)
}

/// Rate of a fixed-work repetition in true units: the sum of the per-thread rates
/// (iterations x 1000 / work_ms). Threads with no time or no iterations are skipped;
/// `None` when none is left (DB12).
pub fn rate_from_workers(w: &Workload, workers: &[WorkerDone]) -> Option<f64> {
    let sum: f64 = workers
        .iter()
        .filter(|t| t.work_ms > 0 && t.iterations > 0)
        .map(|t| t.iterations as f64 * 1000.0 / t.work_ms as f64)
        .sum();
    let r = per_second_to_units(w, sum);
    (r.is_finite() && r > 0.0).then_some(r)
}

/// Median of the valid repetitions (1-3): the middle one, or the mean of the two.
pub fn median3(v: &[f64]) -> Option<f64> {
    if !(1..=3).contains(&v.len()) || v.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    Some(match s.len() {
        2 => (s[0] + s[1]) / 2.0,
        n => s[n / 2],
    })
}

/// Geometric mean of the values; `None` if there is none or one is missing or not
/// positive.
pub(super) fn geomean(values: impl IntoIterator<Item = Option<f64>>) -> Option<f64> {
    let (mut sum, mut n) = (0.0, 0u32);
    for x in values {
        let x = x?;
        if !x.is_finite() || x <= 0.0 {
            return None;
        }
        sum += x.ln();
        n += 1;
    }
    (n > 0).then(|| (sum / f64::from(n)).exp())
}

/// `round(1500 x g)`, `None` when it does not fit.
pub(super) fn scale_points(g: f64) -> Option<u32> {
    let p = (SCALE_POINTS * g).round();
    (p.is_finite() && p <= f64::from(u32::MAX)).then_some(p as u32)
}

pub fn points(
    rates: &BTreeMap<BenchKernel, f64>,
    reference: &BTreeMap<BenchKernel, f64>,
) -> Option<u32> {
    scale_points(geomean(
        WORKLOADS
            .iter()
            .map(|w| Some(*rates.get(&w.id)? / *reference.get(&w.id)?)),
    )?)
}

pub fn scaling(
    single: &BTreeMap<BenchKernel, f64>,
    multi: &BTreeMap<BenchKernel, f64>,
    logical: u32,
) -> Option<f64> {
    if logical == 0 {
        return None;
    }
    geomean(
        WORKLOADS
            .iter()
            .map(|w| Some(*multi.get(&w.id)? / (*single.get(&w.id)? * f64::from(logical)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(f: impl Fn(&Workload) -> f64) -> BTreeMap<BenchKernel, f64> {
        WORKLOADS.iter().map(|w| (w.id, f(w))).collect()
    }

    #[test]
    fn rate_converts_to_true_units() {
        let ntt = &WORKLOADS[0];
        // 900 iterations of 491 520 butterflies in 1 s = 442.368 Mop/s.
        let r = rate(ntt, 900, 1000).unwrap();
        assert!((r - 442.368).abs() < 1e-9, "{r}");
        let gemm = &WORKLOADS[5];
        // 640 x 33 554 432 FLOP in 1 s = 21.47483648 GFLOP/s.
        let r = rate(gemm, 640, 1000).unwrap();
        assert!((r - 21.474_836_48).abs() < 1e-9, "{r}");
    }

    fn wd(iterations: u64, work_ms: u64) -> WorkerDone {
        WorkerDone {
            logical: 0,
            iterations,
            work_ms,
        }
    }

    #[test]
    fn multi_rate_sums_thread_rates() {
        let ntt = &WORKLOADS[0];
        let mut t: Vec<_> = (0..8).map(|_| wd(1000, 1000)).collect();
        t.extend((0..16).map(|_| wd(1000, 2000)));
        // 8 x 1000 + 16 x 500 = 16 000 it/s.
        let want = per_second_to_units(ntt, 16_000.0);
        assert!((rate_from_workers(ntt, &t).unwrap() - want).abs() < 1e-9);
    }

    #[test]
    fn single_worker_rate_equals_checks_over_work_ms() {
        let ntt = &WORKLOADS[0];
        assert_eq!(
            rate_from_workers(ntt, &[wd(900, 1000)]),
            rate(ntt, 900, 1000)
        );
    }

    #[test]
    fn zero_time_workers_are_ignored() {
        let ntt = &WORKLOADS[0];
        let t = [wd(10, 0), wd(0, 100), wd(900, 1000)];
        assert_eq!(rate_from_workers(ntt, &t), rate(ntt, 900, 1000));
        assert_eq!(rate_from_workers(ntt, &[wd(10, 0), wd(0, 5)]), None);
        assert_eq!(rate_from_workers(ntt, &[]), None);
    }

    #[test]
    fn zero_work_ms_rep_is_ignored() {
        assert_eq!(rate(&WORKLOADS[0], 10, 0), None);
        assert_eq!(rate(&WORKLOADS[0], 0, 100), None);
    }

    #[test]
    fn median_of_three_and_of_two() {
        assert_eq!(median3(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median3(&[4.0, 2.0]), Some(3.0));
        assert_eq!(median3(&[5.0]), Some(5.0));
        assert_eq!(median3(&[]), None);
        assert_eq!(median3(&[1.0, 2.0, 3.0, 4.0]), None);
        assert_eq!(median3(&[1.0, f64::NAN]), None);
    }

    #[test]
    fn points_at_the_baseline_are_1500() {
        let b = cpu_baseline();
        assert_eq!(points(&b.single, &b.single), Some(1500));
        assert_eq!(points(&b.multi, &b.multi), Some(1500));
    }

    #[test]
    fn doubling_every_rate_doubles_the_points() {
        let b = cpu_baseline();
        let doubled: BTreeMap<_, _> = b.single.iter().map(|(k, v)| (*k, v * 2.0)).collect();
        assert_eq!(points(&doubled, &b.single), Some(3000));
    }

    #[test]
    fn score_without_all_six_kernels_is_none() {
        let b = cpu_baseline();
        let mut r = b.single.clone();
        r.remove(&BenchKernel::Sort);
        assert_eq!(points(&r, &b.single), None);
        r.insert(BenchKernel::Sort, 0.0);
        assert_eq!(points(&r, &b.single), None);
        r.insert(BenchKernel::Sort, f64::INFINITY);
        assert_eq!(points(&r, &b.single), None);
    }

    #[test]
    fn scaling_with_one_logical_is_one() {
        let s = all(|w| w.iterations as f64);
        assert_eq!(scaling(&s, &s, 1), Some(1.0));
        assert_eq!(scaling(&s, &s, 0), None);
        let m: BTreeMap<_, _> = s.iter().map(|(k, v)| (*k, v * 6.0)).collect();
        let x = scaling(&s, &m, 8).unwrap();
        assert!((x - 0.75).abs() < 1e-12, "{x}");
    }

    #[test]
    fn baseline_of_another_score_version_is_rejected() {
        let text = BASELINE_JSON.replace("\"cpu-1\"", "\"cpu-2\"");
        assert_ne!(text, BASELINE_JSON);
        assert!(parse_baseline(&text).is_err());
    }

    fn calibration_score() -> ScoreFile {
        use super::super::file::{Device, KernelRate, Scores, FORMAT};
        let b = cpu_baseline();
        ScoreFile {
            format: FORMAT,
            id: "0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d".into(),
            at: "2026-10-07T10:00:00Z".into(),
            category: "cpu".into(),
            score_version: SCORE_VERSION.into(),
            provisional: true,
            isa: Some(oma_ipc::load::Isa::Avx512),
            shader_digest: None,
            scores: Scores {
                single: Some(1500),
                multi: Some(1500),
                compute: None,
                graphics: None,
            },
            kernels: WORKLOADS
                .iter()
                .map(|w| KernelRate {
                    id: w.id,
                    unit: w.unit.into(),
                    single: Some(b.single[&w.id] * 1.000_04),
                    multi: Some(b.multi[&w.id]),
                    value: None,
                    spread: None,
                })
                .collect(),
            device: Device {
                model: "AMD Ryzen 7 7800X3D".into(),
                cores: 8,
                logical: 16,
                ..Device::default()
            },
            flags: vec![],
            valid: true,
            scaling: Some(0.6),
            samples: vec![],
            app_version: "0.5.0".into(),
            load_version: None,
        }
    }

    #[test]
    fn calibration_takes_the_medians_of_a_clean_valid_run() {
        let s = calibration_score();
        let c = calibration_from(&s).unwrap();
        assert_eq!((c.version.as_str(), c.provisional), (SCORE_VERSION, false));
        let ntt = s.kernels[0].single.unwrap();
        assert_eq!(
            c.single[&BenchKernel::Ntt],
            format!("{ntt:.3e}").parse::<f64>().unwrap()
        );
        assert_eq!((c.single.len(), c.multi.len()), (6, 6));
    }

    #[test]
    fn calibration_refuses_an_invalid_flagged_foreign_or_partial_run() {
        let mut s = calibration_score();
        s.valid = false;
        assert!(calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.flags = vec!["battery".into()];
        assert!(calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.score_version = "cpu-0".into();
        assert!(calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.kernels[3].multi = None;
        assert!(calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.kernels.pop();
        assert!(calibration_from(&s).is_err());
    }

    #[test]
    fn baseline_parses_and_has_six_kernels_per_mode() {
        let b = cpu_baseline();
        assert_eq!(b.version, "cpu-1");
        assert!(!b.provisional);
        assert_eq!(b.single[&BenchKernel::Gemm], 39.55);
        assert_eq!((b.single.len(), b.multi.len()), (6, 6));
        assert!(b.single.values().chain(b.multi.values()).all(|v| *v > 0.0));
    }
}
