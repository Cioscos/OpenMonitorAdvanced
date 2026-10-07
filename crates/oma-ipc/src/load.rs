//! App <-> `oma-load.exe` protocol (version 4): the messages the app and the stress-test
//! helper exchange over the load pipe, framed with the generic framing of this crate.
//!
//! Same conventions as the service and overlay protocols: every field is always present on
//! the wire (`nil` for "absent"; no `skip_serializing_if`), enumerated values travel as
//! `snake_case` strings, and unknown fields are ignored. A receiver calls
//! [`LoadMessage::validate`] after decoding.

use serde::{Deserialize, Serialize};

use crate::overlay::check_len;
use crate::IpcError;

/// Load protocol version, sent in [`LoadHello::protocol_version`] by both sides.
pub const LOAD_PROTOCOL_VERSION: u32 = 5;

/// Warm-up of a GPU benchmark phase, in seconds: the first windows are not reported.
pub const GPU_BENCH_WARMUP_S: u32 = 4;

/// Prefix of the load pipe name; the app appends a random UUID v4.
pub const LOAD_PIPE_PREFIX: &str = r"\\.\pipe\OpenMonitorAdvanced-Load-";

/// Maximum phases in a [`Plan`].
pub const MAX_PHASES: usize = 512;
/// Maximum total duration of a [`Plan`], in seconds.
pub const MAX_PLAN_SECONDS: u32 = 86_400;
/// Maximum entries in the per-CPU lists (`logical`, `cores`).
pub const MAX_LOGICAL: usize = 1024;
/// Maximum length of any text field, in bytes.
pub const MAX_TEXT_BYTES: usize = 256;
/// Maximum [`Phase::iterations`].
pub const MAX_ITERATIONS: u64 = 1_000_000_000;
/// Maximum [`Phase::pause_before_ms`].
pub const MAX_PAUSE_MS: u32 = 10_000;
/// Maximum [`Plan::ram_bytes`].
pub const MAX_RAM_BYTES: u64 = 1 << 40;

/// Handshake, sent by both sides. The app sends an empty `isa`; the process lists the
/// instruction sets it can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadHello {
    pub protocol_version: u32,
    pub version: String,
    pub isa: Vec<Isa>,
    /// `oma-load` only: fingerprint of the compiled shaders (16 lowercase hex digits).
    #[serde(default)]
    pub shader_digest: Option<String>,
}

/// App to process: stop the running plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopRequest {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isa {
    Avx512,
    Avx2,
    Sse2,
}

/// The instruction sets this CPU runs, best first; the app and `oma-load` use the same
/// policy. `avx512` asks for AVX-512F only (the kernels use F alone); `avx2` also needs
/// FMA; SSE2 is part of x86_64. Empty on other architectures, which have no kernels.
pub fn detected_isa() -> Vec<Isa> {
    let mut isa = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            isa.push(Isa::Avx512);
        }
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            isa.push(Isa::Avx2);
        }
        isa.push(Isa::Sse2);
    }
    isa
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelId {
    K1,
    K2,
    K3,
    K4,
    K5,
    K7,
    K8,
    K9,
    K10,
    /// Benchmark-only loads (fixed work, `iterations` required).
    Hash,
    Compress,
    Sort,
    /// GPU loads (D3D11): FMA chains, integer hash, VRAM check, graphics load, artifact scan.
    S1,
    S2,
    S4,
    S5,
    S6,
    /// Memory stream (stress and benchmark) and the benchmark-only graphics loads.
    S3,
    Fill,
    Texture,
    Overdraw,
}

impl KernelId {
    /// True for the loads that run on the GPU.
    pub fn is_gpu(self) -> bool {
        matches!(
            self,
            Self::S1
                | Self::S2
                | Self::S3
                | Self::S4
                | Self::S5
                | Self::S6
                | Self::Fill
                | Self::Texture
                | Self::Overdraw
        )
    }

    /// True for the GPU loads that only run as benchmark phases (`windows` required).
    pub fn is_gpu_bench_only(self) -> bool {
        matches!(self, Self::Fill | Self::Texture | Self::Overdraw)
    }

