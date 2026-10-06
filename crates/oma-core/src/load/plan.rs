//! Profiles (DA11), "Personalizza" (DA12), RAM quota (DA10) and the plan builder.

use oma_ipc::load::{
    DataSize, Isa, KernelId, LoadMode, Phase, Placement, Plan, RamPattern, Topology,
    MAX_PLAN_SECONDS,
};
use serde::{Deserialize, Serialize};

const MIN_PHASE_S: u32 = 60;
const MIN_RAM_BYTES: u64 = 256 << 20;
const KEEP_FREE_BYTES: u64 = 2 << 30;
const RETRY_S: u32 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Component {
    Cpu,
    Ram,
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
}

/// The preset durations in seconds (DA11).
pub fn presets(component: Component, objective: Objective) -> &'static [(Preset, u32)] {
    use Preset::*;
    match (component, objective) {
        (Component::Cpu, Objective::Normal) => &[(Quick, 300), (Standard, 1800), (Long, 3600)],
        (Component::Ram, Objective::Normal) => &[(Quick, 900), (Standard, 1800), (Long, 3600)],
        (_, Objective::Overclock) => &[(Standard, 3600), (Long, 7200), (Night, 28_800)],
    }
}

/// RAM share for the RAM tests (DA10): the percentage of the available memory, always
/// leaving 2 GiB free.
pub fn ram_budget(available: u64, percent: u32) -> u64 {
    let share = (u128::from(available) * u128::from(percent) / 100) as u64;
    share.min(available.saturating_sub(KEEP_FREE_BYTES))
}

/// Physical cores in test order (DA4): higher efficiency class first, then by number;
/// cores whose logical processors are all parked are left out.
pub fn core_order(topology: &Topology) -> Vec<u32> {
    let mut cores: Vec<(u8, u32)> = Vec::new();
    for l in topology.logical.iter().filter(|l| !l.parked) {
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
    let base_t = match duration {
        ..3601 => 180,
        ..7201 => 300,
        _ => 600,
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
                if let Some(pc) = cut.per_core_s {
                    cut.per_core_s = Some((left / n).clamp(1, pc));
                }
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
        if !m.enabled {
            out.retain(|p| p.kernel != m.kernel);
            continue;
        }
        let idx: Vec<usize> = (0..out.len())
            .filter(|&i| out[i].kernel == m.kernel)
            .collect();
        let Some(minutes) = m.minutes.filter(|_| !idx.is_empty()) else {
            continue;
        };
        let weights: Vec<u32> = idx.iter().map(|&i| out[i].duration_s).collect();
        for (&i, d) in idx.iter().zip(scale(&weights, minutes * 60)) {
            let p = &mut out[i];
            p.duration_s = d.max(1);
            if let (Some(pc), Some(cores)) = (p.per_core_s, &p.cores) {
                p.per_core_s = Some((d / cores.len().max(1) as u32).clamp(1, pc.max(1)));
            }
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
pub fn build_plan(input: &BuildInput) -> Result<Plan, BuildError> {
    let req = input.request;
    let cores = core_order(input.topology);
    if cores.is_empty() {
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
            (Component::Ram, o) => ram_phases(best, o, duration),
            (Component::Cpu, Objective::Normal) => cpu_normal(best, duration),
            (Component::Cpu, Objective::Overclock) => {
                cpu_overclock(avx2, has(Isa::Avx512), &cores, duration)
            }
        }
    };

    let mut stop = input.stop_override;
    if let Some(c) = &req.custom {
        phases = apply_custom(phases, c, &has);
        stop = c.stop_on_first_error.or(stop);
    }
    if phases.is_empty() {
        return Err(BuildError::NoPhases);
    }
    if let Some(s) = stop {
        phases.iter_mut().for_each(|p| p.stop_on_error = s);
    }
    let plan = Plan {
        seed: input.seed,
        ram_bytes: input.ram_budget,
        phases,
    };
    if plan.total_seconds() > u64::from(MAX_PLAN_SECONDS) {
        return Err(BuildError::TooLong);
    }
    let uses_ram = plan
        .phases
        .iter()
        .any(|p| matches!(p.kernel, KernelId::K3 | KernelId::K4 | KernelId::K10));
    if uses_ram && input.ram_budget < MIN_RAM_BYTES {
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
    fn core_cycle_orders_p_before_e_and_skips_parked() {
        let mut t = topo(6, false, 1, true); // cores 0,1 P; 2..5 E
        t.logical[0].parked = true; // core 0 parked entirely
        assert_eq!(core_order(&t), [1, 2, 3, 4, 5]);
        t.logical[3].efficiency_class = 2; // core 3 becomes the fastest
        assert_eq!(core_order(&t), [3, 1, 2, 4, 5]);
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
        assert_eq!(c.cores.as_deref(), Some(&[3, 1, 2, 4, 5][..]));
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
        let mut empty = t;
        empty.logical[0].parked = true;
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
        // A CPU quick profile does not use the RAM kernels.
        assert!(low(&req(Component::Cpu, Objective::Normal, Preset::Quick), 0).is_ok());
    }
}
