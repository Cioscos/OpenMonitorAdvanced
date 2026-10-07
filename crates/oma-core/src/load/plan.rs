//! Profiles (DA11), "Personalizza" (DA12), RAM quota (DA10) and the plan builder.

use oma_ipc::load::{
    DataSize, GpuTarget, Isa, KernelId, LoadMode, Phase, Placement, Plan, RamPattern, Topology,
    MAX_PLAN_SECONDS,
};
use serde::{Deserialize, Serialize};

const MIN_PHASE_S: u32 = 60;
const MIN_RAM_BYTES: u64 = 256 << 20;
/// The memory a RAM share always leaves to Windows (DA10).
pub const KEEP_FREE_BYTES: u64 = 2 << 30;
const RETRY_S: u32 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Component {
    Cpu,
    Ram,
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Objective {
    Normal,
    Overclock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preset {
    Quick,
    Standard,
    Long,
    Night,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadChoice {
    AllLogical,
    OnePerCore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeEdit {
    pub kernel: KernelId,
    pub enabled: bool,
    pub minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Custom {
    pub modes: Vec<ModeEdit>,
    pub isa: Option<Isa>,
    pub threads: ThreadChoice,
    /// In the `core_cycle` phases, both threads of the core.
    pub both_smt: bool,
    pub stop_on_first_error: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryCore {
    pub core: u32,
    pub kernel: KernelId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    pub component: Component,
    pub objective: Objective,
    pub preset: Preset,
    pub custom: Option<Custom>,
    pub retry_core: Option<RetryCore>,
    /// The `device_id` of the GPU to test (DG13); the app resolves the LUID at start.
    #[serde(default)]
    pub gpu: Option<String>,
}

pub struct BuildInput<'a> {
    pub request: &'a StartRequest,
    pub topology: &'a Topology,
    /// The instruction sets the machine can run.
    pub isa: &'a [Isa],
    pub ram_budget: u64,
    /// `performance.stopOnFirstError`.
    pub stop_override: Option<bool>,
    pub seed: u64,
    /// The resolved GPU for `Component::Gpu`.
    pub gpu: Option<GpuTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    #[error("no usable cores")]
    NoCores,
    #[error("the plan has no phases")]
    NoPhases,
    #[error("the plan lasts more than 24 hours")]
    TooLong,
    #[error("unknown core {0}")]
    UnknownCore(u32),
    #[error("the RAM share is below 256 MiB")]
    RamBudget,
    #[error("no suitable GPU")]
    NoGpu,
}

/// The preset durations in seconds (DA11).
pub fn presets(component: Component, objective: Objective) -> &'static [(Preset, u32)] {
    use Preset::*;
    match (component, objective) {
        (Component::Cpu, Objective::Normal) => &[(Quick, 300), (Standard, 1800), (Long, 3600)],
        (Component::Ram, Objective::Normal) => &[(Quick, 900), (Standard, 1800), (Long, 3600)],
        (Component::Gpu, Objective::Normal) => &[(Quick, 300), (Standard, 900), (Long, 1800)],
        (Component::Gpu, Objective::Overclock) => &[(Standard, 1800), (Long, 3600), (Night, 7200)],
        (_, Objective::Overclock) => &[(Standard, 3600), (Long, 7200), (Night, 28_800)],
    }
}

/// RAM share for the RAM tests (DA10): the percentage of the available memory, always
/// leaving 2 GiB free.
pub fn ram_budget(available: u64, percent: u32) -> u64 {
    let share = (u128::from(available) * u128::from(percent) / 100) as u64;
    share.min(available.saturating_sub(KEEP_FREE_BYTES))
}

/// Physical cores in test order (DA4): higher efficiency class first, then by number.
/// Parked cores are included: `Parked` is only the idle state of the moment, and the hard
/// affinity of the workers wakes them.
pub fn core_order(topology: &Topology) -> Vec<u32> {
    let mut cores: Vec<(u8, u32)> = Vec::new();
    for l in &topology.logical {
        if !cores.iter().any(|&(_, c)| c == l.core) {
            cores.push((l.efficiency_class, l.core));
        }
    }
    cores.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    cores.into_iter().map(|(_, c)| c).collect()
}

/// Splits `total` in proportion to `weights`; the last part takes the rounding remainder.
fn scale(weights: &[u32], total: u32) -> Vec<u32> {
    let sum: u64 = weights.iter().map(|&w| u64::from(w)).sum::<u64>().max(1);
    let mut out: Vec<u32> = weights
        .iter()
        .map(|&w| (u64::from(w) * u64::from(total) / sum) as u32)
        .collect();
    let given: u32 = out.iter().sum();
    if let Some(last) = out.last_mut() {
        *last += total - given;
    }
    out
}

fn phase(kernel: KernelId, isa: Isa, size: DataSize, duration_s: u32) -> Phase {
    Phase {
        kernel,
        alt_kernel: None,
        isa,
        size,
        mode: LoadMode::Steady,
        placement: Placement::AllLogical,
        duration_s,
        per_core_s: None,
        both_smt: false,
        cores: None,
        patterns: if kernel == KernelId::K10 {
            vec![
                RamPattern::MovingInversions,
                RamPattern::Modulo20,
                RamPattern::Random,
                RamPattern::Address,
                RamPattern::CrcCopy,
            ]
        } else {
            vec![]
        },
        stop_on_error: false,
        iterations: None,
        pause_before_ms: 0,
        windows: None,
    }
}

fn cycle(mut p: Phase, cores: &[u32], per_core_s: u32) -> Phase {
    p.placement = Placement::CoreCycle;
    p.per_core_s = Some(per_core_s);
    p.duration_s = per_core_s * cores.len() as u32;
    p.cores = Some(cores.to_vec());
    p
}

fn default_size(kernel: KernelId) -> DataSize {
    match kernel {
        KernelId::K2 | KernelId::K5 => DataSize::L2,
        KernelId::K7 => DataSize::L3,
        KernelId::K3 | KernelId::K10 => DataSize::Ram,
        _ => DataSize::Auto,
    }
}

/// CPU, normal objective: the table of DA11, scaled to the duration.
fn cpu_normal(isa: Isa, duration: u32) -> Vec<Phase> {
    use KernelId::*;
    let steps: &[(KernelId, DataSize, u32)] = if duration <= 300 {
        &[
            (K2, DataSize::L2, 120),
            (K5, DataSize::L2, 90),
            (K8, DataSize::Auto, 90),
        ]
    } else {
        &[
            (K1, DataSize::Auto, 600),
            (K2, DataSize::L2, 360),
            (K7, DataSize::L3, 300),
            (K8, DataSize::Auto, 300),
            (K3, DataSize::Ram, 240),
        ]
    };
    let weights: Vec<u32> = steps.iter().map(|s| s.2).collect();
    steps
        .iter()
        .zip(scale(&weights, duration))
        .map(|(&(k, size, _), d)| phase(k, isa, size, d))
        .collect()
}

fn ram_phases(isa: Isa, objective: Objective, duration: u32) -> Vec<Phase> {
    use KernelId::*;
    let k10 = |patterns: Option<&[RamPattern]>, d| {
        let mut p = phase(K10, isa, DataSize::Ram, d);
        if let Some(pt) = patterns {
            p.patterns = pt.to_vec();
        }
        p
    };
    match objective {
        Objective::Normal => {
            let d = scale(&[70, 30], duration);
            vec![
                k10(
                    Some(&[
                        RamPattern::MovingInversions,
                        RamPattern::Random,
                        RamPattern::CrcCopy,
                    ]),
                    d[0],
                ),
                phase(K3, isa, DataSize::Ram, d[1]),
            ]
        }
        Objective::Overclock => {
            let d = scale(&[60, 25, 15], duration);
            let mut v = vec![
                k10(None, d[0]),
                phase(K3, isa, DataSize::Ram, d[1]),
                phase(K4, isa, DataSize::Auto, d[2]),
            ];
            v.iter_mut().for_each(|p| p.stop_on_error = true);
            v
        }
    }
}

/// GPU profiles (DG9). The phase fields the GPU engine ignores keep the CPU defaults.
fn gpu_phases(objective: Objective, duration: u32) -> Vec<Phase> {
    use KernelId::*;
    let gpu = |k, mode, d| {
        let mut p = phase(k, Isa::Sse2, DataSize::Auto, d);
        p.mode = mode;
        p
    };
    let steady = LoadMode::Steady;
    match objective {
        Objective::Normal => {
            let d = scale(&[70, 30], duration);
            let mut load = gpu(S5, steady, d[0]);
            load.alt_kernel = Some(S1);
            vec![load, gpu(S1, LoadMode::Ramp, d[1])]
        }
        Objective::Overclock => {
            let d = scale(&[20, 10, 10, 15, 20, 10, 15], duration);
            let mut v = vec![
                gpu(S4, steady, d[0]),
                gpu(S2, steady, d[1]),
                gpu(S1, steady, d[2]),
                gpu(S6, steady, d[3]),
                gpu(S1, LoadMode::Ramp, d[4]),
                gpu(S1, LoadMode::Alternate, d[5]),
                gpu(S1, LoadMode::PauseResume, d[6]),
            ];
            v.iter_mut().for_each(|p| p.stop_on_error = true);
            v
        }
    }
}

/// One round of the overclock profile for `cores` and per-core time `t`; the all-core
/// phases last `a` seconds each.
fn oc_round(avx2: Isa, avx512: bool, cores: &[u32], t: u32, a: u32) -> Vec<Phase> {
    use KernelId::*;
    let on = |mut p: Phase| {
        p.stop_on_error = true;
        p
    };
    let mut r = vec![
        on(phase(K2, avx2, DataSize::L2, a)),
        on(phase(K5, avx2, DataSize::L3, a)),
        on(phase(K7, avx2, DataSize::L3, a)),
        on(phase(K3, avx2, DataSize::Ram, a)),
    ];
    let pc = t / 3;
    r.push(cycle(phase(K2, avx2, DataSize::L2, 0), cores, pc));
    r.push(cycle(phase(K5, avx2, DataSize::L2, 0), cores, pc));
    let mut light = cycle(phase(K2, Isa::Sse2, DataSize::L2, 0), cores, pc);
    light.mode = LoadMode::Light;
    r.push(light);
    let mut var = on(phase(K1, avx2, DataSize::Auto, a));
    var.mode = LoadMode::Variable;
    var.alt_kernel = Some(K5);
    r.push(var);
    r.push(on(phase(K4, avx2, DataSize::Auto, a)));
    r.push(on(phase(K9, avx2, DataSize::Auto, a)));
    if avx512 {
        r.push(on(phase(K2, Isa::Avx512, DataSize::L2, a)));
        r.push(on(phase(K1, Isa::Avx512, DataSize::Auto, a)));
    }
    r
}

/// CPU overclock profile (DA11): shrink the round to fit, repeat it while time remains.
fn cpu_overclock(avx2: Isa, avx512: bool, cores: &[u32], duration: u32) -> Vec<Phase> {
    let n = cores.len() as u32;
    let base_t = if duration <= 3600 {
        180
    } else if duration <= 7200 {
        300
    } else {
        600
    };
    let fixed_phases: u32 = if avx512 { 9 } else { 7 };
    let round_len = |t: u32, a: u32| fixed_phases * a + n * t;
    // Per-core time first (multiples of 3, down to 60), then the all-core phases (down
    // to 120 s, then to 60 s).
    let mut t = base_t;
    while t > 60 && round_len(t, 300) > duration {
        t -= 3;
    }
    let mut a = 300;
    while a > MIN_PHASE_S && round_len(t, a) > duration {
        a = if a > 120 { (a - 5).max(120) } else { a - 1 };
    }
    let round = oc_round(avx2, avx512, cores, t, a);
    let mut out: Vec<Phase> = Vec::new();
    let mut left = duration;
    loop {
        for p in &round {
            if left >= p.duration_s {
                left -= p.duration_s;
                out.push(p.clone());
                if left == 0 {
                    return out;
                }
                continue;
            }
            // The last phase is cut to what remains; under a minute it joins the previous.
            if left >= MIN_PHASE_S || out.is_empty() {
                let mut cut = p.clone();
                cut.duration_s = left;
                out.push(cut);
            } else if let Some(prev) = out.last_mut() {
                prev.duration_s += left;
            }
            return out;
        }
    }
}

/// Applies "Personalizza" (DA12) to the built phases.
fn apply_custom(mut out: Vec<Phase>, c: &Custom, has: &dyn Fn(Isa) -> bool) -> Vec<Phase> {
    for m in &c.modes {
        if !m.enabled || m.minutes == Some(0) {
            out.retain(|p| p.kernel != m.kernel);
            // A disabled kernel does not run as the alt kernel of a `variable` phase either.
            out.iter_mut()
                .filter(|p| p.alt_kernel == Some(m.kernel))
                .for_each(|p| p.alt_kernel = None);
            continue;
        }
        let idx: Vec<usize> = (0..out.len())
            .filter(|&i| out[i].kernel == m.kernel)
            .collect();
        let Some(minutes) = m.minutes.filter(|_| !idx.is_empty()) else {
            continue;
        };
        let weights: Vec<u32> = idx.iter().map(|&i| out[i].duration_s).collect();
        for (&i, d) in idx.iter().zip(scale(&weights, minutes.saturating_mul(60))) {
            let p = &mut out[i];
            p.duration_s = d.max(MIN_PHASE_S);
        }
    }
    for p in &mut out {
        if let Some(i) = c.isa.filter(|&i| p.mode != LoadMode::Light && has(i)) {
            p.isa = i;
        }
        if p.placement == Placement::CoreCycle {
            p.both_smt = c.both_smt;
        } else if p.placement == Placement::AllLogical && c.threads == ThreadChoice::OnePerCore {
            p.placement = Placement::OnePerCore;
        }
    }
    out
}

/// Builds the plan for a start request (DA11, DA12).
///
/// Contract for `core_cycle` phases: `duration_s` is authoritative and
/// `per_core_s == clamp(duration_s / n, 1, 3600)` with `n` the length of `cores`; the
/// engine cycles through the cores, wrapping, until `duration_s` has elapsed.
pub fn build_plan(input: &BuildInput) -> Result<Plan, BuildError> {
    let req = input.request;
    let is_gpu = req.component == Component::Gpu;
    if is_gpu && input.gpu.is_none() {
        return Err(BuildError::NoGpu);
    }
    let cores = core_order(input.topology);
    if cores.is_empty() || (is_gpu && req.retry_core.is_some()) {
        return Err(BuildError::NoCores);
    }
    let has = |i: Isa| input.isa.contains(&i);
    let best = [Isa::Avx512, Isa::Avx2, Isa::Sse2]
        .into_iter()
        .find(|&i| has(i))
        .unwrap_or(Isa::Sse2);
    let avx2 = if has(Isa::Avx2) { Isa::Avx2 } else { Isa::Sse2 };

    let mut phases = if let Some(r) = &req.retry_core {
        if !input.topology.logical.iter().any(|l| l.core == r.core) {
            return Err(BuildError::UnknownCore(r.core));
        }
        let one = [r.core];
        let mut light = cycle(
            phase(KernelId::K2, Isa::Sse2, DataSize::L2, 0),
            &one,
            RETRY_S,
        );
        light.mode = LoadMode::Light;
        let mut v = vec![
            cycle(phase(KernelId::K2, avx2, DataSize::L2, 0), &one, RETRY_S),
            cycle(phase(KernelId::K5, avx2, DataSize::L2, 0), &one, RETRY_S),
            light,
        ];
        if !matches!(r.kernel, KernelId::K2 | KernelId::K5) {
            let size = default_size(r.kernel);
            v.push(cycle(phase(r.kernel, best, size, 0), &one, RETRY_S));
        }
        v
    } else {
        let duration = presets(req.component, req.objective)
            .iter()
            .find(|(p, _)| *p == req.preset)
            .map(|&(_, s)| s)
            .ok_or(BuildError::NoPhases)?;
        match (req.component, req.objective) {
            (Component::Gpu, o) => gpu_phases(o, duration),
            (Component::Ram, o) => ram_phases(best, o, duration),
            (Component::Cpu, Objective::Normal) => cpu_normal(best, duration),
            (Component::Cpu, Objective::Overclock) => {
                cpu_overclock(avx2, has(Isa::Avx512), &cores, duration)
            }
        }
    };

    let mut stop = input.stop_override;
    // A retry keeps its own fixed phases: "Personalizza" does not apply to it.
    if let Some(c) = req.custom.as_ref().filter(|_| req.retry_core.is_none()) {
        // Instruction set and thread choice mean nothing to the GPU (DG9).
        let gpu_custom;
        let c = if is_gpu {
            gpu_custom = Custom {
                isa: None,
                threads: ThreadChoice::AllLogical,
                ..c.clone()
            };
            &gpu_custom
        } else {
            c
        };
        phases = apply_custom(phases, c, &has);
        stop = c.stop_on_first_error.or(stop);
    }
    if phases.is_empty() {
        return Err(BuildError::NoPhases);
    }
    // `duration_s` is authoritative; per_core_s follows it.
    for p in phases
        .iter_mut()
        .filter(|p| p.placement == Placement::CoreCycle)
    {
        let n = p.cores.as_ref().map_or(cores.len(), Vec::len).max(1) as u32;
        p.per_core_s = Some((p.duration_s / n).clamp(1, 3600));
    }
    if let Some(s) = stop {
        phases.iter_mut().for_each(|p| p.stop_on_error = s);
    }
    let plan = Plan {
        seed: input.seed,
        ram_bytes: if is_gpu { 0 } else { input.ram_budget },
        phases,
        gpu: if is_gpu { input.gpu } else { None },
    };
    if plan.total_seconds() > u64::from(MAX_PLAN_SECONDS) {
        return Err(BuildError::TooLong);
    }
    // CPU plans with K3/K4 build anyway: the runtime reduces or skips them (DA10).
    if req.component == Component::Ram && input.ram_budget < MIN_RAM_BYTES {
        return Err(BuildError::RamBudget);
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::load::{CacheSizes, LoadMessage, LogicalCpu, RunRequest};

    const GIB: u64 = 1 << 30;
    const ALL: [Isa; 3] = [Isa::Avx512, Isa::Avx2, Isa::Sse2];

    /// `groups` processor groups; `hybrid` makes all but the first third of the cores E cores.
    fn topo(cores: u32, smt: bool, groups: u16, hybrid: bool) -> Topology {
        let per_core = if smt { 2 } else { 1 };
        let total = cores * per_core;
        let per_group = total.div_ceil(u32::from(groups));
        let mut logical = Vec::new();
        for core in 0..cores {
            for t in 0..per_core {
                let index = core * per_core + t;
                logical.push(LogicalCpu {
                    index,
                    group: (index / per_group) as u16,
                    number: (index % per_group) as u8,
                    core,
                    core_index: core,
                    efficiency_class: if hybrid && core >= cores / 3 { 0 } else { 1 },
                    llc: 0,
                    parked: false,
                    apic_id: None,
                });
            }
        }
        Topology {
            logical,
            caches: CacheSizes {
                l1d_bytes: 32 << 10,
                l2_bytes: 1 << 20,
                l2_shared_by: per_core,
                l3_bytes: 32 << 20,
                l3_total_bytes: 32 << 20,
            },
            hypervisor: false,
            vendor: "AuthenticAMD".into(),
            brand: "Test".into(),
        }
    }

    fn req(c: Component, o: Objective, p: Preset) -> StartRequest {
        StartRequest {
            component: c,
            objective: o,
            preset: p,
            custom: None,
            retry_core: None,
            gpu: None,
        }
    }

    fn build(r: &StartRequest, t: &Topology, isa: &[Isa]) -> Result<Plan, BuildError> {
        build_with(r, t, isa, 8 * GIB, None)
    }

    fn build_with(
        r: &StartRequest,
        t: &Topology,
        isa: &[Isa],
        ram_budget: u64,
        stop_override: Option<bool>,
    ) -> Result<Plan, BuildError> {
        build_plan(&BuildInput {
            request: r,
            topology: t,
            isa,
            ram_budget,
            stop_override,
            seed: 1,
            gpu: None,
        })
    }

    fn ok(r: &StartRequest, t: &Topology, isa: &[Isa]) -> Plan {
        build(r, t, isa).unwrap()
    }

    fn custom() -> Custom {
        Custom {
            modes: vec![],
            isa: None,
            threads: ThreadChoice::AllLogical,
            both_smt: false,
            stop_on_first_error: None,
        }
    }

    #[test]
    fn every_profile_and_preset_sums_to_its_duration() {
        for t in [
            topo(1, false, 1, false),
            topo(8, true, 1, false),
            topo(24, true, 1, true),
        ] {
            for c in [Component::Cpu, Component::Ram] {
                for o in [Objective::Normal, Objective::Overclock] {
                    for &(p, secs) in presets(c, o) {
                        for isa in [&ALL[..], &ALL[1..], &ALL[2..]] {
                            let plan = ok(&req(c, o, p), &t, isa);
                            let what = format!("{c:?} {o:?} {p:?}");
                            assert_eq!(plan.total_seconds(), u64::from(secs), "{what}");
                            assert!(plan.phases.iter().all(|x| x.duration_s >= 60), "{what}");
                            for x in plan
                                .phases
                                .iter()
                                .filter(|x| x.placement == Placement::CoreCycle)
                            {
                                let n = x.cores.as_ref().unwrap().len() as u32;
                                assert_eq!(
                                    x.per_core_s,
                                    Some((x.duration_s / n).clamp(1, 3600)),
                                    "{what}"
                                );
                            }
                            LoadMessage::Run(RunRequest { plan }).validate().unwrap();
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn cpu_normal_standard_matches_the_table() {
        let t = topo(8, true, 1, false);
        let plan = ok(
            &req(Component::Cpu, Objective::Normal, Preset::Standard),
            &t,
            &ALL,
        );
        let got: Vec<_> = plan
            .phases
            .iter()
            .map(|p| (p.kernel, p.size, p.duration_s))
            .collect();
        use DataSize::*;
        use KernelId::*;
        assert_eq!(
            got,
            [
                (K1, Auto, 600),
                (K2, L2, 360),
                (K7, L3, 300),
                (K8, Auto, 300),
                (K3, Ram, 240)
            ]
        );
        assert!(plan
            .phases
            .iter()
            .all(|p| p.isa == Isa::Avx512 && !p.stop_on_error));
        let long = ok(
            &req(Component::Cpu, Objective::Normal, Preset::Long),
            &t,
            &ALL,
        );
        assert_eq!(long.phases[0].duration_s, 1200);
        let quick = ok(
            &req(Component::Cpu, Objective::Normal, Preset::Quick),
            &t,
            &ALL,
        );
        let q: Vec<_> = quick
            .phases
            .iter()
            .map(|p| (p.kernel, p.duration_s))
            .collect();
        assert_eq!(q, [(K2, 120), (K5, 90), (K8, 90)]);
    }

    #[test]
    fn oc_standard_8_cores_with_avx512() {
        let plan = ok(
            &req(Component::Cpu, Objective::Overclock, Preset::Standard),
            &topo(8, true, 1, false),
            &ALL,
        );
        let cycles: Vec<_> = plan
            .phases
            .iter()
            .filter(|p| p.placement == Placement::CoreCycle)
            .collect();
        assert_eq!(cycles.len(), 3);
        assert!(cycles
            .iter()
            .all(|p| p.per_core_s == Some(37) && p.duration_s == 296));
        assert_eq!(plan.phases.len(), 12);
        let last = plan.phases.last().unwrap();
        assert_eq!((last.kernel, last.isa), (KernelId::K1, Isa::Avx512));
        assert_eq!(last.duration_s, 312);
        assert_eq!(plan.total_seconds(), 3600);
    }

    #[test]
    fn oc_without_avx512_has_no_phase_5() {
        let plan = ok(
            &req(Component::Cpu, Objective::Overclock, Preset::Standard),
            &topo(8, true, 1, false),
            &ALL[1..],
        );
        assert!(plan.phases.iter().all(|p| p.isa != Isa::Avx512));
        assert_eq!(plan.total_seconds(), 3600);
    }

    #[test]
    fn oc_with_many_cores_shrinks_per_core_time_to_60() {
        let plan = ok(
            &req(Component::Cpu, Objective::Overclock, Preset::Standard),
            &topo(64, false, 1, false),
            &ALL[1..],
        );
        let c = plan
            .phases
            .iter()
            .find(|p| p.placement == Placement::CoreCycle)
            .unwrap();
        assert_eq!(c.per_core_s, Some(20));
        assert_eq!(plan.total_seconds(), 3600);
    }

    #[test]
    fn core_cycle_orders_p_before_e_and_includes_parked() {
        let mut t = topo(6, false, 1, true); // cores 0,1 P; 2..5 E
        t.logical[0].parked = true; // core 0 parked entirely: still tested
        assert_eq!(core_order(&t), [0, 1, 2, 3, 4, 5]);
        t.logical[3].efficiency_class = 2; // core 3 becomes the fastest
        assert_eq!(core_order(&t), [3, 0, 1, 2, 4, 5]);
        let plan = ok(
            &req(Component::Cpu, Objective::Overclock, Preset::Standard),
            &t,
            &ALL,
        );
        let c = plan
            .phases
            .iter()
            .find(|p| p.placement == Placement::CoreCycle)
            .unwrap();
        assert_eq!(c.cores.as_deref(), Some(&[3, 0, 1, 2, 4, 5][..]));
    }

    #[test]
    fn parked_smt_siblings_keep_their_core() {
        let mut t = topo(4, true, 1, false);
        // Core 1 fully parked, core 2 with one parked sibling.
        for i in [2, 3, 5] {
            t.logical[i].parked = true;
        }
        assert_eq!(core_order(&t), [0, 1, 2, 3]);
        let plan = ok(
            &req(Component::Cpu, Objective::Overclock, Preset::Standard),
            &t,
            &ALL,
        );
        assert!(plan
            .phases
            .iter()
            .filter(|p| p.placement == Placement::CoreCycle)
            .all(|p| p.cores.as_deref() == Some(&[0, 1, 2, 3][..])));
    }

    #[test]
    fn plan_for_128_logical_in_two_groups() {
        let t = topo(64, true, 2, false);
        assert_eq!(t.logical.len(), 128);
        for o in [Objective::Normal, Objective::Overclock] {
            let plan = ok(&req(Component::Cpu, o, Preset::Standard), &t, &ALL);
            LoadMessage::Run(RunRequest { plan }).validate().unwrap();
        }
    }

    #[test]
    fn single_core_machine_builds_every_profile() {
        let t = topo(1, false, 1, false);
        for c in [Component::Cpu, Component::Ram] {
            for o in [Objective::Normal, Objective::Overclock] {
                for &(p, _) in presets(c, o) {
                    ok(&req(c, o, p), &t, &ALL[2..]);
                }
            }
        }
        let mut parked = t.clone();
        parked.logical[0].parked = true;
        ok(
            &req(Component::Cpu, Objective::Normal, Preset::Quick),
            &parked,
            &ALL,
        );
        let mut empty = t;
        empty.logical.clear();
        assert_eq!(
            build(
                &req(Component::Cpu, Objective::Normal, Preset::Quick),
                &empty,
                &ALL
            ),
            Err(BuildError::NoCores)
        );
    }

    #[test]
    fn custom_disables_and_rescales_modes() {
        let mut r = req(Component::Cpu, Objective::Normal, Preset::Standard);
        let mut c = custom();
        c.modes = vec![
            ModeEdit {
                kernel: KernelId::K1,
                enabled: false,
                minutes: None,
            },
            ModeEdit {
                kernel: KernelId::K2,
                enabled: true,
                minutes: Some(10),
            },
        ];
        r.custom = Some(c);
        let plan = ok(&r, &topo(8, true, 1, false), &ALL);
        let got: Vec<_> = plan
            .phases
            .iter()
            .map(|p| (p.kernel, p.duration_s))
            .collect();
        use KernelId::*;
        assert_eq!(got, [(K2, 600), (K7, 300), (K8, 300), (K3, 240)]);
    }

    #[test]
    fn custom_isa_keeps_light_on_sse2() {
        let mut r = req(Component::Cpu, Objective::Overclock, Preset::Standard);
        let mut c = custom();
        c.isa = Some(Isa::Avx2);
        r.custom = Some(c);
        let plan = ok(&r, &topo(8, true, 1, false), &ALL);
        for p in &plan.phases {
            let want = if p.mode == LoadMode::Light {
                Isa::Sse2
            } else {
                Isa::Avx2
            };
            assert_eq!(p.isa, want);
        }
    }

    #[test]
    fn custom_both_smt_applies_to_core_cycle_only() {
        let mut r = req(Component::Cpu, Objective::Overclock, Preset::Standard);
        let mut c = custom();
        c.both_smt = true;
        c.threads = ThreadChoice::OnePerCore;
        r.custom = Some(c);
        let plan = ok(&r, &topo(8, true, 1, false), &ALL);
        for p in &plan.phases {
            assert_eq!(p.both_smt, p.placement == Placement::CoreCycle);
            assert_ne!(p.placement, Placement::AllLogical);
        }
    }

    #[test]
    fn custom_removing_everything_is_no_phases() {
        let mut r = req(Component::Cpu, Objective::Normal, Preset::Quick);
        let mut c = custom();
        c.modes = [KernelId::K2, KernelId::K5, KernelId::K8]
            .map(|kernel| ModeEdit {
                kernel,
                enabled: false,
                minutes: None,
            })
            .to_vec();
        r.custom = Some(c);
        assert_eq!(
            build(&r, &topo(4, true, 1, false), &ALL),
            Err(BuildError::NoPhases)
        );
    }

    #[test]
    fn custom_over_24_hours_is_too_long() {
        let mut r = req(Component::Cpu, Objective::Normal, Preset::Quick);
        let mut c = custom();
        c.modes = vec![ModeEdit {
            kernel: KernelId::K2,
            enabled: true,
            minutes: Some(25 * 60),
        }];
        r.custom = Some(c);
        assert_eq!(
            build(&r, &topo(4, true, 1, false), &ALL),
            Err(BuildError::TooLong)
        );
    }

    #[test]
    fn stop_override_and_custom_precedence() {
        let t = topo(4, true, 1, false);
        let mut r = req(Component::Cpu, Objective::Overclock, Preset::Standard);
        let stops = |p: Plan| p.phases.iter().map(|x| x.stop_on_error).collect::<Vec<_>>();
        let run = |r: &StartRequest, o| stops(build_with(r, &t, &ALL, 8 * GIB, o).unwrap());
        // Profile: the per-core phases never stop, the others do.
        let base = run(&r, None);
        assert!(base.contains(&true) && base.contains(&false));
        assert!(run(&r, Some(false)).iter().all(|s| !s));
        assert!(run(&r, Some(true)).iter().all(|s| *s));
        let mut c = custom();
        c.stop_on_first_error = Some(false);
        r.custom = Some(c);
        assert!(run(&r, Some(true)).iter().all(|s| !s));
    }

    #[test]
    fn retry_core_builds_only_that_core() {
        let t = topo(8, true, 1, false);
        let mut r = req(Component::Cpu, Objective::Overclock, Preset::Standard);
        r.retry_core = Some(RetryCore {
            core: 5,
            kernel: KernelId::K2,
        });
        let plan = ok(&r, &t, &ALL);
        assert_eq!(plan.phases.len(), 3);
        assert!(plan
            .phases
            .iter()
            .all(|p| p.cores.as_deref() == Some(&[5][..]) && p.duration_s == 120));
        r.retry_core = Some(RetryCore {
            core: 5,
            kernel: KernelId::K7,
        });
        let plan = ok(&r, &t, &ALL);
        assert_eq!(plan.phases.len(), 4);
        assert_eq!(plan.phases[3].kernel, KernelId::K7);
        assert_eq!(plan.total_seconds(), 480);
        r.retry_core = Some(RetryCore {
            core: 99,
            kernel: KernelId::K2,
        });
        assert_eq!(build(&r, &t, &ALL), Err(BuildError::UnknownCore(99)));
    }

    #[test]
    fn retry_core_ignores_custom() {
        let t = topo(8, true, 1, false);
        let mut r = req(Component::Cpu, Objective::Overclock, Preset::Standard);
        r.retry_core = Some(RetryCore {
            core: 5,
            kernel: KernelId::K2,
        });
        let plain = ok(&r, &t, &ALL);
        let mut c = custom();
        c.modes = vec![ModeEdit {
            kernel: KernelId::K2,
            enabled: false,
            minutes: None,
        }];
        c.isa = Some(Isa::Avx512);
        c.both_smt = true;
        c.stop_on_first_error = Some(true);
        r.custom = Some(c);
        assert_eq!(ok(&r, &t, &ALL), plain);
    }

    #[test]
    fn custom_without_k5_drops_the_alt_kernel() {
        let mut r = req(Component::Cpu, Objective::Overclock, Preset::Standard);
        let mut c = custom();
        c.modes = vec![ModeEdit {
            kernel: KernelId::K5,
            enabled: false,
            minutes: None,
        }];
        r.custom = Some(c);
        let plan = ok(&r, &topo(8, true, 1, false), &ALL);
        assert!(plan.phases.iter().all(|p| p.kernel != KernelId::K5));
        let var = plan
            .phases
            .iter()
            .find(|p| p.mode == LoadMode::Variable)
            .unwrap();
        assert_eq!((var.kernel, var.alt_kernel), (KernelId::K1, None));
    }

    #[test]
    fn ram_budget_leaves_two_gib_free() {
        assert_eq!(ram_budget(8 * GIB, 70), 8 * GIB * 70 / 100);
        assert_eq!(ram_budget(3 * GIB, 70), GIB);
        assert_eq!(ram_budget(GIB, 70), 0);
    }

    #[test]
    fn ram_profiles_fail_below_256_mib() {
        let t = topo(4, true, 1, false);
        let ram = req(Component::Ram, Objective::Normal, Preset::Quick);
        let low = |r: &StartRequest, b| build_with(r, &t, &ALL, b, None);
        assert_eq!(low(&ram, (256 << 20) - 1), Err(BuildError::RamBudget));
        assert!(low(&ram, 256 << 20).is_ok());
    }

    #[test]
    fn cpu_plan_builds_with_a_tiny_ram_budget() {
        let t = topo(4, true, 1, false);
        for p in [Preset::Standard, Preset::Long] {
            let r = req(Component::Cpu, Objective::Normal, p);
            let plan = build_with(&r, &t, &ALL, 0, None).unwrap();
            assert!(plan.phases.iter().any(|x| x.kernel == KernelId::K3));
        }
        let oc = req(Component::Cpu, Objective::Overclock, Preset::Standard);
        assert!(build_with(&oc, &t, &ALL, 0, None).is_ok());
    }

    #[test]
    fn custom_minutes_edge_cases() {
        let t = topo(4, true, 1, false);
        let with = |minutes| {
            let mut r = req(Component::Cpu, Objective::Normal, Preset::Standard);
            let mut c = custom();
            c.modes = vec![ModeEdit {
                kernel: KernelId::K2,
                enabled: true,
                minutes: Some(minutes),
            }];
            r.custom = Some(c);
            build(&r, &t, &ALL)
        };
        assert_eq!(with(u32::MAX), Err(BuildError::TooLong));
        let zero = with(0).unwrap();
        assert!(zero.phases.iter().all(|p| p.kernel != KernelId::K2));
        let one = with(1).unwrap();
        assert!(one.phases.iter().all(|p| p.duration_s >= 60));
    }

    #[test]
    fn profile_snapshots() {
        use DataSize::*;
        use Isa::*;
        use KernelId::*;
        use LoadMode::*;
        use Placement::*;
        let t = topo(8, true, 1, false);
        let oc = ok(
            &req(Component::Cpu, Objective::Overclock, Preset::Standard),
            &t,
            &ALL,
        );
        let got: Vec<_> = oc
            .phases
            .iter()
            .map(|p| {
                (
                    p.kernel,
                    p.alt_kernel,
                    p.isa,
                    p.size,
                    p.mode,
                    p.placement,
                    p.stop_on_error,
                )
            })
            .collect();
        let want = vec![
            (K2, None, Avx2, L2, Steady, AllLogical, true),
            (K5, None, Avx2, L3, Steady, AllLogical, true),
            (K7, None, Avx2, L3, Steady, AllLogical, true),
            (K3, None, Avx2, Ram, Steady, AllLogical, true),
            (K2, None, Avx2, L2, Steady, CoreCycle, false),
            (K5, None, Avx2, L2, Steady, CoreCycle, false),
            (K2, None, Sse2, L2, Light, CoreCycle, false),
            (K1, Some(K5), Avx2, Auto, Variable, AllLogical, true),
            (K4, None, Avx2, Auto, Steady, AllLogical, true),
            (K9, None, Avx2, Auto, Steady, AllLogical, true),
            (K2, None, Avx512, L2, Steady, AllLogical, true),
            (K1, None, Avx512, Auto, Steady, AllLogical, true),
        ];
        assert_eq!(got, want);

        let all = vec![
            RamPattern::MovingInversions,
            RamPattern::Modulo20,
            RamPattern::Random,
            RamPattern::Address,
            RamPattern::CrcCopy,
        ];
        let ram = |o| {
            let p = ok(&req(Component::Ram, o, Preset::Standard), &t, &ALL);
            p.phases
                .iter()
                .map(|x| (x.kernel, x.patterns.clone(), x.duration_s, x.stop_on_error))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ram(Objective::Normal),
            vec![
                (
                    K10,
                    vec![
                        RamPattern::MovingInversions,
                        RamPattern::Random,
                        RamPattern::CrcCopy
                    ],
                    1260,
                    false
                ),
                (K3, vec![], 540, false),
            ]
        );
        assert_eq!(
            ram(Objective::Overclock),
            vec![
                (K10, all, 2160, true),
                (K3, vec![], 900, true),
                (K4, vec![], 540, true)
            ]
        );
    }

    fn gpu_target() -> GpuTarget {
        GpuTarget {
            luid: 0x1234,
            integrated: false,
        }
    }

    fn gpu_req(o: Objective, p: Preset) -> StartRequest {
        StartRequest {
            gpu: Some("gpu-0".into()),
            ..req(Component::Gpu, o, p)
        }
    }

    fn build_gpu(r: &StartRequest, gpu: Option<GpuTarget>) -> Result<Plan, BuildError> {
        build_plan(&BuildInput {
            request: r,
            topology: &topo(4, true, 1, false),
            isa: &ALL,
            ram_budget: 8 * GIB,
            stop_override: None,
            seed: 1,
            gpu,
        })
    }

    fn ok_gpu(r: &StartRequest) -> Plan {
        let plan = build_gpu(r, Some(gpu_target())).unwrap();
        LoadMessage::Run(RunRequest { plan: plan.clone() })
            .validate()
            .unwrap();
        plan
    }

    fn shape(p: &Plan) -> Vec<(KernelId, LoadMode, u32)> {
        p.phases
            .iter()
            .map(|p| (p.kernel, p.mode, p.duration_s))
            .collect()
    }

    #[test]
    fn gpu_presets_match_the_spec() {
        use Preset::*;
        assert_eq!(
            presets(Component::Gpu, Objective::Normal),
            &[(Quick, 300), (Standard, 900), (Long, 1800)]
        );
        assert_eq!(
            presets(Component::Gpu, Objective::Overclock),
            &[(Standard, 1800), (Long, 3600), (Night, 7200)]
        );
    }

    #[test]
    fn gpu_normal_plan_is_s5_s1_then_ramp() {
        use KernelId::*;
        let plan = ok_gpu(&gpu_req(Objective::Normal, Preset::Standard));
        assert_eq!(
            shape(&plan),
            [(S5, LoadMode::Steady, 630), (S1, LoadMode::Ramp, 270)]
        );
        assert_eq!(plan.phases[0].alt_kernel, Some(S1));
        assert!(plan.phases.iter().all(|p| !p.stop_on_error));
        assert_eq!(plan.gpu, Some(gpu_target()));
        assert_eq!(plan.ram_bytes, 0);
        let p = &plan.phases[0];
        assert_eq!(
            (p.isa, p.size, p.placement),
            (Isa::Sse2, DataSize::Auto, Placement::AllLogical)
        );
        assert!(p.patterns.is_empty() && p.iterations.is_none() && p.pause_before_ms == 0);
    }

    #[test]
    fn gpu_overclock_round_has_the_seven_phases() {
        use KernelId::*;
        use LoadMode::*;
        let plan = ok_gpu(&gpu_req(Objective::Overclock, Preset::Standard));
        assert_eq!(
            shape(&plan),
            [
                (S4, Steady, 360),
                (S2, Steady, 180),
                (S1, Steady, 180),
                (S6, Steady, 270),
                (S1, Ramp, 360),
                (S1, Alternate, 180),
                (S1, PauseResume, 270),
            ]
        );
        assert!(plan.phases.iter().all(|p| p.stop_on_error));
    }

    #[test]
    fn gpu_phase_totals_equal_the_preset() {
        for o in [Objective::Normal, Objective::Overclock] {
            for &(p, secs) in presets(Component::Gpu, o) {
                let plan = ok_gpu(&gpu_req(o, p));
                assert_eq!(plan.total_seconds(), u64::from(secs), "{o:?} {p:?}");
            }
        }
    }

    #[test]
    fn gpu_without_target_is_no_gpu() {
        let r = gpu_req(Objective::Normal, Preset::Quick);
        assert_eq!(build_gpu(&r, None), Err(BuildError::NoGpu));
    }

    #[test]
    fn gpu_custom_ignores_isa_and_threads() {
        let mut r = gpu_req(Objective::Normal, Preset::Standard);
        r.custom = Some(Custom {
            isa: Some(Isa::Avx512),
            threads: ThreadChoice::OnePerCore,
            ..custom()
        });
        let plan = ok_gpu(&r);
        assert!(plan
            .phases
            .iter()
            .all(|p| p.isa == Isa::Sse2 && p.placement == Placement::AllLogical));
    }

    #[test]
    fn gpu_custom_minutes_rescale_the_phase() {
        let mut r = gpu_req(Objective::Overclock, Preset::Standard);
        r.custom = Some(Custom {
            modes: vec![ModeEdit {
                kernel: KernelId::S4,
                enabled: true,
                minutes: Some(10),
            }],
            ..custom()
        });
        let plan = ok_gpu(&r);
        assert_eq!(plan.phases[0].duration_s, 600);
        assert_eq!(plan.total_seconds(), 600 + 1440);
    }

    #[test]
    fn gpu_retry_core_is_refused() {
        let mut r = gpu_req(Objective::Normal, Preset::Quick);
        r.retry_core = Some(RetryCore {
            core: 0,
            kernel: KernelId::K2,
        });
        assert_eq!(build_gpu(&r, Some(gpu_target())), Err(BuildError::NoCores));
    }

    #[test]
    fn cpu_plans_have_no_gpu() {
        let t = topo(4, true, 1, false);
        for c in [Component::Cpu, Component::Ram] {
            let plan = ok(&req(c, Objective::Normal, Preset::Standard), &t, &ALL);
            assert_eq!(plan.gpu, None);
        }
    }
}