    /// True for the loads that only run as fixed-work benchmark phases.
    pub fn is_bench_only(self) -> bool {
        matches!(self, Self::Hash | Self::Compress | Self::Sort)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSize {
    L1,
    L2,
    L3,
    Ram,
    Auto,
    /// Fixed size, the same on every machine (benchmark).
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadMode {
    Steady,
    Variable,
    Light,
    /// GPU only: load ramps from 20 to 100 %.
    Ramp,
    /// GPU only: full load and 15 % load alternate.
    Alternate,
    /// GPU only: full load, then a pause, repeated.
    PauseResume,
}

impl LoadMode {
    /// True for the modes only the GPU kernels run.
    pub fn is_gpu_only(self) -> bool {
        matches!(self, Self::Ramp | Self::Alternate | Self::PauseResume)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    AllLogical,
    OnePerCore,
    CoreCycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RamPattern {
    MovingInversions,
    Modulo20,
    Random,
    Address,
    CrcCopy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreState {
    Untested,
    Testing,
    Passed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Mismatch,
    ReferenceDisagreement,
    ReferenceInvalid,
    Hung,
    /// The GPU was removed or reset; `actual` carries the device-removed HRESULT.
    DeviceLost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Completed,
    Stopped,
    FirstError,
    Failed,
}

/// One logical processor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalCpu {
    /// Global, progressive index.
    pub index: u32,
    pub group: u16,
    pub number: u8,
    pub core: u32,
    pub core_index: u32,
    pub efficiency_class: u8,
    pub llc: u32,
    pub parked: bool,
    pub apic_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheSizes {
    pub l1d_bytes: u64,
    pub l2_bytes: u64,
    pub l2_shared_by: u32,
    pub l3_bytes: u64,
    pub l3_total_bytes: u64,
}

/// Process to app: the machine topology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topology {
    pub logical: Vec<LogicalCpu>,
    pub caches: CacheSizes,
    pub hypervisor: bool,
    pub vendor: String,
    pub brand: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    pub kernel: KernelId,
    pub alt_kernel: Option<KernelId>,
    pub isa: Isa,
    pub size: DataSize,
    pub mode: LoadMode,
    pub placement: Placement,
    pub duration_s: u32,
    pub per_core_s: Option<u32>,
    pub both_smt: bool,
    pub cores: Option<Vec<u32>>,
    pub patterns: Vec<RamPattern>,
    pub stop_on_error: bool,
    /// Fixed work: iterations per thread (benchmark). `serde(default)` only reads v1 sessions.
    #[serde(default)]
    pub iterations: Option<u64>,
    /// Pause before the phase starts, in milliseconds.
    #[serde(default)]
    pub pause_before_ms: u32,
    /// GPU benchmark: the measured 1 s windows after the warm-up.
    #[serde(default)]
    pub windows: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub seed: u64,
    pub ram_bytes: u64,
    pub phases: Vec<Phase>,
    /// Target GPU of a GPU plan; a plan without it runs CPU kernels only.
    #[serde(default)]
    pub gpu: Option<GpuTarget>,
}

/// The adapter a GPU plan runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuTarget {
    /// The adapter LUID (`HighPart << 32 | LowPart`), never 0.
    pub luid: u64,
    pub integrated: bool,
}

impl Plan {
    /// Sum of the phase durations, in seconds.
    pub fn total_seconds(&self) -> u64 {
        self.phases.iter().map(|p| u64::from(p.duration_s)).sum()
    }
}

/// App to process: run a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRequest {
    pub plan: Plan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreProgress {
    pub core: u32,
    pub state: CoreState,
}

/// Process to app: periodic progress. `rate` is the iterations per second of all threads
/// since the previous `Progress`, the phase start or the gate, whichever is last: the first
/// `Progress` of a phase and those of a pause or a reference carry 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub phase: u32,
    pub phase_elapsed_ms: u64,
    pub elapsed_ms: u64,
    pub checks: u64,
    pub errors: u64,
    pub current_core: Option<u32>,
    pub cores: Vec<CoreProgress>,
    pub memory_bytes: u64,
    pub rate: Option<f64>,
    /// GPU `ramp` and `alternate`: the current load level, 1-100.
    #[serde(default)]
    pub load_percent: Option<u8>,
}

/// Process to app: a computation error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeError {
    pub phase: u32,
    pub kernel: KernelId,
    pub isa: Isa,
    pub kind: ErrorKind,
    pub logical: Option<u32>,
    pub core: Option<u32>,
    pub iteration: u64,
    pub expected: u64,
    pub actual: u64,
    pub seed: u64,
    /// GPU `ramp` and `alternate`: the load level when the error happened, 1-100.
    #[serde(default)]
    pub load_percent: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub phase: u32,
    pub code: String,
    pub value: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseDone {
    pub phase: u32,
    pub checks: u64,
    pub errors: u64,
    pub duration_ms: u64,
    pub skipped: Option<String>,
    /// Fixed-work phases: milliseconds from the gate opening to the last thread done.
    #[serde(default)]
    pub work_ms: Option<u64>,
    /// Fixed-work phases: each thread's counted iterations and time (DB12); empty for the
    /// timed and the skipped phases.
    #[serde(default)]
    pub workers: Vec<WorkerDone>,
    /// GPU benchmark phases: one rate per measured window, in base units per second
    /// (FLOP, operations, bytes, pixels or texels); empty for the other phases.
    #[serde(default)]
    pub rates: Vec<f64>,
}

/// One thread of a fixed-work phase: the iterations it counted and the milliseconds from
/// the gate opening to its last one (or to the cap, a stop or an error).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerDone {
    pub logical: u32,
    pub iterations: u64,
    pub work_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finished {
    pub reason: FinishReason,
    pub checks: u64,
    pub errors: u64,
}

/// A message on the load pipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum LoadMessage {
    Hello(LoadHello),
    Run(RunRequest),
    Stop(StopRequest),
    Topology(Topology),
    Progress(Progress),
    Error(ComputeError),
    Notice(Notice),
    PhaseDone(PhaseDone),
    Finished(Finished),
}

fn check_text(what: &str, s: &str) -> Result<(), IpcError> {
    if s.len() > MAX_TEXT_BYTES {
        Err(IpcError::Decode(format!(
            "{what} is {} bytes, the maximum is {MAX_TEXT_BYTES}",
            s.len()
        )))
    } else {
        Ok(())
    }
}

fn check_load_percent(v: Option<u8>) -> Result<(), IpcError> {
    match v {
        Some(n) if !(1..=100).contains(&n) => Err(IpcError::Decode(format!(
            "load_percent {n} is out of 1-100"
        ))),
        _ => Ok(()),
    }
}

fn check_phase(p: &Phase, gpu: bool) -> Result<(), IpcError> {
    let is_gpu = p.kernel.is_gpu();
    if is_gpu != gpu || p.alt_kernel.is_some_and(|k| k.is_gpu() != gpu) {
        return Err(IpcError::Decode(
            "a plan with a gpu runs only GPU kernels, one without runs none".into(),
        ));
    }
    if (p.mode.is_gpu_only() && !is_gpu)
        || (is_gpu && matches!(p.mode, LoadMode::Variable | LoadMode::Light))
    {
        return Err(IpcError::Decode(
            "ramp, alternate and pause_resume are GPU only; variable and light are CPU only".into(),
        ));
    }
    if is_gpu && p.iterations.is_some() {
        return Err(IpcError::Decode("GPU phases refuse iterations".into()));
    }
    if p.duration_s == 0 {
        return Err(IpcError::Decode("duration_s is 0".into()));
    }
    match p.windows {
        Some(w) => {
            if !(1..=30).contains(&w) {
                return Err(IpcError::Decode(format!("windows {w} is out of 1-30")));
            }
            if !is_gpu || p.mode != LoadMode::Steady || p.alt_kernel.is_some() {
                return Err(IpcError::Decode(
                    "windows need a steady GPU phase without alt_kernel".into(),
                ));
            }
            if p.duration_s < GPU_BENCH_WARMUP_S + u32::from(w) {
                return Err(IpcError::Decode(
                    "duration_s is shorter than the warm-up plus the windows".into(),
                ));
            }
        }
        None if p.kernel.is_gpu_bench_only() => {
            return Err(IpcError::Decode(
                "fill, texture and overdraw need windows".into(),
            ))
        }
        None => {}
    }
    match p.per_core_s {
        Some(s) if !(1..=3600).contains(&s) => {
            return Err(IpcError::Decode(format!("per_core_s {s} is out of 1-3600")))
        }
        None if p.placement == Placement::CoreCycle => {
            return Err(IpcError::Decode("core_cycle needs per_core_s".into()))
        }
        _ => {}
    }
    if let Some(cores) = &p.cores {
        check_len("cores", cores.len(), MAX_LOGICAL)?;
    }
    if (p.kernel == KernelId::K10) == p.patterns.is_empty() {
        return Err(IpcError::Decode(
            "patterns are required for k10 and refused for the other kernels".into(),
        ));
    }
    if let Some(n) = p.iterations {
        if !(1..=MAX_ITERATIONS).contains(&n) {
            return Err(IpcError::Decode(format!(
                "iterations {n} is out of 1-{MAX_ITERATIONS}"
            )));
        }
        if p.mode != LoadMode::Steady || p.placement == Placement::CoreCycle {
            return Err(IpcError::Decode(
                "iterations need the steady mode and no core_cycle".into(),
            ));
        }
    } else if p.kernel.is_bench_only() {
        return Err(IpcError::Decode(
            "hash, compress and sort need iterations".into(),
        ));
    }
    if p.pause_before_ms > MAX_PAUSE_MS {
        return Err(IpcError::Decode(format!(
            "pause_before_ms {} is over {MAX_PAUSE_MS}",
            p.pause_before_ms
        )));
    }
    if p.size == DataSize::Fixed
        && !(p.kernel.is_bench_only()
            || matches!(p.kernel, KernelId::K2 | KernelId::K5 | KernelId::K7))
    {
        return Err(IpcError::Decode(
            "the fixed size is only for k2, k5, k7, hash, compress and sort".into(),
        ));
    }
    Ok(())
}

impl LoadMessage {
    /// Rejects lists, texts and plans over their limits and non-finite numbers. The
    /// receiver calls it after decoding; the decoder alone lets them through.
    pub fn validate(&self) -> Result<(), IpcError> {
        match self {
            Self::Hello(h) => {
                check_text("version", &h.version)?;
                h.shader_digest
                    .as_deref()
                    .map_or(Ok(()), |d| check_text("shader_digest", d))
            }
            Self::Run(r) => {
                let plan = &r.plan;
                check_len("phases", plan.phases.len(), MAX_PHASES)?;
                if plan.phases.is_empty() {
                    return Err(IpcError::Decode("the plan has no phases".into()));
                }
                if plan.ram_bytes > MAX_RAM_BYTES {
                    return Err(IpcError::Decode("ram_bytes is over the maximum".into()));
                }
                if plan.gpu.is_some_and(|g| g.luid == 0) {
                    return Err(IpcError::Decode("gpu luid is 0".into()));
                }
                let gpu = plan.gpu.is_some();
                plan.phases.iter().try_for_each(|p| check_phase(p, gpu))?;
                if plan.total_seconds() > u64::from(MAX_PLAN_SECONDS) {
                    return Err(IpcError::Decode(
                        "the plan lasts more than the maximum".into(),
                    ));
                }
                Ok(())
            }
            Self::Topology(t) => {
                check_len("logical", t.logical.len(), MAX_LOGICAL)?;
                check_text("vendor", &t.vendor)?;
                check_text("brand", &t.brand)
            }
            Self::Progress(p) => {
                check_len("cores", p.cores.len(), MAX_LOGICAL)?;
                check_load_percent(p.load_percent)?;
                match p.rate {
                    Some(r) if !r.is_finite() || r < 0.0 => {
                        Err(IpcError::Decode("rate is not a finite non-negative".into()))
                    }
                    _ => Ok(()),
                }
            }
            Self::Notice(n) => check_text("code", &n.code),
            Self::PhaseDone(d) => {
                check_len("workers", d.workers.len(), MAX_LOGICAL)?;
                check_len("rates", d.rates.len(), 64)?;
                if d.rates.iter().any(|r| !r.is_finite() || *r < 0.0) {
                    return Err(IpcError::Decode(
                        "rates must be finite and non-negative".into(),
                    ));
                }
                d.skipped
                    .as_deref()
                    .map_or(Ok(()), |s| check_text("skipped", s))
            }
            Self::Error(e) => check_load_percent(e.load_percent),
            Self::Stop(_) | Self::Finished(_) => Ok(()),
        }
    }
}

/// True when the peer speaks the same load protocol version.
pub fn load_compatible(hello: &LoadHello) -> bool {
    hello.protocol_version == LOAD_PROTOCOL_VERSION
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{encode_frame_of, FrameDecoder};

    #[test]
    fn sse2_is_always_detected() {
        #[cfg(target_arch = "x86_64")]
        assert_eq!(detected_isa().last(), Some(&Isa::Sse2));
    }

    fn round_trip(msg: LoadMessage) {
        let frame = encode_frame_of(&msg).unwrap();
        let mut decoder = FrameDecoder::new();
        decoder.push(&frame).unwrap();
        assert_eq!(decoder.next_of::<LoadMessage>().unwrap(), Some(msg));
    }

    fn phase() -> Phase {
        Phase {
            kernel: KernelId::K1,
            alt_kernel: None,
            isa: Isa::Avx2,
            size: DataSize::L1,
            mode: LoadMode::Steady,
            placement: Placement::AllLogical,
            duration_s: 60,
            per_core_s: None,
            both_smt: false,
            cores: None,
            patterns: vec![],
            stop_on_error: true,
            iterations: None,
            pause_before_ms: 0,
            windows: None,
        }
    }

    fn plan(phases: Vec<Phase>) -> LoadMessage {
        LoadMessage::Run(RunRequest {
            plan: Plan {
                seed: 7,
                ram_bytes: 1 << 30,
                phases,
                gpu: None,
            },
        })
    }

    fn progress() -> Progress {
        Progress {
            phase: 0,
            phase_elapsed_ms: 1500,
            elapsed_ms: 2500,
            checks: 10,
            errors: 0,
            current_core: Some(1),
            cores: vec![CoreProgress {
                core: 0,
                state: CoreState::Passed,
            }],
            memory_bytes: 4096,
            rate: Some(1.5e6),
            load_percent: None,
        }
    }

    fn topology() -> Topology {
        Topology {
            logical: vec![LogicalCpu {
                index: 0,
                group: 0,
                number: 0,
                core: 0,
                core_index: 0,
                efficiency_class: 0,
                llc: 0,
                parked: false,
                apic_id: None,
            }],
            caches: CacheSizes {
                l1d_bytes: 32768,
                l2_bytes: 1 << 20,
                l2_shared_by: 2,
                l3_bytes: 32 << 20,
                l3_total_bytes: 64 << 20,
            },
            hypervisor: false,
            vendor: "AuthenticAMD".into(),
            brand: "AMD Ryzen".into(),
        }
    }

    fn hello() -> LoadHello {
        LoadHello {
            protocol_version: LOAD_PROTOCOL_VERSION,
            version: "0.6.0".into(),
            isa: vec![Isa::Avx2, Isa::Sse2],
            shader_digest: None,
        }
    }

    #[test]
    fn every_message_round_trips() {
        let mut p = phase();
        p.alt_kernel = Some(KernelId::K2);
        p.placement = Placement::CoreCycle;
        p.per_core_s = Some(10);
        p.cores = Some(vec![0, 2]);
        round_trip(LoadMessage::Hello(hello()));
        round_trip(plan(vec![p]));
        round_trip(LoadMessage::Stop(StopRequest {}));
        round_trip(LoadMessage::Topology(topology()));
        round_trip(LoadMessage::Progress(progress()));
        round_trip(LoadMessage::Error(ComputeError {
            phase: 1,
            kernel: KernelId::K10,
            isa: Isa::Sse2,
            kind: ErrorKind::ReferenceDisagreement,
            logical: Some(3),
            core: None,
            iteration: 9,
            expected: 1,
            actual: 2,
            seed: 3,
            load_percent: Some(55),
        }));
        round_trip(LoadMessage::Notice(Notice {
            phase: 0,
            code: "ram_reduced".into(),
            value: Some(5),
        }));
        round_trip(LoadMessage::PhaseDone(PhaseDone {
            phase: 0,
            checks: 1,
            errors: 0,
            duration_ms: 1000,
            skipped: Some("isa".into()),
            work_ms: Some(900),
            workers: vec![],
            rates: vec![],
        }));
        round_trip(LoadMessage::Finished(Finished {
            reason: FinishReason::FirstError,
            checks: 1,
            errors: 1,
        }));
    }

    #[test]
    fn absent_values_are_nil_keys() {
        let v = serde_json::to_value(phase()).unwrap();
        assert!(v.as_object().unwrap().contains_key("alt_kernel"));
        assert!(v["alt_kernel"].is_null());
        assert!(v["per_core_s"].is_null());
    }

    #[test]
    fn enums_are_snake_case_strings() {
        assert_eq!(serde_json::to_value(KernelId::K10).unwrap(), "k10");
        assert_eq!(
            serde_json::to_value(Placement::CoreCycle).unwrap(),
            "core_cycle"
        );
        assert_eq!(
            serde_json::to_value(RamPattern::MovingInversions).unwrap(),
            "moving_inversions"
        );
        assert_eq!(
            serde_json::to_value(FinishReason::FirstError).unwrap(),
            "first_error"
        );
    }

    #[test]
    fn oversized_plan_is_rejected() {
        assert!(plan(vec![phase(); MAX_PHASES]).validate().is_ok());
        assert!(plan(vec![phase(); MAX_PHASES + 1]).validate().is_err());
        assert!(plan(vec![]).validate().is_err());
        let mut long = phase();
        long.duration_s = MAX_PLAN_SECONDS;
        assert!(plan(vec![long.clone()]).validate().is_ok());
        assert!(plan(vec![long, phase()]).validate().is_err());
        let mut zero = phase();
        zero.duration_s = 0;
        assert!(plan(vec![zero]).validate().is_err());
        let mut ram = plan(vec![phase()]);
        if let LoadMessage::Run(r) = &mut ram {
            r.plan.ram_bytes = MAX_RAM_BYTES + 1;
        }
        assert!(ram.validate().is_err());

        let mut cores = phase();
        cores.cores = Some(vec![0; MAX_LOGICAL + 1]);
        assert!(plan(vec![cores]).validate().is_err());
        let mut t = topology();
        t.logical = vec![t.logical[0].clone(); MAX_LOGICAL + 1];
        assert!(LoadMessage::Topology(t).validate().is_err());
        let mut p = progress();
        p.cores = vec![p.cores[0].clone(); MAX_LOGICAL + 1];
        assert!(LoadMessage::Progress(p).validate().is_err());
    }

    #[test]
    fn core_cycle_needs_per_core_seconds() {
        let mut p = phase();
        p.placement = Placement::CoreCycle;
        assert!(plan(vec![p.clone()]).validate().is_err());
        for (s, ok) in [(0, false), (1, true), (3600, true), (3601, false)] {
            p.per_core_s = Some(s);
            assert_eq!(plan(vec![p.clone()]).validate().is_ok(), ok, "{s}");
        }
        let mut q = phase();
        q.per_core_s = Some(0);
        assert!(plan(vec![q]).validate().is_err());
    }

    #[test]
    fn k10_needs_patterns_and_others_refuse_them() {
        let mut p = phase();
        p.patterns = vec![RamPattern::Random];
        assert!(plan(vec![p.clone()]).validate().is_err());
        p.kernel = KernelId::K10;
        assert!(plan(vec![p.clone()]).validate().is_ok());
        p.patterns = vec![];
        assert!(plan(vec![p]).validate().is_err());
    }

    fn bench_phase() -> Phase {
        let mut p = phase();
        p.kernel = KernelId::Hash;
        p.size = DataSize::Fixed;
        p.iterations = Some(1000);
        p.pause_before_ms = 2000;
        p
    }

    #[test]
    fn bench_phase_round_trips() {
        let mut sort = bench_phase();
        sort.kernel = KernelId::Sort;
        round_trip(plan(vec![bench_phase(), sort]));
        assert!(plan(vec![bench_phase()]).validate().is_ok());
        assert_eq!(serde_json::to_value(KernelId::Hash).unwrap(), "hash");
        assert_eq!(
            serde_json::to_value(KernelId::Compress).unwrap(),
            "compress"
        );
        assert_eq!(serde_json::to_value(KernelId::Sort).unwrap(), "sort");
        assert_eq!(serde_json::to_value(DataSize::Fixed).unwrap(), "fixed");
        let v = serde_json::to_value(bench_phase()).unwrap();
        assert_eq!(v["iterations"], 1000);
        assert_eq!(v["pause_before_ms"], 2000);
    }

    #[test]
    fn iterations_out_of_range_is_rejected() {
        for (n, ok) in [
            (0, false),
            (1, true),
            (1_000_000_000, true),
            (1_000_000_001, false),
        ] {
            let mut p = bench_phase();
            p.iterations = Some(n);
            assert_eq!(plan(vec![p]).validate().is_ok(), ok, "{n}");
        }
        for (ms, ok) in [(0, true), (10_000, true), (10_001, false)] {
            let mut p = bench_phase();
            p.pause_before_ms = ms;
            assert_eq!(plan(vec![p]).validate().is_ok(), ok, "{ms}");
        }
    }

    #[test]
    fn iterations_need_steady_and_no_core_cycle() {
        for mode in [LoadMode::Variable, LoadMode::Light] {
            let mut p = bench_phase();
            p.mode = mode;
            assert!(plan(vec![p]).validate().is_err());
        }
        let mut p = bench_phase();
        p.placement = Placement::CoreCycle;
        p.per_core_s = Some(5);
        assert!(plan(vec![p]).validate().is_err());
    }

    #[test]
    fn bench_kernels_need_iterations() {
        for k in [KernelId::Hash, KernelId::Compress, KernelId::Sort] {
            let mut p = bench_phase();
            p.kernel = k;
            assert!(plan(vec![p.clone()]).validate().is_ok());
            p.iterations = None;
            assert!(plan(vec![p]).validate().is_err(), "{k:?}");
        }
    }

    #[test]
    fn fixed_size_only_for_bench_kernels() {
        for (k, ok) in [
            (KernelId::K2, true),
            (KernelId::K5, true),
            (KernelId::K7, true),
            (KernelId::Hash, true),
            (KernelId::K1, false),
            (KernelId::K3, false),
            (KernelId::K4, false),
            (KernelId::K8, false),
            (KernelId::K9, false),
        ] {
            let mut p = phase();
            p.kernel = k;
            p.size = DataSize::Fixed;
            p.iterations = k.is_bench_only().then_some(10);
            assert_eq!(plan(vec![p]).validate().is_ok(), ok, "{k:?}");
        }
    }

    #[test]
    fn v1_phase_without_bench_fields_still_parses() {
        let json = serde_json::json!({
            "kernel": "k1", "alt_kernel": null, "isa": "avx2", "size": "l1",
            "mode": "steady", "placement": "all_logical", "duration_s": 60,
            "per_core_s": null, "both_smt": false, "cores": null,
            "patterns": [], "stop_on_error": true
        });
        let p: Phase = serde_json::from_value(json).unwrap();
        assert_eq!(p.iterations, None);
        assert_eq!(p.pause_before_ms, 0);
    }

    #[test]
    fn phase_done_without_work_ms_parses() {
        let json = serde_json::json!({
            "phase": 0, "checks": 1, "errors": 0, "duration_ms": 5, "skipped": null
        });
        let d: PhaseDone = serde_json::from_value(json).unwrap();
        assert_eq!(d.work_ms, None);
    }

    fn phase_done(workers: Vec<WorkerDone>) -> PhaseDone {
        PhaseDone {
            phase: 0,
            checks: 2,
            errors: 0,
            duration_ms: 5,
            skipped: None,
            work_ms: Some(4),
            workers,
            rates: vec![],
        }
    }

    #[test]
    fn phase_done_workers_round_trip() {
        let w = WorkerDone {
            logical: 3,
            iterations: 1,
            work_ms: 4,
        };
        round_trip(LoadMessage::PhaseDone(phase_done(vec![w.clone(), w])));
    }

    #[test]
    fn phase_done_without_workers_parses() {
        let json = serde_json::json!({
            "phase": 0, "checks": 1, "errors": 0, "duration_ms": 5, "skipped": null,
            "work_ms": null
        });
        let d: PhaseDone = serde_json::from_value(json).unwrap();
        assert!(d.workers.is_empty());
    }

    #[test]
    fn too_many_workers_are_rejected() {
        let w = WorkerDone {
            logical: 0,
            iterations: 1,
            work_ms: 1,
        };
        let ok = LoadMessage::PhaseDone(phase_done(vec![w.clone(); MAX_LOGICAL]));
        assert!(ok.validate().is_ok());
        let bad = LoadMessage::PhaseDone(phase_done(vec![w; MAX_LOGICAL + 1]));
        assert!(bad.validate().is_err());
    }

    #[test]
    fn long_text_is_rejected() {
        let long = "x".repeat(MAX_TEXT_BYTES + 1);
        let ok = "x".repeat(MAX_TEXT_BYTES);
        let notice = |code: String| {
            LoadMessage::Notice(Notice {
                phase: 0,
                code,
                value: None,
            })
        };
        assert!(notice(ok).validate().is_ok());
        assert!(notice(long.clone()).validate().is_err());
        let mut h = hello();
        h.version = long.clone();
        assert!(LoadMessage::Hello(h).validate().is_err());
        let mut t = topology();
        t.brand = long.clone();
        assert!(LoadMessage::Topology(t).validate().is_err());
        let done = LoadMessage::PhaseDone(PhaseDone {
            phase: 0,
            checks: 0,
            errors: 0,
            duration_ms: 0,
            skipped: Some(long),
            work_ms: None,
            workers: vec![],
            rates: vec![],
        });
        assert!(done.validate().is_err());
    }

    #[test]
    fn non_finite_values_are_rejected() {
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            let mut p = progress();
            p.rate = Some(bad);
            assert!(LoadMessage::Progress(p).validate().is_err(), "{bad}");
        }
        let mut p = progress();
        p.rate = None;
        assert!(LoadMessage::Progress(p).validate().is_ok());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let json = serde_json::json!({
            "type": "notice",
            "body": {"phase": 1, "code": "c", "value": null, "future": 1},
            "extra": true
        });
        let msg: LoadMessage = serde_json::from_value(json).unwrap();
        assert_eq!(
            msg,
            LoadMessage::Notice(Notice {
                phase: 1,
                code: "c".into(),
                value: None
            })
        );
    }

    #[test]
    fn hello_compatibility() {
        assert!(load_compatible(&hello()));
        let mut h = hello();
        h.protocol_version = 4;
        assert!(!load_compatible(&h));
        h.protocol_version = 5;
        assert!(load_compatible(&h));
    }

    fn gpu_phase() -> Phase {
        let mut p = phase();
        p.kernel = KernelId::S1;
        p
    }

    fn gpu_plan(phases: Vec<Phase>) -> LoadMessage {
        let mut m = plan(phases);
        if let LoadMessage::Run(r) = &mut m {
            r.plan.gpu = Some(GpuTarget {
                luid: 0x17e99,
                integrated: false,
            });
        }
        m
    }

    #[test]
    fn gpu_kernels_and_modes_are_snake_case() {
        assert_eq!(serde_json::to_value(KernelId::S4).unwrap(), "s4");
        assert_eq!(
            serde_json::to_value(LoadMode::PauseResume).unwrap(),
            "pause_resume"
        );
        assert_eq!(serde_json::to_value(LoadMode::Ramp).unwrap(), "ramp");
        assert_eq!(
            serde_json::to_value(ErrorKind::DeviceLost).unwrap(),
            "device_lost"
        );
        round_trip(gpu_plan(vec![gpu_phase()]));
    }

    #[test]
    fn gpu_plan_needs_gpu_kernels() {
        assert!(gpu_plan(vec![gpu_phase()]).validate().is_ok());
        assert!(gpu_plan(vec![phase()]).validate().is_err());
        assert!(plan(vec![gpu_phase()]).validate().is_err());
    }

    #[test]
    fn gpu_alt_kernel_must_be_gpu() {
        let mut p = gpu_phase();
        p.kernel = KernelId::S5;
        p.alt_kernel = Some(KernelId::S1);
        assert!(gpu_plan(vec![p.clone()]).validate().is_ok());
        p.alt_kernel = Some(KernelId::K5);
        assert!(gpu_plan(vec![p]).validate().is_err());
    }

    #[test]
    fn gpu_modes_only_on_gpu_kernels() {
        for mode in [LoadMode::Ramp, LoadMode::Alternate, LoadMode::PauseResume] {
            let mut g = gpu_phase();
            g.mode = mode;
            assert!(gpu_plan(vec![g]).validate().is_ok(), "{mode:?}");
            let mut c = phase();
            c.kernel = KernelId::K2;
            c.mode = mode;
            assert!(plan(vec![c]).validate().is_err(), "{mode:?}");
        }
        for mode in [LoadMode::Variable, LoadMode::Light] {
            let mut g = gpu_phase();
            g.mode = mode;
            assert!(gpu_plan(vec![g]).validate().is_err(), "{mode:?}");
        }
    }

    #[test]
    fn gpu_phases_refuse_iterations() {
        let mut p = gpu_phase();
        p.iterations = Some(10);
        assert!(gpu_plan(vec![p]).validate().is_err());
    }

    #[test]
    fn zero_luid_is_rejected() {
        let mut m = gpu_plan(vec![gpu_phase()]);
        if let LoadMessage::Run(r) = &mut m {
            r.plan.gpu.as_mut().unwrap().luid = 0;
        }
        assert!(m.validate().is_err());
    }

    #[test]
    fn load_percent_out_of_range_is_rejected() {
        for (v, ok) in [(0, false), (1, true), (100, true), (101, false)] {
            let mut p = progress();
            p.load_percent = Some(v);
            assert_eq!(LoadMessage::Progress(p).validate().is_ok(), ok, "{v}");
            let e = LoadMessage::Error(ComputeError {
                phase: 0,
                kernel: KernelId::S1,
                isa: Isa::Sse2,
                kind: ErrorKind::Mismatch,
                logical: None,
                core: None,
                iteration: 1,
                expected: 1,
                actual: 2,
                seed: 3,
                load_percent: Some(v),
            });
            assert_eq!(e.validate().is_ok(), ok, "{v}");
        }
    }

    #[test]
    fn v3_run_without_gpu_still_decodes() {
        let json = serde_json::json!({"seed": 1, "ram_bytes": 0, "phases": []});
        let p: Plan = serde_json::from_value(json).unwrap();
        assert_eq!(p.gpu, None);
        let json = serde_json::json!({
            "phase": 0, "phase_elapsed_ms": 1, "elapsed_ms": 1, "checks": 0, "errors": 0,
            "current_core": null, "cores": [], "memory_bytes": 0, "rate": null
        });
        let pr: Progress = serde_json::from_value(json).unwrap();
        assert_eq!(pr.load_percent, None);
    }

    fn gpu_bench_phase(kernel: KernelId) -> Phase {
        let mut p = gpu_phase();
        p.kernel = kernel;
        p.duration_s = 30;
        p.windows = Some(5);
        p
    }

    fn valid(p: Phase) -> bool {
        gpu_plan(vec![p]).validate().is_ok()
    }

    #[test]
    fn bench_gpu_kernels_are_snake_case() {
        assert_eq!(serde_json::to_value(KernelId::S3).unwrap(), "s3");
        assert_eq!(serde_json::to_value(KernelId::Fill).unwrap(), "fill");
        assert_eq!(serde_json::to_value(KernelId::Texture).unwrap(), "texture");
        assert_eq!(
            serde_json::to_value(KernelId::Overdraw).unwrap(),
            "overdraw"
        );
        for k in [
            KernelId::S3,
            KernelId::Fill,
            KernelId::Texture,
            KernelId::Overdraw,
        ] {
            assert!(k.is_gpu(), "{k:?}");
        }
        assert!(!KernelId::S3.is_gpu_bench_only());
        assert!(KernelId::Fill.is_gpu_bench_only());
        assert!(!KernelId::S1.is_gpu_bench_only());
    }

    #[test]
    fn gpu_bench_phase_round_trips() {
        round_trip(gpu_plan(vec![
            gpu_bench_phase(KernelId::S1),
            gpu_bench_phase(KernelId::Fill),
        ]));
        assert!(valid(gpu_bench_phase(KernelId::Overdraw)));
        round_trip(LoadMessage::PhaseDone(PhaseDone {
            phase: 1,
            checks: 0,
            errors: 0,
            duration_ms: 9000,
            skipped: None,
            work_ms: None,
            workers: vec![],
            rates: vec![1.5e9, 2.5e9],
        }));
        let mut h = hello();
        h.shader_digest = Some("0123456789abcdef".into());
        round_trip(LoadMessage::Hello(h));
    }

    #[test]
    fn windows_out_of_range_is_rejected() {
        for (n, ok) in [(0, false), (1, true), (30, true), (31, false)] {
            let mut p = gpu_bench_phase(KernelId::S1);
            p.windows = Some(n);
            p.duration_s = 100;
            assert_eq!(valid(p), ok, "{n}");
        }
    }

    #[test]
    fn windows_need_a_steady_gpu_phase_without_alt_kernel() {
        let mut p = gpu_bench_phase(KernelId::S1);
        p.windows = Some(5);
        let mut c = phase();
        c.kernel = KernelId::K2;
        c.windows = Some(5);
        assert!(plan(vec![c]).validate().is_err());
        let mut r = p.clone();
        r.mode = LoadMode::Ramp;
        assert!(!valid(r));
        let mut a = p.clone();
        a.alt_kernel = Some(KernelId::S1);
        assert!(!valid(a));
        assert!(valid(p));
    }

    #[test]
    fn windows_need_the_warmup() {
        let mut p = gpu_bench_phase(KernelId::S1);
        p.duration_s = 8;
        assert!(!valid(p.clone()));
        p.duration_s = GPU_BENCH_WARMUP_S + 5;
        assert_eq!(p.duration_s, 9);
        assert!(valid(p));
    }

    #[test]
    fn bench_only_gpu_kernels_need_windows() {
        for k in [KernelId::Fill, KernelId::Texture, KernelId::Overdraw] {
            let mut p = gpu_bench_phase(k);
            assert!(valid(p.clone()), "{k:?}");
            p.windows = None;
            assert!(!valid(p), "{k:?}");
        }
        let mut s3 = gpu_bench_phase(KernelId::S3);
        s3.windows = None;
        assert!(valid(s3));
    }

    #[test]
    fn rates_must_be_finite_and_few() {
        let done = |rates: Vec<f64>| {
            LoadMessage::PhaseDone(PhaseDone {
                phase: 0,
                checks: 0,
                errors: 0,
                duration_ms: 1,
                skipped: None,
                work_ms: None,
                workers: vec![],
                rates,
            })
        };
        assert!(done(vec![0.0, 1.0e12]).validate().is_ok());
        assert!(done(vec![1.0; 64]).validate().is_ok());
        assert!(done(vec![f64::NAN]).validate().is_err());
        assert!(done(vec![-1.0]).validate().is_err());
        assert!(done(vec![1.0; 65]).validate().is_err());
    }

    #[test]
    fn v4_messages_still_decode() {
        let json = serde_json::json!({
            "kernel": "s1", "alt_kernel": null, "isa": "sse2", "size": "auto",
            "mode": "steady", "placement": "all_logical", "duration_s": 5,
            "per_core_s": null, "both_smt": false, "cores": null, "patterns": [],
            "stop_on_error": true
        });
        let p: Phase = serde_json::from_value(json).unwrap();
        assert_eq!(p.windows, None);
        let json = serde_json::json!({
            "phase": 0, "checks": 0, "errors": 0, "duration_ms": 1, "skipped": null
        });
        let d: PhaseDone = serde_json::from_value(json).unwrap();
        assert!(d.rates.is_empty());
        let json = serde_json::json!({"protocol_version": 4, "version": "x", "isa": []});
        let h: LoadHello = serde_json::from_value(json).unwrap();
        assert_eq!(h.shader_digest, None);
    }
}
