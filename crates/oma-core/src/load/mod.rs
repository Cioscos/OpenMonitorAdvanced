//! Stress-test catalogue, profiles and plan builder (pure; no Windows code). The plan
//! types themselves live in `oma-ipc::load`.

mod catalog;
mod outcome;
mod plan;
pub(crate) mod run;
mod sensors;
mod session;
mod thermal;

pub use catalog::catalog_json;
pub use outcome::{decide, Outcome, OutcomeFacts, VerdictKey, NOTHING_RAN};
pub use plan::{
    build_plan, core_order, presets, ram_budget, BuildError, BuildInput, Component, Custom,
    ModeEdit, Objective, Preset, RetryCore, StartRequest, ThreadChoice, KEEP_FREE_BYTES,
};
pub use run::{Action, Clock, PhaseInfo, RunConfig, RunController, RunState, RunStatus, WheaEvent};
pub use sensors::{read_sample, resolve_cpu_sensors, CpuSensorIds, SensorSample};
pub use session::{
    is_session_file_name, is_session_id, parse_journal, parse_session, prune, session_file_name,
    summary, CoreResult, ErrorRecord, FormatError, Journal, OutcomeDetail, PhaseResult, Sample,
    Session, SessionEvent, SessionSummary, Stats, WheaCounts, FORMAT, KEEP_SESSIONS, MAX_ERRORS,
};
pub use thermal::{cpu_stop_threshold, ThermalEvent, ThermalGuard};
