//! App <-> `oma-load.exe` protocol (version 1): the messages the app and the stress-test
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
pub const LOAD_PROTOCOL_VERSION: u32 = 1;

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
/// Maximum [`Plan::ram_bytes`].
pub const MAX_RAM_BYTES: u64 = 1 << 40;

/// Handshake, sent by both sides. The app sends an empty `isa`; the process lists the
/// instruction sets it can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadHello {
    pub protocol_version: u32,
    pub version: String,
    pub isa: Vec<Isa>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSize {
    L1,
    L2,
    L3,
    Ram,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadMode {
    Steady,
    Variable,
    Light,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub seed: u64,
    pub ram_bytes: u64,
    pub phases: Vec<Phase>,
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
/// over the last second.
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub phase: u32,
    pub code: String,
    pub value: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseDone {
    pub phase: u32,
    pub checks: u64,
    pub errors: u64,
    pub duration_ms: u64,
    pub skipped: Option<String>,
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

fn check_phase(p: &Phase) -> Result<(), IpcError> {
    if p.duration_s == 0 {
        return Err(IpcError::Decode("duration_s is 0".into()));
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
    Ok(())
}

impl LoadMessage {
    /// Rejects lists, texts and plans over their limits and non-finite numbers. The
    /// receiver calls it after decoding; the decoder alone lets them through.
    pub fn validate(&self) -> Result<(), IpcError> {
        match self {
            Self::Hello(h) => check_text("version", &h.version),
            Self::Run(r) => {
                let plan = &r.plan;
                check_len("phases", plan.phases.len(), MAX_PHASES)?;
                if plan.phases.is_empty() {
                    return Err(IpcError::Decode("the plan has no phases".into()));
                }
                if plan.ram_bytes > MAX_RAM_BYTES {
                    return Err(IpcError::Decode("ram_bytes is over the maximum".into()));
                }
                plan.phases.iter().try_for_each(check_phase)?;
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
                match p.rate {
                    Some(r) if !r.is_finite() || r < 0.0 => {
                        Err(IpcError::Decode("rate is not a finite non-negative".into()))
                    }
                    _ => Ok(()),
                }
            }
            Self::Notice(n) => check_text("code", &n.code),
            Self::PhaseDone(d) => d
                .skipped
                .as_deref()
                .map_or(Ok(()), |s| check_text("skipped", s)),
            Self::Stop(_) | Self::Error(_) | Self::Finished(_) => Ok(()),
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
        }
    }

    fn plan(phases: Vec<Phase>) -> LoadMessage {
        LoadMessage::Run(RunRequest {
            plan: Plan {
                seed: 7,
                ram_bytes: 1 << 30,
                phases,
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
        h.protocol_version = 2;
        assert!(!load_compatible(&h));
    }
}
