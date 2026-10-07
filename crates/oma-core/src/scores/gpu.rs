//! GPU benchmark (DH1, DH2, DH6, DH7): the six loads, the plan, the fixed-scale reference
//! rates and points, and the median and spread of the measured windows.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use oma_ipc::load::{DataSize, GpuTarget, Isa, KernelId, LoadMode, Phase, Placement, Plan};
use serde::{Deserialize, Serialize};

use super::file::ScoreFile;
use super::plan::{BenchMode, BenchStep};
use super::score::{geomean, round4, scale_points, BaselineError};
use super::workloads::BenchKernel;

/// Measured 1 s windows per load (DH6).
pub const GPU_WINDOWS: u8 = 5;
/// Per-phase cap in seconds, warm-up included (DH6).
pub const GPU_CAP_S: u32 = 30;
pub const GPU_SCORE_VERSION: &str = "gpu-1";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuLoad {
    pub id: BenchKernel,
    pub kernel: KernelId,
    pub mode: BenchMode,
    /// Display unit of the score file and of the baseline.
    pub unit: &'static str,
    /// Base units (FLOP, operations, bytes, pixels, texels) per display unit:
    /// `PhaseDone.rates` and `Progress.rate` divided by it give `unit`.
    pub per_unit: f64,
}

/// In the order of the plan (DH6): the three Compute loads, then the three Graphics ones.
pub const GPU_LOADS: [GpuLoad; 6] = [
    GpuLoad {
        id: BenchKernel::Fma,
        kernel: KernelId::S1,
        mode: BenchMode::Compute,
        unit: "TFLOPS",
        per_unit: 1e12,
    },
    GpuLoad {
        id: BenchKernel::IntHash,
        kernel: KernelId::S2,
        mode: BenchMode::Compute,
        unit: "TIOPS",
        per_unit: 1e12,
    },
    GpuLoad {
        id: BenchKernel::Bandwidth,
        kernel: KernelId::S3,
        mode: BenchMode::Compute,
        unit: "GB/s",
        per_unit: 1e9,
    },
    GpuLoad {
        id: BenchKernel::Fill,
        kernel: KernelId::Fill,
        mode: BenchMode::Graphics,
        unit: "Gpixel/s",
        per_unit: 1e9,
    },
    GpuLoad {
        id: BenchKernel::Texture,
        kernel: KernelId::Texture,
        mode: BenchMode::Graphics,
        unit: "Gtexel/s",
        per_unit: 1e9,
    },
    GpuLoad {
        id: BenchKernel::Overdraw,
        kernel: KernelId::Overdraw,
        mode: BenchMode::Graphics,
        unit: "Gpixel/s",
        per_unit: 1e9,
    },
];

/// Six phases, one per load, with the neutral CPU fields of DG9.
pub fn gpu_bench_plan(target: GpuTarget, seed: u64) -> (Plan, Vec<BenchStep>) {
    let phases = GPU_LOADS
        .iter()
        .map(|l| Phase {
            kernel: l.kernel,
            alt_kernel: None,
            isa: Isa::Sse2,
            size: DataSize::Auto,
            mode: LoadMode::Steady,
            placement: Placement::AllLogical,
            duration_s: GPU_CAP_S,
            per_core_s: None,
            both_smt: false,
            cores: None,
            patterns: vec![],
            stop_on_error: true,
            iterations: None,
            pause_before_ms: 0,
            windows: Some(GPU_WINDOWS),
        })
        .collect();
    let steps = GPU_LOADS
        .iter()
        .map(|l| BenchStep {
            kernel: l.id,
            mode: l.mode,
            rep: 1,
        })
        .collect();
    let plan = Plan {
        seed,
        ram_bytes: 0,
        phases,
        gpu: Some(target),
    };
    (plan, steps)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuBaseline {
    pub version: String,
    pub provisional: bool,
    /// Display units (`GpuLoad::unit`), like the score file's `value`.
    pub compute: BTreeMap<BenchKernel, f64>,
    pub graphics: BTreeMap<BenchKernel, f64>,
}

impl GpuBaseline {
    pub(super) fn table(&self, mode: BenchMode) -> Option<&BTreeMap<BenchKernel, f64>> {
        match mode {
            BenchMode::Compute => Some(&self.compute),
            BenchMode::Graphics => Some(&self.graphics),
            BenchMode::Single | BenchMode::Multi => None,
        }
    }
}

const GPU_BASELINE_JSON: &str = include_str!("gpu-1-baseline.json");

fn loads(mode: BenchMode) -> impl Iterator<Item = &'static GpuLoad> {
    GPU_LOADS.iter().filter(move |l| l.mode == mode)
}

