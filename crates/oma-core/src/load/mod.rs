//! Stress-test catalogue, profiles and plan builder (pure; no Windows code). The plan
//! types themselves live in `oma-ipc::load`.

mod catalog;
mod outcome;
mod plan;
mod run;
mod sensors;
mod session;
mod thermal;

pub use catalog::catalog_json;
pub use outcome::{decide, Outcome, OutcomeFacts, VerdictKey};
pub use plan::{
    build_plan, core_order, presets, ram_budget, BuildError, BuildInput, Component, Custom,
    ModeEdit, Objective, Preset, RetryCore, StartRequest, ThreadChoice,
};
pub use run::{Action, Clock, PhaseInfo, RunConfig, RunController, RunState, RunStatus, WheaEvent};
pub use sensors::{read_sample, resolve_cpu_sensors, CpuSensorIds, SensorSample};
pub use session::{
    is_session_file_name, is_session_id, parse_journal, parse_session, prune, session_file_name,
    summary, CoreResult, ErrorRecord, FormatError, Journal, OutcomeDetail, PhaseResult, Sample,
    Session, SessionEvent, SessionSummary, Stats, WheaCounts, KEEP_SESSIONS, MAX_ERRORS,
};
pub use thermal::{cpu_stop_threshold, ThermalEvent, ThermalGuard};
