//! The benchmark plan (DB4, DB5): 6 workloads x (single, multi) x (warm-up + 3 reps).

use oma_ipc::load::{DataSize, Isa, LoadMode, Phase, Placement, Plan, Topology};
use serde::Serialize;

use super::workloads::{BenchKernel, WORKLOADS};
use crate::load::core_order;

/// Per-phase cap in seconds (DB5).
pub const CAP_S: u32 = 30;
/// Pause before each warm-up phase.
pub const WARMUP_PAUSE_MS: u32 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchMode {
    Single,
    Multi,
    // GPU benchmark groups.
    Compute,
    Graphics,
}

/// `rep` 0 is the warm-up, 1-3 the repetitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BenchStep {
    pub kernel: BenchKernel,
    pub mode: BenchMode,
    pub rep: u8,
}

pub fn bench_plan(topology: &Topology, isa: Isa, seed: u64) -> (Plan, Vec<BenchStep>) {
    let core = core_order(topology).first().copied().unwrap_or(0);
    let mut phases = Vec::with_capacity(48);
    let mut steps = Vec::with_capacity(48);
    for mode in [BenchMode::Single, BenchMode::Multi] {
        for w in &WORKLOADS {
            for rep in 0..=3u8 {
                phases.push(Phase {
                    kernel: w.kernel,
                    alt_kernel: None,
                    isa,
                    size: DataSize::Fixed,
                    mode: LoadMode::Steady,
                    placement: match mode {
                        BenchMode::Single => Placement::OnePerCore,
                        BenchMode::Multi | BenchMode::Compute | BenchMode::Graphics => {
                            Placement::AllLogical
                        }
                    },
                    duration_s: CAP_S,
                    per_core_s: None,
                    both_smt: false,
                    cores: (mode == BenchMode::Single).then(|| vec![core]),
                    patterns: vec![],
                    stop_on_error: true,
                    iterations: Some(w.iterations),
                    pause_before_ms: if rep == 0 { WARMUP_PAUSE_MS } else { 0 },
                    windows: None,

                    disk: None,
                });
                steps.push(BenchStep {
                    kernel: w.id,
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

        disk: None,
    };
    (plan, steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::load::{CacheSizes, LoadMessage, LogicalCpu, RunRequest};

    fn topo(cores: u32, smt: bool, hybrid: bool) -> Topology {
        let per = if smt { 2 } else { 1 };
        let mut logical = Vec::new();
        for core in 0..cores {
            for t in 0..per {
                logical.push(LogicalCpu {
                    index: core * per + t,
                    group: 0,
                    number: (core * per + t) as u8,
                    core,
                    core_index: core,
                    efficiency_class: if hybrid && core < 2 { 0 } else { 1 },
                    llc: 0,
                    parked: false,
                    apic_id: None,
                });
            }
        }
        Topology {
            logical,
            caches: CacheSizes {
                l1d_bytes: 32768,
                l2_bytes: 1 << 20,
                l2_shared_by: 1,
                l3_bytes: 32 << 20,
                l3_total_bytes: 32 << 20,
            },
            hypervisor: false,
            vendor: "V".into(),
            brand: "B".into(),
        }
    }

    #[test]
    fn bench_plan_has_48_phases_single_then_multi() {
        let (plan, steps) = bench_plan(&topo(8, true, false), Isa::Avx2, 7);
        assert_eq!(plan.phases.len(), 48);
        assert_eq!(steps.len(), 48);
        assert_eq!(plan.ram_bytes, 0);
        assert_eq!(plan.seed, 7);
        assert!(steps[..24].iter().all(|s| s.mode == BenchMode::Single));
        assert!(steps[24..].iter().all(|s| s.mode == BenchMode::Multi));
        assert_eq!((steps[0].kernel, steps[0].rep), (BenchKernel::Ntt, 0));
        assert_eq!((steps[3].kernel, steps[3].rep), (BenchKernel::Ntt, 3));
        assert_eq!((steps[4].kernel, steps[4].rep), (BenchKernel::Hash, 0));
        assert_eq!(steps[24].kernel, BenchKernel::Ntt);
        for (p, s) in plan.phases.iter().zip(&steps) {
            let w = WORKLOADS.iter().find(|w| w.id == s.kernel).unwrap();
            assert_eq!(p.kernel, w.kernel);
            assert_eq!(p.iterations, Some(w.iterations));
            assert_eq!(p.size, DataSize::Fixed);
            assert_eq!(p.mode, LoadMode::Steady);
            assert_eq!(p.isa, Isa::Avx2);
            assert_eq!(p.duration_s, 30);
            assert!(p.stop_on_error);
            assert!(p.patterns.is_empty());
        }
        assert!(plan.phases[24..]
            .iter()
            .all(|p| p.placement == Placement::AllLogical && p.cores.is_none()));
    }

    #[test]
    fn single_phases_pin_the_first_core_of_the_best_class() {
        let (plan, _) = bench_plan(&topo(8, true, true), Isa::Avx512, 1);
        // Cores 0 and 1 are efficiency class 0: the first core of class 1 is core 2.
        for p in &plan.phases[..24] {
            assert_eq!(p.placement, Placement::OnePerCore);
            assert_eq!(p.cores, Some(vec![2]));
        }
    }

    #[test]
    fn each_warmup_pauses_two_seconds() {
        let (plan, steps) = bench_plan(&topo(4, false, false), Isa::Sse2, 1);
        for (p, s) in plan.phases.iter().zip(&steps) {
            assert_eq!(p.pause_before_ms, if s.rep == 0 { 2000 } else { 0 });
        }
        assert_eq!(steps.iter().filter(|s| s.rep == 0).count(), 12);
    }

    #[test]
    fn bench_plan_on_a_single_logical_cpu() {
        let (plan, _) = bench_plan(&topo(1, false, false), Isa::Sse2, 1);
        assert_eq!(plan.phases.len(), 48);
        assert_eq!(plan.phases[0].cores, Some(vec![0]));
    }

    #[test]
    fn bench_plan_validates() {
        let (plan, _) = bench_plan(&topo(8, true, true), Isa::Avx512, 1);
        LoadMessage::Run(RunRequest { plan }).validate().unwrap();
    }
}