fn checked(b: GpuBaseline) -> Result<GpuBaseline, BaselineError> {
    let complete = |mode| {
        let table = b.table(mode).expect("a GPU group");
        loads(mode).all(|l| table.get(&l.id).is_some_and(|v| v.is_finite() && *v > 0.0))
    };
    if b.version != GPU_SCORE_VERSION {
        Err(BaselineError(format!(
            "version {} is not {GPU_SCORE_VERSION}",
            b.version
        )))
    } else if !complete(BenchMode::Compute) || !complete(BenchMode::Graphics) {
        Err(BaselineError("a load is missing or not positive".into()))
    } else {
        Ok(b)
    }
}

fn parse_gpu_baseline(text: &str) -> Result<GpuBaseline, BaselineError> {
    checked(serde_json::from_str(text).map_err(|e| BaselineError(e.to_string()))?)
}

/// The reference rates of the GPU fixed scale (compiled in).
pub fn gpu_baseline() -> &'static GpuBaseline {
    static BASELINE: OnceLock<GpuBaseline> = OnceLock::new();
    BASELINE.get_or_init(|| {
        parse_gpu_baseline(GPU_BASELINE_JSON).expect("gpu-1-baseline.json is valid")
    })
}

/// Median of the windows and spread `(max - min) / median`, over the finite values > 0
/// only (DH7); `None` when none is left.
pub fn median_spread(rates: &[f64]) -> Option<(f64, f64)> {
    let mut v: Vec<f64> = rates
        .iter()
        .copied()
        .filter(|x| x.is_finite() && *x > 0.0)
        .collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let median = if n % 2 == 0 {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    } else {
        v[n / 2]
    };
    Some((median, (v[n - 1] - v[0]) / median))
}

/// Points of a group: `round(1500 x geometric mean(rate / reference))` over its three
/// loads, rates in display units; `None` without all three (DH1, DH7).
pub fn gpu_points(
    rates: &BTreeMap<BenchKernel, f64>,
    mode: BenchMode,
    baseline: &GpuBaseline,
) -> Option<u32> {
    let reference = baseline.table(mode)?;
    scale_points(geomean(
        loads(mode).map(|l| Some(*rates.get(&l.id)? / *reference.get(&l.id)?)),
    )?)
}

