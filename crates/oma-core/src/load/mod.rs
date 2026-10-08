//! Stress-test catalogue, profiles and plan builder (pure; no Windows code). The plan
//! types themselves live in `oma-ipc::load`.

mod catalog;
mod outcome;
mod plan;
pub(crate) mod run;
mod sensors;
mod session;
mod stability;
mod thermal;

pub use catalog::catalog_json;
pub use outcome::{decide, Outcome, OutcomeFacts, VerdictKey, NOTHING_RAN};
pub use plan::{
    build_plan, core_order, disk_reserve, estimated_writes, presets, ram_budget, stress_file_bytes,
    BuildError, BuildInput, Component, Custom, DiskPlanInput, DiskStart, ModeEdit, Objective,
    Preset, RetryCore, StartRequest, ThreadChoice, KEEP_FREE_BYTES,
};
pub use run::{
    Action, Clock, DiskStatus, PhaseInfo, RunConfig, RunController, RunState, RunStatus, WheaEvent,
    LOAD_EXIT_DEVICE_LOST, LOAD_EXIT_IO,
};
pub use sensors::{
    read_disk_sample, read_gpu_sample, read_sample, resolve_cpu_sensors, resolve_disk_sensors,
    resolve_gpu_sensors, CpuSensorIds, DiskSensorIds, GpuSensorIds, SensorSample,
};
pub use session::{
    is_session_file_name, is_session_id, parse_journal, parse_session, prune, session_file_name,
    summary, CoreResult, DiskSession, ErrorRecord, FormatError, Journal, OutcomeDetail,
    PhaseResult, Sample, Session, SessionEvent, SessionSummary, SlcResult, Stats, WheaCounts,
    FORMAT, KEEP_SESSIONS, MAX_ERRORS,
};
pub use stability::{StabilityMeter, MIN_STABILITY, WARMUP_MS, WINDOW_MS};
pub use thermal::{
    cpu_stop_threshold, disk_stop_threshold, gpu_stop_threshold, ThermalEvent, ThermalGuard,
};