/// The baseline calibrated from a GPU score file: its medians to 4 significant digits,
/// `provisional: false`. Refuses a run that is invalid, flagged, of another score version
/// or without all six loads.
pub fn gpu_calibration_from(score: &ScoreFile) -> Result<GpuBaseline, BaselineError> {
    if !score.valid {
        return Err(BaselineError("the run is not valid".into()));
    }
    if !score.flags.is_empty() {
        return Err(BaselineError(format!(
            "the run has flags: {}",
            score.flags.join(", ")
        )));
    }
    if score.score_version != GPU_SCORE_VERSION {
        return Err(BaselineError(format!(
            "score version {} is not {GPU_SCORE_VERSION}",
            score.score_version
        )));
    }
    let medians = |mode| {
        loads(mode)
            .filter_map(|l| {
                let k = score.kernels.iter().find(|k| k.id == l.id)?;
                Some((l.id, round4(k.value?)))
            })
            .collect()
    };
    checked(GpuBaseline {
        version: GPU_SCORE_VERSION.into(),
        provisional: false,
        compute: medians(BenchMode::Compute),
        graphics: medians(BenchMode::Graphics),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scores::{Device, KernelRate, Scores, FORMAT};
    use oma_ipc::load::{LoadMessage, RunRequest};

    const TARGET: GpuTarget = GpuTarget {
        luid: 0x17e99,
        integrated: false,
    };

    #[test]
    fn gpu_bench_plan_has_six_phases_compute_then_graphics() {
        let (plan, steps) = gpu_bench_plan(TARGET, 7);
        assert_eq!((plan.phases.len(), steps.len()), (6, 6));
        assert_eq!((plan.seed, plan.ram_bytes, plan.gpu), (7, 0, Some(TARGET)));
        let kernels: Vec<_> = plan.phases.iter().map(|p| p.kernel).collect();
        assert_eq!(
            kernels,
            [
                KernelId::S1,
                KernelId::S2,
                KernelId::S3,
                KernelId::Fill,
                KernelId::Texture,
                KernelId::Overdraw
            ]
        );
        assert!(steps[..3].iter().all(|s| s.mode == BenchMode::Compute));
        assert!(steps[3..].iter().all(|s| s.mode == BenchMode::Graphics));
        let ids: Vec<_> = steps.iter().map(|s| s.kernel).collect();
        assert_eq!(
            ids,
            [
                BenchKernel::Fma,
                BenchKernel::IntHash,
                BenchKernel::Bandwidth,
                BenchKernel::Fill,
                BenchKernel::Texture,
                BenchKernel::Overdraw
            ]
        );
        assert!(steps.iter().all(|s| s.rep == 1));
        for p in &plan.phases {
            assert_eq!(p.windows, Some(5));
            assert_eq!(p.duration_s, 30);
            assert_eq!(p.mode, LoadMode::Steady);
            assert!(p.stop_on_error && p.alt_kernel.is_none());
            assert_eq!(
                (p.isa, p.size, p.placement),
                (Isa::Sse2, DataSize::Auto, Placement::AllLogical)
            );
            assert_eq!(
                (p.iterations, p.pause_before_ms, p.cores.as_ref()),
                (None, 0, None)
            );
        }
    }

    #[test]
    fn gpu_bench_plan_validates() {
        let (plan, _) = gpu_bench_plan(TARGET, 1);
        LoadMessage::Run(RunRequest { plan }).validate().unwrap();
    }

    #[test]
    fn median_spread_of_five_windows() {
        assert_eq!(
            median_spread(&[10.0, 11.0, 9.0, 10.0, 12.0]),
            Some((10.0, 0.3))
        );
        assert_eq!(median_spread(&[4.0, 2.0]), Some((3.0, 2.0 / 3.0)));
    }

    #[test]
    fn median_spread_ignores_zero_and_non_finite() {
        let v = [0.0, f64::NAN, 10.0, f64::INFINITY, -1.0, 10.0, 10.0];
        assert_eq!(median_spread(&v), Some((10.0, 0.0)));
        assert_eq!(median_spread(&[0.0, f64::NAN]), None);
        assert_eq!(median_spread(&[]), None);
    }

    #[test]
    fn gpu_points_at_the_baseline_are_1500() {
        let b = gpu_baseline();
        assert_eq!(gpu_points(&b.compute, BenchMode::Compute, b), Some(1500));
        assert_eq!(gpu_points(&b.graphics, BenchMode::Graphics, b), Some(1500));
        assert_eq!(gpu_points(&b.compute, BenchMode::Single, b), None);
    }

    #[test]
    fn doubling_the_graphics_rates_doubles_the_graphics_points() {
        let b = gpu_baseline();
        let mut r: BTreeMap<_, _> = b.graphics.iter().map(|(k, v)| (*k, v * 2.0)).collect();
        r.extend(b.compute.iter().map(|(k, v)| (*k, *v)));
        assert_eq!(gpu_points(&r, BenchMode::Graphics, b), Some(3000));
        assert_eq!(gpu_points(&r, BenchMode::Compute, b), Some(1500));
    }

    #[test]
    fn group_without_all_three_loads_has_no_points() {
        let b = gpu_baseline();
        let mut r = b.compute.clone();
        r.extend(b.graphics.iter().map(|(k, v)| (*k, *v)));
        r.remove(&BenchKernel::Bandwidth);
        assert_eq!(gpu_points(&r, BenchMode::Compute, b), None);
        assert_eq!(gpu_points(&r, BenchMode::Graphics, b), Some(1500));
        r.insert(BenchKernel::Bandwidth, 0.0);
        assert_eq!(gpu_points(&r, BenchMode::Compute, b), None);
        r.insert(BenchKernel::Bandwidth, f64::NAN);
        assert_eq!(gpu_points(&r, BenchMode::Compute, b), None);
    }

    #[test]
    fn gpu_baseline_parses_and_has_three_loads_per_group() {
        let b = gpu_baseline();
        assert_eq!(b.version, "gpu-1");
        assert!(!b.provisional);
        assert_eq!((b.compute.len(), b.graphics.len()), (3, 3));
        assert!(loads(BenchMode::Compute).all(|l| b.compute[&l.id] > 0.0));
        assert!(loads(BenchMode::Graphics).all(|l| b.graphics[&l.id] > 0.0));
        assert_eq!(b.compute[&BenchKernel::Fma], 46.29);
        assert!(parse_gpu_baseline(&GPU_BASELINE_JSON.replace("\"gpu-1\"", "\"gpu-2\"")).is_err());
        assert!(parse_gpu_baseline(&GPU_BASELINE_JSON.replace("\"fill\"", "\"sort\"")).is_err());
    }

    fn calibration_score() -> ScoreFile {
        let b = gpu_baseline();
        ScoreFile {
            format: FORMAT,
            id: "0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d".into(),
            at: "2026-10-07T10:00:00Z".into(),
            category: "gpu".into(),
            score_version: GPU_SCORE_VERSION.into(),
            provisional: true,
            isa: None,
            shader_digest: Some("0123456789abcdef".into()),
            scores: Scores {
                single: None,
                multi: None,
                compute: Some(1500),
                graphics: Some(1500),
            },
            kernels: GPU_LOADS
                .iter()
                .map(|l| KernelRate {
                    id: l.id,
                    unit: l.unit.into(),
                    single: None,
                    multi: None,
                    value: Some(b.table(l.mode).unwrap()[&l.id] * 1.000_04),
                    spread: Some(0.01),
                })
                .collect(),
            device: Device {
                model: "NVIDIA GeForce RTX 4080".into(),
                cores: 0,
                logical: 0,
                device_id: Some("gpu-pci-0100".into()),
                vendor_id: Some(0x10de),
                dedicated_bytes: Some(16 << 30),
                integrated: Some(false),
            },
            flags: vec![],
            valid: true,
            scaling: None,
            samples: vec![],
            app_version: "0.6.0".into(),
            load_version: Some("0.6.0".into()),
        }
    }

    #[test]
    fn gpu_calibration_takes_the_medians_of_a_clean_valid_run() {
        let s = calibration_score();
        let c = gpu_calibration_from(&s).unwrap();
        assert_eq!((c.version.as_str(), c.provisional), ("gpu-1", false));
        assert_eq!(c.compute[&BenchKernel::Fma], 46.29);
        assert_eq!((c.compute.len(), c.graphics.len()), (3, 3));
    }

    #[test]
    fn gpu_calibration_refuses_an_invalid_flagged_or_partial_run() {
        let mut s = calibration_score();
        s.valid = false;
        assert!(gpu_calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.flags = vec!["busy_gpu".into()];
        assert!(gpu_calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.score_version = "cpu-1".into();
        assert!(gpu_calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.kernels[4].value = None;
        assert!(gpu_calibration_from(&s).is_err());
        let mut s = calibration_score();
        s.kernels.remove(2);
        assert!(gpu_calibration_from(&s).is_err());
    }
}
