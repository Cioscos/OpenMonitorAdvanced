//! The stress session controller: a pure state machine driven by load messages,
//! sensor samples, WHEA results and a clock (plan A4, DA5, DA13-DA16). It performs no
//! I/O; the runner executes the returned `Action`s.

use std::collections::{BTreeMap, BTreeSet};

use oma_ipc::load::{
    ComputeError, CoreProgress, CoreState, ErrorKind, FinishReason, Isa, KernelId, LoadMessage,
    LoadMode, Placement,
};
use serde::Serialize;

use super::outcome::{decide, Outcome, OutcomeFacts};
use super::plan::{Component, Objective, Preset};
use super::sensors::SensorSample;
use super::session::SlcResult;
use super::session::{
    CoreResult, ErrorRecord, Journal, OutcomeDetail, PhaseResult, Sample, Session, SessionEvent,
    Stats, FORMAT, MAX_ERRORS,
};
use super::stability::StabilityMeter;
use super::thermal::{ThermalEvent, ThermalGuard};

const SAMPLE_EVERY_MS: u64 = 5_000;
const JOURNAL_EVERY_MS: u64 = 30_000;
const SAVE_EVERY_MS: u64 = 60_000;
const WHEA_EVERY_MS: u64 = 5_000;
pub(crate) const SILENT_PIPE_MS: u64 = 5_000;
const STOP_GRACE_MS: u64 = 3_000;
const SLEEP_JUMP_MS: u64 = 1_000;
const STATUS_EVENTS: usize = 200;
const MAX_EVENTS: usize = 1_000;
const FINAL_POLL_MS: u64 = 2_000;
const TEMP_MISSING_MS: u64 = 10_000;
/// How long a run may outlast its plan before it counts as hung.
pub(crate) const OVERRUN_MS: u64 = 120_000;
/// `oma-load` exits with this code on an invalid command line or message (`EXIT_USAGE`).
const LOAD_EXIT_USAGE: i32 = 1;
/// `oma-load` exits with this code when the GPU was lost (`EXIT_DEVICE_LOST`).
pub const LOAD_EXIT_DEVICE_LOST: i32 = 4;
/// `oma-load` exits with this code after a persistent I/O error (`EXIT_IO`).
pub const LOAD_EXIT_IO: i32 = 5;
/// How far before an `slc_cliff` notice a hot sample still makes it a thermal suspect (DC8).
const SLC_HOT_WINDOW_MS: u64 = 10_000;
/// A sample this close to the stop threshold counts as hot (DC8).
const SLC_HOT_MARGIN_C: f64 = 5.0;
const INVALID_PLAN: &str = "performance.start.invalid_plan";
const NO_GPU: &str = "performance.start.no_gpu";
const GPU_ERROR: &str = "performance.start.gpu_error";
const ACCESS_DENIED: &str = "performance.start.access_denied";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub mono_ms: u64,
    /// Unix time, only to show the time of day.
    pub wall_ms: i64,
    /// Time spent asleep (plan DA15).
    pub asleep_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WheaEvent {
    pub record_id: u64,
    pub event_id: u32,
    pub apic_id: Option<u32>,
    pub time_utc: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunConfig {
    pub threshold_c: f64,
    pub thermal_stop: bool,
    pub service_available: bool,
    /// Core numbers (plan DA4).
    pub cores: Vec<u32>,
    pub apic_to_core: BTreeMap<u32, u32>,
    /// Newest WHEA record already in the log at the start (older history is ignored).
    pub whea_after: Option<u64>,
    /// The newest record could not be read at the start: the first poll that works only
    /// sets the baseline, so the history of the log is never counted.
    pub whea_baseline_missing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SendStop,
    Kill,
    WriteJournal(Journal),
    SaveSession,
    DeleteJournal,
    PollWhea { after_record: Option<u64> },
    Toast,
    Finished(Outcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Idle,
    Starting,
    Running,
    Stopping,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseInfo {
    pub kernel: KernelId,
    pub mode: LoadMode,
    pub placement: Placement,
    pub duration_s: u32,
    pub isa: Isa,
}

/// Disk runs: the latest throughput and the bytes moved so far.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskStatus {
    pub read_bps: f64,
    pub write_bps: f64,
    pub written_bytes: u64,
    pub read_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStatus {
    pub state: RunState,
    pub session_id: String,
    pub component: Component,
    pub objective: Objective,
    pub preset: Preset,
    pub elapsed_ms: u64,
    pub total_ms: u64,
    pub phase_index: u32,
    pub phases: Vec<PhaseInfo>,
    pub temp_c: Option<f64>,
    pub temp_max_c: Option<f64>,
    pub stop_c: Option<f64>,
    pub power_w: Option<f64>,
    pub clock_mhz: Option<f64>,
    /// GPU `ramp` and `alternate`: the current load level.
    pub load_percent: Option<u8>,
    /// GPU runs: the throughput stability so far, 0-1.
    pub stability: Option<f64>,
    /// GPU runs: the schema device id of the GPU under test, for the UI's chart.
    pub gpu_device_id: Option<String>,
    /// Disk runs only.
    pub disk: Option<DiskStatus>,
    pub checks: u64,
    pub errors: u64,
    pub whea_corrected: u64,
    pub whea_fatal: u64,
    pub cores: Vec<CoreProgress>,
    pub current_core: Option<u32>,
    pub events: Vec<SessionEvent>,
    /// `noService`, `tempMissing`, `wheaUnreadable`, `ramReduced`, `ramInsufficient`,
    /// `pcieReplay`, `vramReduced`, `smartMissing` (disk).
    pub warnings: Vec<String>,
    pub outcome: Option<Outcome>,
}

/// Why we asked the load process to stop.
#[derive(Debug, Clone, Copy)]
enum StopCause {
    User,
    Thermal(f64),
    /// A disk data error on a phase that stops on the first error.
    FirstError,
}

/// Where the session stood when it ended; `compute_outcome` reads it.
#[derive(Debug, Clone, Copy, Default)]
struct End {
    phase: u32,
    at_ms: u64,
    temp_c: Option<f64>,
    current_core: Option<u32>,
}

#[derive(Debug, Default)]
struct Sums {
    sum: f64,
    n: u64,
    max: Option<f64>,
}

impl Sums {
    fn add(&mut self, v: Option<f64>) {
        if let Some(v) = v {
            self.sum += v;
            self.n += 1;
            self.max = Some(self.max.map_or(v, |m| m.max(v)));
        }
    }
    fn avg(&self) -> Option<f64> {
        (self.n > 0).then(|| self.sum / self.n as f64)
    }
}

pub struct RunController {
    session: Session,
    config: RunConfig,
    state: RunState,
    start_mono: u64,
    mono: u64,
    last_asleep: u64,
    last_msg_ms: u64,
    stop_deadline: Option<u64>,
    guard: Option<ThermalGuard>,
    warnings: Vec<String>,
    last_sample: SensorSample,
    /// Temperature, power, clock.
    sums: [Sums; 3],
    last_sample_push: Option<u64>,
    phase: u32,
    current_core: Option<u32>,
    checks: u64,
    journal_seen: Option<(u32, Option<u32>)>,
    last_journal_ms: u64,
    last_save_ms: u64,
    next_whea_ms: u64,
    started: bool,
    /// Set at the end; the final verdict waits for the last WHEA poll.
    ended: bool,
    final_deadline: Option<u64>,
    temp_since: Option<u64>,
    // Facts for the outcome.
    completed: bool,
    crashed: bool,
    hung: bool,
    suspended: bool,
    user_stop: bool,
    thermal_stop: Option<f64>,
    errors: u64,
    error_cores: BTreeSet<u32>,
    coreless_errors: bool,
    failed_to_start: Option<String>,
    whea_core: Option<u32>,
    end: End,
    /// GPU runs only (DG7).
    stability: Option<StabilityMeter>,
    device_lost: Option<u32>,
    /// A `Progress` arrived: the first phase began.
    phase_seen: bool,
    load_percent: Option<u8>,
    /// The first PCIe replay count read (DG14), and whether a rise was reported.
    pcie_base: Option<u32>,
    pcie_warned: bool,
    // Disk runs.
    disk_full: bool,
    /// The persistent I/O error that ends the process with `LOAD_EXIT_IO`.
    io_failed: bool,
    disk_status: Option<DiskStatus>,
    /// The last sample hot enough to explain an SLC cliff (DC8).
    hot_at: Option<u64>,
    /// Disk runs: when the current phase began, from the latest `Progress`.
    phase_start: Option<u64>,
}

fn rfc3339(wall_ms: i64) -> String {
    crate::report::utc_iso8601(wall_ms.max(0) as u64)
}

impl RunController {
    pub fn new(mut session: Session, config: RunConfig, now: Clock) -> Self {
        session.whea.last_record = config.whea_after;
        for &core in &config.cores {
            if !session.cores.iter().any(|c| c.core == core) {
                session.cores.push(CoreResult {
                    core,
                    state: CoreState::Untested,
                    first_error: None,
                });
            }
        }
        let mut c = Self {
            session,
            state: RunState::Starting,
            start_mono: now.mono_ms,
            mono: now.mono_ms,
            last_asleep: now.asleep_ms,
            last_msg_ms: now.mono_ms,
            stop_deadline: None,
            guard: (config.thermal_stop && config.service_available)
                .then(|| ThermalGuard::new(config.threshold_c)),
            warnings: Vec::new(),
            last_sample: SensorSample::default(),
            sums: Default::default(),
            last_sample_push: None,
            phase: 0,
            current_core: None,
            checks: 0,
            journal_seen: None,
            last_journal_ms: now.mono_ms,
            last_save_ms: now.mono_ms,
            next_whea_ms: now.mono_ms + WHEA_EVERY_MS,
            started: false,
            ended: false,
            final_deadline: None,
            temp_since: None,
            completed: false,
            crashed: false,
            hung: false,
            suspended: false,
            user_stop: false,
            thermal_stop: None,
            errors: 0,
            error_cores: BTreeSet::new(),
            coreless_errors: false,
            failed_to_start: None,
            whea_core: None,
            end: End::default(),
            stability: None,
            device_lost: None,
            phase_seen: false,
            load_percent: None,
            pcie_base: None,
            pcie_warned: false,
            disk_full: false,
            io_failed: false,
            disk_status: None,
            hot_at: None,
            phase_start: None,
            config,
        };
        if c.is_disk() {
            // Drive temperatures come from the app's own sensors, like the GPU's.
            c.guard = c
                .config
                .thermal_stop
                .then(|| ThermalGuard::new(c.config.threshold_c));
        } else if c.is_gpu() {
            // GPU temperatures arrive without the service (DG12).
            c.stability = Some(StabilityMeter::new(c.session.objective));
            c.guard = c
                .config
                .thermal_stop
                .then(|| ThermalGuard::new(c.config.threshold_c));
        } else if !c.config.service_available {
            c.warn("noService");
        }
        c
    }

    fn is_gpu(&self) -> bool {
        self.session.component == Component::Gpu
    }

    fn is_disk(&self) -> bool {
        self.session.component == Component::Disk
    }

    /// An error with where it happened: the clock of its core, or the GPU's for a GPU run.
    fn record(&self, e: &ComputeError) -> ErrorRecord {
        let clock = match e.core {
            Some(c) => self
                .last_sample
                .core_clock_mhz
                .get(c as usize)
                .copied()
                .flatten(),
            None if self.is_gpu() => self.last_sample.clock_mhz,
            None => None,
        };
        ErrorRecord {
            error: e.clone(),
            at_ms: self.mono - self.start_mono,
            temp_c: self.last_sample.temp_c,
            clock_mhz: clock,
        }
    }

    fn keep(&mut self, record: ErrorRecord) {
        if self.session.errors.len() < MAX_ERRORS {
            self.session.errors.push(record);
        } else {
            self.session.errors_dropped += 1;
        }
    }

    fn thermal_armed(&self) -> bool {
        self.config.thermal_stop
            && (self.config.service_available || self.is_gpu() || self.is_disk())
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn is_finished(&self) -> bool {
        self.state == RunState::Finished
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn event(&mut self, code: &str, params: &[(&str, String)]) {
        if self.session.events.len() >= MAX_EVENTS {
            self.session.events.remove(0);
            self.session.events_dropped += 1;
        }
        self.session.events.push(SessionEvent {
            at_ms: self.mono - self.start_mono,
            code: code.to_string(),
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        });
    }

    fn tick(&mut self, now: Clock) {
        if !self.ended {
            self.mono = now.mono_ms.max(self.mono);
        }
    }

    /// The core's result, added if missing. A failed core stays failed.
    fn set_core(&mut self, core: u32, state: CoreState) -> &mut CoreResult {
        let i = match self.session.cores.iter().position(|c| c.core == core) {
            Some(i) => i,
            None => {
                self.session.cores.push(CoreResult {
                    core,
                    state: CoreState::Untested,
                    first_error: None,
                });
                self.session.cores.sort_by_key(|c| c.core);
                self.session
                    .cores
                    .iter()
                    .position(|c| c.core == core)
                    .unwrap_or(0)
            }
        };
        let c = &mut self.session.cores[i];
        if c.state != CoreState::Failed {
            c.state = state;
        }
        c
    }

    fn journal(&mut self, now: Clock) -> Journal {
        self.last_journal_ms = now.mono_ms;
        self.journal_seen = Some((self.phase, self.current_core));
        Journal {
            format: FORMAT,
            session_id: self.session.id.clone(),
            plan_summary: format!(
                "{} phases, {} s",
                self.session.plan.phases.len(),
                self.session.plan.total_seconds()
            ),
            phase_index: self.phase,
            kernel: self
                .session
                .plan
                .phases
                .get(self.phase as usize)
                .map(|p| p.kernel),
            core: self.current_core,
            updated_at: rfc3339(now.wall_ms),
            clean_end: false,
            disk_folder: self.session.request.disk.as_ref().map(|d| d.folder.clone()),
        }
    }

    fn begin_stop(&mut self, cause: StopCause) -> Vec<Action> {
        match cause {
            StopCause::User => {
                self.user_stop = true;
                self.event("user_stop", &[]);
            }
            StopCause::Thermal(t) => {
                self.thermal_stop = Some(t);
                self.event("thermal_stop", &[("temp", format!("{t:.0}"))]);
            }
            StopCause::FirstError => self.event("first_error_stop", &[]),
        }
        self.state = RunState::Stopping;
        self.stop_deadline = Some(self.mono + STOP_GRACE_MS);
        vec![Action::SendStop]
    }

    fn whea_count(&self, ids: &[u32]) -> u64 {
        ids.iter()
            .map(|id| self.session.whea.by_id.get(id).copied().unwrap_or(0))
            .sum()
    }

    fn compute_outcome(&mut self) -> Outcome {
        self.session.stability = self.stability.as_ref().and_then(StabilityMeter::result);
        let s = &self.session;
        let all_skipped = !s.phases.is_empty() && s.phases.iter().all(|p| p.skipped.is_some());
        let facts = OutcomeFacts {
            failed_to_start: self.failed_to_start.clone(),
            system_crash: false,
            app_closed: false,
            crashed: self.crashed,
            device_lost: self.device_lost,
            hung: self.hung,
            errors: self.errors,
            error_cores: self.error_cores.clone(),
            coreless_errors: self.coreless_errors,
            disk_full: self.disk_full,
            thermal_stop: self.thermal_stop,
            suspended: self.suspended,
            user_stop: self.user_stop,
            whea_corrected: self.whea_count(&[17, 19]),
            completed: self.completed,
            stability: self.session.stability,
            // A disk benchmark writes without verifying: bytes moved count as running.
            nothing_ran: self.completed
                && (all_skipped || (self.checks == 0 && !self.moved_bytes())),
        };
        let (outcome, verdict) = decide(&facts);
        let e = self.end;
        let core = match (self.error_cores.len(), self.errors) {
            (1, _) => self.error_cores.iter().next().copied(),
            (_, 0) if outcome == Outcome::Marginal => self.whea_core,
            (_, 0) => e.current_core,
            _ => None,
        };
        let clock_mhz = core
            .and_then(|c| self.session.cores.iter().find(|r| r.core == c))
            .and_then(|r| r.first_error.as_ref())
            .and_then(|r| r.clock_mhz)
            .or(self.last_sample.clock_mhz);
        self.session.outcome = Some(outcome);
        self.session.outcome_detail = Some(OutcomeDetail {
            verdict: verdict.key.to_string(),
            params: verdict.params,
            phase: Some(e.phase),
            kernel: self
                .session
                .plan
                .phases
                .get(e.phase as usize)
                .map(|p| p.kernel),
            core,
            temp_c: self.thermal_stop.or(e.temp_c),
            clock_mhz,
            at_ms: Some(e.at_ms),
        });
        outcome
    }

    fn moved_bytes(&self) -> bool {
        self.session
            .disk
            .as_ref()
            .is_some_and(|d| d.read_bytes + d.written_bytes > 0)
    }

    fn update_stats(&mut self) {
        self.session.stats = Stats {
            temp_max_c: self.sums[0].max,
            temp_avg_c: self.sums[0].avg(),
            power_max_w: self.sums[1].max,
            power_avg_w: self.sums[1].avg(),
            clock_max_mhz: self.sums[2].max,
            clock_avg_mhz: self.sums[2].avg(),
        };
    }

    /// Ends the session; the verdict waits for the last WHEA poll (or 2 s).
    fn finish(&mut self, now: Clock) -> Vec<Action> {
        self.end = End {
            phase: self.phase,
            at_ms: self.mono - self.start_mono,
            temp_c: self.last_sample.temp_c,
            current_core: self.current_core,
        };
        self.ended = true;
        self.final_deadline = Some(self.mono + FINAL_POLL_MS);
        self.session.ended_at = Some(rfc3339(now.wall_ms));
        self.update_stats();
        vec![Action::PollWhea {
            after_record: self.session.whea.last_record,
        }]
    }

    fn complete(&mut self) -> Vec<Action> {
        self.final_deadline = None;
        self.state = RunState::Finished;
        let outcome = self.compute_outcome();
        vec![
            Action::SaveSession,
            Action::DeleteJournal,
            Action::Toast,
            Action::Finished(outcome),
        ]
    }

    /// The first call of any `on_*` writes the journal and the session, so a crash in
    /// the first minute can be recovered.
    fn guarded(&mut self, now: Clock, f: impl FnOnce(&mut Self) -> Vec<Action>) -> Vec<Action> {
        let mut out = vec![];
        if !self.started && !self.ended {
            self.started = true;
            self.last_save_ms = now.mono_ms;
            out.push(Action::WriteJournal(self.journal(now)));
            out.push(Action::SaveSession);
        }
        out.extend(f(self));
        out
    }

    pub fn on_load(&mut self, msg: &LoadMessage, now: Clock) -> Vec<Action> {
        self.guarded(now, |s| s.load(msg, now))
    }

    fn load(&mut self, msg: &LoadMessage, now: Clock) -> Vec<Action> {
        if self.ended {
            return vec![];
        }
        self.tick(now);
        self.last_msg_ms = now.mono_ms;
        if self.state == RunState::Starting {
            self.state = RunState::Running;
        }
        let mut out = vec![];
        match msg {
            LoadMessage::Hello(h) => self.session.load_version = Some(h.version.clone()),
            LoadMessage::Progress(p) => {
                self.phase_seen = true;
                self.load_percent = p.load_percent;
                if let Some(m) = self.stability.as_mut() {
                    // DG7: only the steady phases of the throughput kernels.
                    let counts = self
                        .session
                        .plan
                        .phases
                        .get(p.phase as usize)
                        .is_some_and(|ph| {
                            ph.mode == LoadMode::Steady
                                && matches!(
                                    ph.kernel,
                                    KernelId::S1 | KernelId::S2 | KernelId::S3 | KernelId::S5
                                )
                        });
                    let start = self.mono.saturating_sub(p.phase_elapsed_ms);
                    m.phase_started(p.phase, counts, start);
                    if let Some(r) = p.rate {
                        m.rate(r, self.mono);
                    }
                }
                // The base is fixed at the first sighting of a phase: heartbeats during an
                // open or a flush report `phase_elapsed_ms` 0 and must not move it.
                if self.phase_start.is_none() || p.phase != self.phase {
                    self.phase_start = Some(self.mono.saturating_sub(p.phase_elapsed_ms));
                }
                self.phase = p.phase;
                self.current_core = p.current_core;
                self.checks = p.checks;
                if let Some(d) = &p.disk {
                    self.disk_status = Some(DiskStatus {
                        read_bps: d.read_bps,
                        write_bps: d.write_bps,
                        written_bytes: d.written_bytes,
                        read_bytes: d.read_bytes,
                    });
                    if let Some(s) = self.session.disk.as_mut() {
                        s.read_bytes = d.read_bytes;
                        s.written_bytes = d.written_bytes;
                    }
                }
                // oma-load caps its `Error` messages but counts every error.
                self.errors = self.errors.max(p.errors);
                for c in &p.cores {
                    self.set_core(c.core, c.state);
                }
                if self.journal_seen != Some((self.phase, self.current_core)) {
                    out.push(Action::WriteJournal(self.journal(now)));
                }
            }
            LoadMessage::Error(e) if e.kind == ErrorKind::Hung => {
                self.hung = true;
                self.event("hung", &[("phase", e.phase.to_string())]);
            }
            LoadMessage::Error(e) if e.kind == ErrorKind::DeviceLost => {
                // `actual` carries the `GetDeviceRemovedReason` HRESULT.
                let code = e.actual as u32;
                self.device_lost = Some(code);
                // Kept for the result's driver code and load level, never counted as a
                // mismatch: the verdict stays `device_lost`.
                let record = self.record(e);
                self.keep(record);
                self.event(
                    "device_lost",
                    &[
                        ("phase", e.phase.to_string()),
                        ("code", format!("0x{code:08X}")),
                    ],
                );
            }
            LoadMessage::Error(e) if e.kind == ErrorKind::ReferenceInvalid => {
                // A defect of our own reference, never an error of a core.
                let code = if self.is_gpu() {
                    "reference_invalid_gpu"
                } else {
                    "reference_invalid"
                };
                self.event(code, &[("phase", e.phase.to_string())]);
            }
            LoadMessage::Error(e) => {
                self.errors += 1;
                self.io_failed |= e.kind == ErrorKind::IoError;
                let record = self.record(e);
                let core = e.core.filter(|_| e.kind == ErrorKind::Mismatch);
                self.coreless_errors |= core.is_none();
                if let Some(core) = core {
                    self.error_cores.insert(core);
                    let c = self.set_core(core, CoreState::Failed);
                    c.first_error.get_or_insert_with(|| record.clone());
                }
                self.keep(record);
                // `oma-load` goes on after a data error: stopping is our job (R8).
                if self.is_disk()
                    && self.state != RunState::Stopping
                    && !self.io_failed
                    && self
                        .session
                        .plan
                        .phases
                        .get(e.phase as usize)
                        .is_some_and(|p| p.stop_on_error)
                {
                    out.extend(self.begin_stop(StopCause::FirstError));
                }
            }
            LoadMessage::Notice(n) => {
                match n.code.as_str() {
                    "ram_reduced" => self.warn("ramReduced"),
                    "ram_insufficient" => self.warn("ramInsufficient"),
                    "vram_reduced" => self.warn("vramReduced"),
                    "gpu_missing" if !self.phase_seen => {
                        self.failed_to_start = Some(NO_GPU.into());
                    }
                    "gpu_error" if !self.phase_seen => {
                        self.failed_to_start = Some(GPU_ERROR.into());
                    }
                    "disk_full" => self.disk_full = true,
                    "access_denied" if !self.phase_seen => {
                        self.failed_to_start = Some(ACCESS_DENIED.into());
                    }
                    "slc_cliff" => {
                        let suspect = self
                            .hot_at
                            .is_some_and(|t| self.mono.saturating_sub(t) <= SLC_HOT_WINDOW_MS);
                        if let Some(d) = self.session.disk.as_mut() {
                            d.slc = Some(SlcResult {
                                cache_bytes: n.value.unwrap_or(0),
                                steady_bps: None,
                                thermal_suspect: suspect,
                            });
                        }
                    }
                    "slc_steady" => {
                        let slc = self.session.disk.as_mut().and_then(|d| d.slc.as_mut());
                        if let (Some(slc), Some(v)) = (slc, n.value) {
                            slc.steady_bps = Some(v as f64);
                        }
                    }
                    // Other codes (`vram_bits`, `artifact_tiles`...) are diary events only.
                    _ => {}
                }
                let mut params = vec![("phase", n.phase.to_string())];
                if let Some(v) = n.value {
                    params.push(("value", v.to_string()));
                }
                self.event(&n.code, &params);
            }
            LoadMessage::PhaseDone(d) => {
                if let Some(p) = self.session.plan.phases.get(d.phase as usize) {
                    let outcome = match (&d.skipped, d.errors) {
                        (Some(_), _) => "skipped",
                        // The phase our stop cut short did not run its time: not «passed».
                        (None, 0) if self.state == RunState::Stopping => "stopped",
                        (None, 0) => "passed",
                        _ => "errors",
                    };
                    self.session.phases.push(PhaseResult {
                        index: d.phase,
                        kernel: p.kernel,
                        outcome: outcome.into(),
                        duration_ms: d.duration_ms,
                        checks: d.checks,
                        errors: d.errors,
                        skipped: d.skipped.clone(),
                    });
                }
            }
            LoadMessage::Finished(f) => {
                self.errors = self.errors.max(f.errors);
                self.checks = self.checks.max(f.checks);
                match f.reason {
                    FinishReason::Completed => self.completed = true,
                    FinishReason::Stopped => {
                        if self.thermal_stop.is_none() {
                            self.user_stop = true;
                        }
                    }
                    FinishReason::FirstError => {}
                    FinishReason::Failed => {
                        // A full disk and an I/O error are named by their own facts.
                        let named = self.disk_full || self.io_failed;
                        if !self.hung && self.device_lost.is_none() && !named {
                            self.crashed = true;
                        }
                    }
                }
                out.extend(self.finish(now));
            }
            // Messages the app sends, or the topology (the runner handles it).
            LoadMessage::Run(_) | LoadMessage::Stop(_) | LoadMessage::Topology(_) => {}
        }
        out
    }

    pub fn on_sample(
        &mut self,
        sample: &SensorSample,
        service_available: bool,
        now: Clock,
    ) -> Vec<Action> {
        self.guarded(now, |s| s.sample(sample, service_available, now))
    }

    fn sample(
        &mut self,
        sample: &SensorSample,
        service_available: bool,
        now: Clock,
    ) -> Vec<Action> {
        if self.ended {
            return vec![];
        }
        self.tick(now);
        if !self.is_gpu() && !self.is_disk() && service_available != self.config.service_available {
            self.config.service_available = service_available;
            if service_available {
                self.warnings.retain(|w| w != "noService");
                self.guard = self
                    .config
                    .thermal_stop
                    .then(|| ThermalGuard::new(self.config.threshold_c));
            } else {
                self.guard = None;
                self.warn("noService");
            }
        }
        if let (Some(m), Some(on)) = (self.stability.as_mut(), sample.throttling) {
            m.throttling(on, self.mono);
        }
        if self.is_disk() {
            self.disk_sample(sample);
        }
        self.last_sample = sample.clone();
        self.sums[0].add(sample.temp_c);
        self.sums[1].add(sample.power_w);
        self.sums[2].add(sample.clock_mhz);
        self.update_stats();
        match sample.temp_c {
            Some(_) => {
                self.temp_since = Some(now.mono_ms);
                self.warnings.retain(|w| w != "tempMissing");
            }
            None => {
                let since = *self.temp_since.get_or_insert(now.mono_ms);
                if now.mono_ms.saturating_sub(since) > TEMP_MISSING_MS {
                    self.warn("tempMissing");
                }
            }
        }
        let t = self.mono - self.start_mono;
        if self
            .last_sample_push
            .is_none_or(|p| t - p >= SAMPLE_EVERY_MS)
        {
            self.last_sample_push = Some(t);
            self.session.samples.push(Sample {
                t_ms: t,
                temp_c: sample.temp_c,
                power_w: sample.power_w,
                clock_mhz: sample.clock_mhz,
            });
        }
        match self
            .guard
            .as_mut()
            .map(|g| g.observe(sample.temp_c, now.mono_ms))
        {
            Some(ThermalEvent::Trip(t)) if self.state != RunState::Stopping => {
                self.begin_stop(StopCause::Thermal(t))
            }
            _ => vec![],
        }
    }

    /// The SMART counter before and after, and the heat that may explain an SLC cliff.
    fn disk_sample(&mut self, sample: &SensorSample) {
        if sample
            .temp_c
            .is_some_and(|t| t >= self.config.threshold_c - SLC_HOT_MARGIN_C)
        {
            self.hot_at = Some(self.mono);
        }
        let Some(d) = self.session.disk.as_mut() else {
            return;
        };
        match sample.host_written_gib {
            Some(g) => {
                d.host_written_before_gib.get_or_insert(g);
                d.host_written_after_gib = Some(g);
                self.warnings.retain(|w| w != "smartMissing");
            }
            None if d.host_written_before_gib.is_none() => self.warn("smartMissing"),
            None => {}
        }
    }

    pub fn on_whea(&mut self, result: Result<Vec<WheaEvent>, ()>, now: Clock) -> Vec<Action> {
        self.guarded(now, |s| s.whea(result, now))
    }

    fn whea(&mut self, result: Result<Vec<WheaEvent>, ()>, now: Clock) -> Vec<Action> {
        if self.state == RunState::Finished {
            return vec![];
        }
        self.tick(now);
        let events = match result {
            Ok(e) if self.config.whea_baseline_missing => {
                // The first answer is the history of the log: only the baseline.
                self.config.whea_baseline_missing = false;
                if let Some(last) = e.iter().map(|e| e.record_id).max() {
                    self.session.whea.last_record = Some(last);
                }
                return if self.ended { self.complete() } else { vec![] };
            }
            Ok(e) => e,
            Err(()) => {
                if !self.session.whea.unreadable {
                    self.session.whea.unreadable = true;
                    self.warn("wheaUnreadable");
                    self.event("whea_unreadable", &[]);
                }
                return if self.ended { self.complete() } else { vec![] };
            }
        };
        for e in events {
            if !matches!(e.event_id, 17..=19)
                || self
                    .session
                    .whea
                    .last_record
                    .is_some_and(|l| e.record_id <= l)
            {
                continue;
            }
            self.session.whea.last_record = Some(e.record_id);
            *self.session.whea.by_id.entry(e.event_id).or_default() += 1;
            let core = e
                .apic_id
                .and_then(|a| self.config.apic_to_core.get(&a).copied());
            if let Some(a) = e.apic_id {
                *self.session.whea.by_apic.entry(a).or_default() += 1;
            }
            if e.event_id != 18 && self.whea_core.is_none() {
                self.whea_core = core;
            }
            let mut params = vec![("id", e.event_id.to_string())];
            if let Some(a) = e.apic_id {
                params.push(("apic", a.to_string()));
            }
            if let Some(c) = core {
                params.push(("core", c.to_string()));
            }
            self.event("whea", &params);
        }
        if self.ended {
            return self.complete();
        }
        vec![]
    }

    pub fn on_clock(&mut self, now: Clock) -> Vec<Action> {
        self.guarded(now, |s| s.clock(now))
    }

    fn clock(&mut self, now: Clock) -> Vec<Action> {
        if self.ended {
            // Waiting for the final WHEA poll; give up after 2 s.
            return match self.final_deadline {
                Some(d) if now.mono_ms >= d => self.complete(),
                _ => vec![],
            };
        }
        self.tick(now);
        // Before any pipe check: after a sleep the pipe is silent too (DA15).
        if let Some(out) = self.sleep_check(now) {
            return out;
        }
        let mut out = vec![];
        // Strict phases (R9) must end by their nominal time. A fill (R7) and the V loads
        // end on completion or after a final verify, so only the silent pipe flags them.
        let soft = self
            .session
            .plan
            .phases
            .get(self.phase as usize)
            .is_some_and(|p| {
                matches!(
                    p.kernel,
                    KernelId::DiskFill | KernelId::V1 | KernelId::V2 | KernelId::V3 | KernelId::V4
                )
            });
        // Disk phases differ in length (a fill can run far past its time): each strict one
        // is measured from its own start, not from the plan's.
        let limit = match self.session.plan.phases.get(self.phase as usize) {
            Some(p) if self.is_disk() => {
                let from = self.phase_start.unwrap_or(self.start_mono);
                self.mono.saturating_sub(from)
                    > u64::from(p.duration_s) * 1000 + u64::from(p.pause_before_ms) + OVERRUN_MS
            }
            _ => {
                self.mono - self.start_mono > self.session.plan.total_seconds() * 1000 + OVERRUN_MS
            }
        };
        let overrun = !soft && limit;
        if self.state == RunState::Running
            && (overrun || now.mono_ms.saturating_sub(self.last_msg_ms) > SILENT_PIPE_MS)
        {
            self.hung = true;
            self.event("hung", &[]);
            out.push(Action::Kill);
            out.extend(self.finish(now));
            return out;
        }
        if self.state == RunState::Stopping && self.stop_deadline.is_some_and(|d| now.mono_ms >= d)
        {
            out.push(Action::Kill);
            out.extend(self.finish(now));
            return out;
        }
        if now.mono_ms >= self.next_whea_ms {
            self.next_whea_ms = now.mono_ms + WHEA_EVERY_MS;
            out.push(Action::PollWhea {
                after_record: self.session.whea.last_record,
            });
        }
        if self.journal_seen.is_some()
            && now.mono_ms.saturating_sub(self.last_journal_ms) >= JOURNAL_EVERY_MS
        {
            out.push(Action::WriteJournal(self.journal(now)));
        }
        if now.mono_ms.saturating_sub(self.last_save_ms) >= SAVE_EVERY_MS {
            self.last_save_ms = now.mono_ms;
            out.push(Action::SaveSession);
        }
        out
    }

    /// Ends the session as `suspended` when the PC slept since the last check (DA15). The
    /// runner calls it after a wait, before the messages of the pipe, so a `Finished`
    /// queued during the sleep cannot end the session first.
    pub fn on_sleep_check(&mut self, now: Clock) -> Vec<Action> {
        self.guarded(now, |s| {
            if s.ended {
                return vec![];
            }
            s.tick(now);
            s.sleep_check(now).unwrap_or_default()
        })
    }

    fn sleep_check(&mut self, now: Clock) -> Option<Vec<Action>> {
        let slept = now.asleep_ms.saturating_sub(self.last_asleep);
        self.last_asleep = now.asleep_ms;
        if slept <= SLEEP_JUMP_MS {
            return None;
        }
        self.suspended = true;
        self.event("suspended", &[]);
        let mut out = vec![Action::SendStop, Action::Kill];
        out.extend(self.finish(now));
        Some(out)
    }

    /// The NVIDIA PCIe replay count, at the start and every 5 s (DG14). The first value is
    /// the baseline; a rise warns once. Never an error.
    pub fn on_pcie_replay(&mut self, count: u32, now: Clock) -> Vec<Action> {
        self.guarded(now, |s| {
            if s.ended {
                return vec![];
            }
            s.tick(now);
            let base = *s.pcie_base.get_or_insert(count);
            if count > base && !s.pcie_warned {
                s.pcie_warned = true;
                s.warn("pcieReplay");
                s.event(
                    "pcie_replay",
                    &[("from", base.to_string()), ("to", count.to_string())],
                );
            }
            vec![]
        })
    }

    pub fn on_user_stop(&mut self, now: Clock) -> Vec<Action> {
        self.guarded(now, |s| s.user_stop(now))
    }

    fn user_stop(&mut self, now: Clock) -> Vec<Action> {
        if self.ended || !matches!(self.state, RunState::Starting | RunState::Running) {
            return vec![];
        }
        self.tick(now);
        self.begin_stop(StopCause::User)
    }

    pub fn on_exit(&mut self, code: Option<i32>, now: Clock) -> Vec<Action> {
        self.guarded(now, |s| s.exit(code, now))
    }

    fn exit(&mut self, code: Option<i32>, now: Clock) -> Vec<Action> {
        if self.ended {
            return vec![];
        }
        self.tick(now);
        // A clean exit after our own stop request is not a crash; a usage exit means the
        // plan was refused, so it never ran.
        if code == Some(LOAD_EXIT_USAGE) {
            self.failed_to_start = Some(INVALID_PLAN.into());
        } else if code == Some(LOAD_EXIT_DEVICE_LOST) {
            // 0: the exit code carries no HRESULT.
            self.device_lost.get_or_insert(0);
        } else if code == Some(LOAD_EXIT_IO) {
            // The `io_error` is the cause; keep it counted if its message was lost.
            self.errors = self.errors.max(1);
            self.coreless_errors = true;
            self.io_failed = true;
        } else if !(self.state == RunState::Stopping && code == Some(0)) {
            self.crashed = true;
            let code = code.map_or("-".into(), |c| c.to_string());
            self.event("crashed", &[("code", code)]);
        }
        self.finish(now)
    }

    pub fn status(&self) -> RunStatus {
        let s = &self.session;
        RunStatus {
            state: self.state,
            session_id: s.id.clone(),
            component: s.component,
            objective: s.objective,
            preset: s.preset,
            elapsed_ms: self.mono - self.start_mono,
            total_ms: s.plan.total_seconds() * 1000,
            phase_index: self.phase,
            phases: s
                .plan
                .phases
                .iter()
                .map(|p| PhaseInfo {
                    kernel: p.kernel,
                    mode: p.mode,
                    placement: p.placement,
                    duration_s: p.duration_s,
                    isa: p.isa,
                })
                .collect(),
            temp_c: self.last_sample.temp_c,
            temp_max_c: self.sums[0].max,
            stop_c: self.thermal_armed().then_some(self.config.threshold_c),
            power_w: self.last_sample.power_w,
            clock_mhz: self.last_sample.clock_mhz,
            load_percent: self.load_percent,
            stability: self
                .stability
                .as_ref()
                .and_then(StabilityMeter::result)
                .or(s.stability),
            gpu_device_id: s.gpu_device_id.clone(),
            disk: self.disk_status.clone(),
            checks: self.checks,
            errors: self.errors,
            whea_corrected: self.whea_count(&[17, 19]),
            whea_fatal: self.whea_count(&[18]),
            cores: s
                .cores
                .iter()
                .map(|c| CoreProgress {
                    core: c.core,
                    state: c.state,
                })
                .collect(),
            current_core: self.current_core,
            events: s
                .events
                .iter()
                .skip(s.events.len().saturating_sub(STATUS_EVENTS))
                .cloned()
                .collect(),
            warnings: self.warnings.clone(),
            outcome: s.outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::plan::StartRequest;
    use oma_ipc::load::{
        ComputeError, DataSize, Finished, LoadHello, Notice, Phase, PhaseDone, Plan, Progress,
        StopRequest,
    };

    fn phase(kernel: KernelId) -> Phase {
        Phase {
            kernel,
            alt_kernel: None,
            isa: Isa::Avx2,
            size: DataSize::L2,
            mode: LoadMode::Steady,
            placement: Placement::AllLogical,
            duration_s: 60,
            per_core_s: None,
            both_smt: false,
            cores: None,
            patterns: vec![],
            stop_on_error: false,
            iterations: None,
            pause_before_ms: 0,
            windows: None,

            disk: None,
        }
    }

    fn session() -> Session {
        Session {
            format: FORMAT,
            id: "0b9f6c1e-7d2a-4c53-9a1e-3f5d8e2b7a10".into(),
            started_at: "2026-10-06T14:03:09Z".into(),
            ended_at: None,
            component: Component::Cpu,
            device: "CPU".into(),
            objective: Objective::Normal,
            preset: Preset::Quick,
            request: StartRequest {
                component: Component::Cpu,
                objective: Objective::Normal,
                preset: Preset::Quick,
                custom: None,
                retry_core: None,
                gpu: None,
                disk: None,
            },
            plan: Plan {
                seed: 1,
                ram_bytes: 0,
                phases: vec![phase(KernelId::K2), phase(KernelId::K5)],
                gpu: None,

                disk: None,
            },
            outcome: None,
            outcome_detail: None,
            phases: vec![],
            cores: vec![],
            errors: vec![],
            errors_dropped: 0,
            events_dropped: 0,
            whea: Default::default(),
            stats: Default::default(),
            samples: vec![],
            events: vec![],
            app_version: "0".into(),
            load_version: None,
            stability: None,
            gpu_device_id: None,
            disk: None,
        }
    }

    fn clock(t: u64) -> Clock {
        Clock {
            mono_ms: t,
            wall_ms: 1_000_000_000_000 + t as i64,
            asleep_ms: 0,
        }
    }

    fn ctl(thermal: bool, service: bool) -> RunController {
        RunController::new(
            session(),
            RunConfig {
                threshold_c: 90.0,
                thermal_stop: thermal,
                service_available: service,
                cores: vec![0, 1, 2, 3],
                apic_to_core: [(8, 1)].into(),
                whea_after: None,
                whea_baseline_missing: false,
            },
            clock(0),
        )
    }

    fn progress(phase: u32, core: Option<u32>) -> LoadMessage {
        LoadMessage::Progress(Progress {
            phase,
            phase_elapsed_ms: 0,
            elapsed_ms: 0,
            checks: 10,
            errors: 0,
            current_core: core,
            cores: vec![],
            memory_bytes: 0,
            rate: None,
            load_percent: None,

            disk: None,
        })
    }

    fn error(core: Option<u32>, kind: ErrorKind) -> LoadMessage {
        LoadMessage::Error(ComputeError {
            phase: 0,
            kernel: KernelId::K2,
            isa: Isa::Avx2,
            kind,
            logical: core,
            core,
            iteration: 3,
            expected: 1,
            actual: 2,
            seed: 1,
            load_percent: None,

            transient: None,
        })
    }

    fn finished(reason: FinishReason) -> LoadMessage {
        LoadMessage::Finished(Finished {
            reason,
            checks: 5,
            errors: 0,
        })
    }

    fn sample(temp: Option<f64>) -> SensorSample {
        SensorSample {
            temp_c: temp,
            power_w: Some(100.0),
            clock_mhz: Some(4500.0),
            core_clock_mhz: vec![Some(4000.0), Some(4100.0), Some(4200.0), Some(4300.0)],
            throttling: None,
            thermal_throttling: None,
            ..Default::default()
        }
    }

    /// The final WHEA poll answers (nothing new): the verdict is out.
    fn settle(c: &mut RunController, mut a: Vec<Action>) -> Vec<Action> {
        assert!(a.iter().any(is_poll), "the end asks for a final poll");
        a.extend(c.on_whea(Ok(vec![]), clock(c.mono)));
        a
    }

    fn has_finished(a: &[Action], o: Outcome) -> bool {
        a.contains(&Action::Finished(o))
    }

    fn verdict(c: &RunController) -> &str {
        &c.session().outcome_detail.as_ref().unwrap().verdict
    }

    fn count(a: &[Action], f: fn(&Action) -> bool) -> usize {
        a.iter().filter(|x| f(x)).count()
    }

    fn is_journal(a: &Action) -> bool {
        matches!(a, Action::WriteJournal(_))
    }

    fn is_poll(a: &Action) -> bool {
        matches!(a, Action::PollWhea { .. })
    }

    #[test]
    fn normal_run_completes_as_passed() {
        let mut c = ctl(true, true);
        c.on_load(
            &LoadMessage::Hello(LoadHello {
                protocol_version: 1,
                version: "9".into(),
                isa: vec![],
                shader_digest: None,
            }),
            clock(100),
        );
        c.on_load(&progress(0, None), clock(1000));
        c.on_load(
            &LoadMessage::PhaseDone(PhaseDone {
                phase: 0,
                checks: 5,
                errors: 0,
                duration_ms: 1000,
                skipped: None,
                work_ms: None,
                workers: vec![],
                rates: vec![],

                disk: None,
            }),
            clock(2000),
        );
        let a = c.on_load(&finished(FinishReason::Completed), clock(3000));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::Passed));
        for want in [Action::SaveSession, Action::DeleteJournal, Action::Toast] {
            assert!(a.contains(&want), "{want:?}");
        }
        assert_eq!(count(&a, is_poll), 1);
        assert!(c.is_finished());
        let s = c.session();
        assert_eq!(s.outcome, Some(Outcome::Passed));
        assert_eq!(s.ended_at.as_deref(), Some("2001-09-09T01:46:43Z"));
        assert_eq!(s.load_version.as_deref(), Some("9"));
        assert_eq!(s.phases.len(), 1);
        assert_eq!(c.status().state, RunState::Finished);
        // Nothing happens after the end.
        assert!(c.on_clock(clock(99_000)).is_empty());
    }

    #[test]
    fn error_on_one_core_is_unstable_core_n_with_clock_and_temp() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, Some(2)), clock(1000));
        c.on_sample(&sample(Some(71.0)), true, clock(1500));
        c.on_load(&error(Some(2), ErrorKind::Mismatch), clock(2000));
        let a = c.on_load(&finished(FinishReason::FirstError), clock(2500));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::Errors));
        let s = c.session();
        assert_eq!(verdict(&c), "errors_core");
        let d = s.outcome_detail.as_ref().unwrap();
        assert_eq!(
            (d.core, d.temp_c, d.clock_mhz),
            (Some(2), Some(71.0), Some(4200.0))
        );
        assert_eq!(s.errors[0].clock_mhz, Some(4200.0));
        assert_eq!(s.errors[0].temp_c, Some(71.0));
        assert_eq!(s.cores[2].state, CoreState::Failed);
        assert_eq!(c.status().errors, 1);
    }

    #[test]
    fn error_count_is_the_largest_of_errors_progress_and_finished() {
        // oma-load sends at most 16 `Error` messages per worker and phase, but counts them all.
        let mut c = ctl(false, true);
        c.on_load(&error(Some(1), ErrorKind::Mismatch), clock(1000));
        let mut p = progress(0, None);
        if let LoadMessage::Progress(p) = &mut p {
            p.errors = 40;
        }
        c.on_load(&p, clock(1500));
        assert_eq!(c.status().errors, 40);
        let mut f = finished(FinishReason::Completed);
        if let LoadMessage::Finished(f) = &mut f {
            f.errors = 50;
        }
        let a = c.on_load(&f, clock(2000));
        settle(&mut c, a);
        assert_eq!(c.status().errors, 50);
        assert_eq!(
            verdict(&c),
            "errors_core",
            "the core still comes from the Error"
        );
        assert_eq!(c.session().cores[1].state, CoreState::Failed);
    }

    #[test]
    fn thermal_trip_stops_and_records_the_temperature() {
        let mut c = ctl(true, true);
        c.on_load(&progress(0, None), clock(1000));
        assert!(c
            .on_sample(&sample(Some(95.0)), true, clock(1000))
            .is_empty());
        let a = c.on_sample(&sample(Some(96.0)), true, clock(2000));
        assert_eq!(a, vec![Action::SendStop]);
        assert_eq!(c.status().state, RunState::Stopping);
        let a = c.on_load(&finished(FinishReason::Stopped), clock(2200));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::StoppedThermal));
        let d = c.session().outcome_detail.as_ref().unwrap();
        assert_eq!(d.params["temp"], "96");
        assert_eq!(d.temp_c, Some(96.0));
    }

    #[test]
    fn intermediate_save_carries_the_stats_for_a_crash() {
        let mut c = ctl(true, true);
        c.on_load(&progress(0, None), clock(1000));
        c.on_sample(&sample(Some(70.0)), true, clock(1000));
        c.on_load(&progress(0, None), clock(61_000));
        let a = c.on_clock(clock(61_500));
        assert!(a.contains(&Action::SaveSession));
        assert_eq!(c.session().stats.temp_max_c, Some(70.0));
    }

    #[test]
    fn phase_cut_by_a_stop_reads_stopped_not_passed() {
        let mut c = ctl(true, true);
        c.on_load(&progress(0, None), clock(1000));
        c.on_sample(&sample(Some(95.0)), true, clock(1000));
        c.on_sample(&sample(Some(96.0)), true, clock(2000));
        c.on_load(&phase_done(0, None), clock(2100));
        assert_eq!(c.session().phases[0].outcome, "stopped");
    }

    #[test]
    fn no_service_runs_with_a_warning_and_no_thermal_stop() {
        let mut c = ctl(true, false);
        assert_eq!(c.status().warnings, ["noService"]);
        assert_eq!(c.status().stop_c, None);
        c.on_load(&progress(0, None), clock(1000));
        for t in [2000, 3000, 4000] {
            assert!(c
                .on_sample(&sample(Some(120.0)), false, clock(t))
                .is_empty());
        }
        // The service returns: the stop is armed again.
        c.on_sample(&sample(Some(120.0)), true, clock(5000));
        assert!(c.status().warnings.is_empty());
        assert_eq!(c.status().stop_c, Some(90.0));
        let a = c.on_sample(&sample(Some(120.0)), true, clock(6000));
        assert_eq!(a, vec![Action::SendStop]);
    }

    #[test]
    fn user_stop_saves_stopped_user() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(1000));
        assert_eq!(c.on_user_stop(clock(2000)), vec![Action::SendStop]);
        assert!(c.on_user_stop(clock(2100)).is_empty());
        let a = c.on_load(&finished(FinishReason::Stopped), clock(2500));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::StoppedUser));
        assert!(a.contains(&Action::SaveSession));
    }

    #[test]
    fn sleep_ends_as_suspended_before_the_pipe_check() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(1000));
        let a = c.on_clock(Clock {
            mono_ms: 60_000,
            wall_ms: 0,
            asleep_ms: 50_000,
        });
        let a = settle(&mut c, a);
        assert_eq!(a[..2], [Action::SendStop, Action::Kill]);
        assert!(has_finished(&a, Outcome::Suspended));
    }

    #[test]
    fn wall_clock_change_is_not_a_suspend() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(1000));
        let a = c.on_clock(Clock {
            mono_ms: 1250,
            wall_ms: 5_000_000_000_000,
            asleep_ms: 0,
        });
        assert!(a.is_empty());
        assert!(!c.is_finished());
        // A nap of under a second does not count either.
        let a = c.on_clock(Clock {
            mono_ms: 1500,
            wall_ms: 0,
            asleep_ms: 900,
        });
        assert!(!c.is_finished() && a.is_empty());
    }

    #[test]
    fn silent_pipe_is_hung_after_five_seconds() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(1000));
        assert!(c.on_clock(clock(6000)).iter().all(|a| *a != Action::Kill));
        let a = c.on_clock(clock(6001));
        let a = settle(&mut c, a);
        assert_eq!(a[0], Action::Kill);
        assert!(has_finished(&a, Outcome::Hung));
    }

    #[test]
    fn silent_pipe_is_ignored_before_the_first_message() {
        let mut c = ctl(false, true);
        assert!(c.on_clock(clock(30_000)).iter().all(|a| *a != Action::Kill));
        assert!(!c.is_finished());
    }

    #[test]
    fn hung_error_then_failed_is_hung() {
        let mut c = ctl(false, true);
        c.on_load(&error(Some(1), ErrorKind::Hung), clock(1000));
        let a = c.on_load(&finished(FinishReason::Failed), clock(1100));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::Hung));
        let mut c = ctl(false, true);
        let a = c.on_load(&finished(FinishReason::Failed), clock(1100));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::Crashed));
    }

    #[test]
    fn exit_without_finished_is_crashed() {
        let mut c = ctl(false, true);
        c.on_load(&progress(1, Some(3)), clock(1000));
        let a = c.on_exit(Some(-1), clock(2000));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::Crashed));
        let d = c.session().outcome_detail.as_ref().unwrap();
        assert_eq!(
            (d.phase, d.core, d.kernel),
            (Some(1), Some(3), Some(KernelId::K5))
        );
    }

    #[test]
    fn stop_without_answer_kills_after_three_seconds() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(1000));
        c.on_user_stop(clock(2000));
        assert!(c.on_clock(clock(4999)).iter().all(|a| *a != Action::Kill));
        let a = c.on_clock(clock(5000));
        let a = settle(&mut c, a);
        assert_eq!(a[0], Action::Kill);
        assert!(has_finished(&a, Outcome::StoppedUser));
    }

    #[test]
    fn silence_while_stopping_ends_by_the_grace_not_as_hung() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(1000));
        c.on_user_stop(clock(10_000));
        // 12 s without a message, but we are stopping: no `hung`.
        let a = c.on_clock(clock(13_000));
        let a = settle(&mut c, a);
        assert_eq!(a[0], Action::Kill);
        assert!(has_finished(&a, Outcome::StoppedUser));
    }

    #[test]
    fn thermal_stop_without_answer_ends_in_kill() {
        let mut c = ctl(true, true);
        c.on_load(&progress(0, None), clock(1000));
        c.on_sample(&sample(Some(99.0)), true, clock(2000));
        c.on_sample(&sample(Some(99.0)), true, clock(3000));
        let a = c.on_clock(clock(6000));
        let a = settle(&mut c, a);
        assert_eq!(a[0], Action::Kill);
        assert!(has_finished(&a, Outcome::StoppedThermal));
    }

    #[test]
    fn sleep_while_stopping_is_suspended() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(1000));
        c.on_user_stop(clock(2000));
        let a = c.on_clock(Clock {
            mono_ms: 2500,
            wall_ms: 0,
            asleep_ms: 9000,
        });
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::Suspended));
    }

    #[test]
    fn final_poll_timeout_finishes_after_two_seconds() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(1000));
        let a = c.on_load(&finished(FinishReason::Completed), clock(2000));
        assert_eq!(count(&a, is_poll), 1);
        assert!(!has_finished(&a, Outcome::Passed));
        assert!(!c.is_finished());
        assert!(c.on_clock(clock(3999)).is_empty());
        let a = c.on_clock(clock(4000));
        assert!(has_finished(&a, Outcome::Passed));
        assert!(c.is_finished());
        // A late answer changes nothing.
        assert!(c.on_whea(Ok(vec![]), clock(4100)).is_empty());
    }

    #[test]
    fn unreadable_final_poll_finishes_and_warns() {
        let mut c = ctl(false, true);
        c.on_load(&finished(FinishReason::Completed), clock(2000));
        let a = c.on_whea(Err(()), clock(2100));
        assert!(has_finished(&a, Outcome::Passed));
        assert!(c.session().whea.unreadable);
    }

    #[test]
    fn first_call_writes_journal_and_session() {
        let mut c = ctl(false, true);
        let a = c.on_sample(&sample(Some(50.0)), true, clock(500));
        assert!(matches!(a[0], Action::WriteJournal(_)));
        assert_eq!(a[1], Action::SaveSession);
        assert!(c
            .on_sample(&sample(Some(50.0)), true, clock(1500))
            .is_empty());
    }

    #[test]
    fn whea_history_before_the_start_is_ignored() {
        let mut c = RunController::new(
            session(),
            RunConfig {
                threshold_c: 90.0,
                thermal_stop: false,
                service_available: true,
                cores: vec![],
                apic_to_core: BTreeMap::new(),
                whea_after: Some(100),
                whea_baseline_missing: false,
            },
            clock(0),
        );
        assert_eq!(c.session().whea.last_record, Some(100));
        let ev = |rec| WheaEvent {
            record_id: rec,
            event_id: 19,
            apic_id: None,
            time_utc: String::new(),
        };
        c.on_whea(Ok(vec![ev(90), ev(100), ev(101)]), clock(1000));
        assert_eq!(c.status().whea_corrected, 1);
        let a = c.on_clock(clock(5000));
        assert!(a.contains(&Action::PollWhea {
            after_record: Some(101)
        }));
    }

    #[test]
    fn reference_invalid_is_an_event_not_an_error() {
        let mut c = ctl(false, true);
        c.on_load(&error(Some(1), ErrorKind::ReferenceInvalid), clock(1000));
        assert_eq!(c.status().errors, 0);
        assert!(c.session().errors.is_empty());
        assert_eq!(c.session().cores[1].state, CoreState::Untested);
        assert!(c
            .status()
            .events
            .iter()
            .any(|e| e.code == "reference_invalid"));
        c.on_load(
            &error(Some(1), ErrorKind::ReferenceDisagreement),
            clock(1100),
        );
        assert_eq!(c.status().errors, 1);
        assert_eq!(c.session().cores[1].state, CoreState::Untested);
        let a = c.on_load(&finished(FinishReason::FirstError), clock(1200));
        let _ = settle(&mut c, a);
        assert_eq!(verdict(&c), "errors");
    }

    #[test]
    fn events_keep_the_newest_and_count_the_dropped() {
        let mut c = ctl(false, true);
        for i in 0..(MAX_EVENTS + 5) {
            c.on_load(
                &LoadMessage::Notice(Notice {
                    phase: i as u32,
                    code: "x".into(),
                    value: None,
                }),
                clock(1000),
            );
        }
        let s = c.session();
        assert_eq!(s.events.len(), MAX_EVENTS);
        assert_eq!(s.events_dropped, 5);
        assert_eq!(s.events[0].params["phase"], "5");
        assert_eq!(c.status().events.len(), 200);
    }

    #[test]
    fn stats_are_in_the_session_before_the_end() {
        let mut c = ctl(false, true);
        c.on_sample(&sample(Some(60.0)), true, clock(0));
        c.on_sample(&sample(Some(80.0)), true, clock(1000));
        let st = &c.session().stats;
        assert_eq!((st.temp_max_c, st.temp_avg_c), (Some(80.0), Some(70.0)));
    }

    #[test]
    fn clean_exit_after_a_stop_is_the_stop() {
        let mut c = ctl(false, true);
        c.on_user_stop(clock(1000));
        let a = c.on_exit(Some(0), clock(1500));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::StoppedUser));
    }

    #[test]
    fn journal_every_phase_change_and_every_thirty_seconds() {
        let mut c = ctl(false, true);
        assert_eq!(
            count(&c.on_load(&progress(0, None), clock(1000)), is_journal),
            1
        );
        assert_eq!(
            count(&c.on_load(&progress(0, None), clock(2000)), is_journal),
            0
        );
        let a = c.on_load(&progress(1, None), clock(3000));
        match &a[0] {
            Action::WriteJournal(j) => {
                assert_eq!((j.phase_index, j.kernel), (1, Some(KernelId::K5)));
                assert_eq!(j.updated_at, "2001-09-09T01:46:43Z");
                assert!(!j.clean_end);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            count(&c.on_load(&progress(1, Some(2)), clock(4000)), is_journal),
            1
        );
        for t in [10_000, 20_000, 33_999] {
            c.on_load(&progress(1, Some(2)), clock(t));
            assert_eq!(count(&c.on_clock(clock(t)), is_journal), 0, "{t}");
        }
        c.on_load(&progress(1, Some(2)), clock(34_000));
        assert_eq!(count(&c.on_clock(clock(34_000)), is_journal), 1);
    }

    #[test]
    fn session_saved_every_sixty_seconds() {
        let mut c = ctl(false, true);
        let saves = |a: &[Action]| count(a, |x| *x == Action::SaveSession);
        // The first call (t = 1 s) saves once; then every 60 s.
        assert_eq!(saves(&c.on_load(&progress(0, None), clock(1000))), 1);
        for t in (2..=60).map(|s| s * 1000) {
            c.on_load(&progress(0, None), clock(t));
            assert_eq!(saves(&c.on_clock(clock(t))), 0);
        }
        c.on_load(&progress(0, None), clock(61_000));
        assert_eq!(saves(&c.on_clock(clock(61_000))), 1);
        c.on_load(&progress(0, None), clock(62_000));
        assert_eq!(saves(&c.on_clock(clock(62_000))), 0);
    }

    #[test]
    fn whea_19_with_apic_maps_to_a_core_and_gives_marginal() {
        let mut c = ctl(false, true);
        let ev = |id, rec, apic| WheaEvent {
            record_id: rec,
            event_id: id,
            apic_id: apic,
            time_utc: String::new(),
        };
        c.on_whea(
            Ok(vec![ev(19, 5, Some(8)), ev(18, 6, None), ev(1, 7, None)]),
            clock(1000),
        );
        // The same record again is not counted twice.
        c.on_whea(Ok(vec![ev(19, 5, Some(8))]), clock(2000));
        let st = c.status();
        assert_eq!((st.whea_corrected, st.whea_fatal), (1, 1));
        assert_eq!(c.session().whea.by_apic[&8], 1);
        assert_eq!(c.session().whea.last_record, Some(6));
        assert!(st
            .events
            .iter()
            .any(|e| e.code == "whea" && e.params["core"] == "1"));
        let a = c.on_load(&finished(FinishReason::Completed), clock(3000));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::Marginal));
        assert_eq!(c.session().outcome_detail.as_ref().unwrap().core, Some(1));
    }

    #[test]
    fn whea_found_by_the_final_poll_updates_the_outcome() {
        let mut c = ctl(false, true);
        let a = c.on_load(&finished(FinishReason::Completed), clock(1000));
        assert!(!has_finished(&a, Outcome::Passed));
        let e = WheaEvent {
            record_id: 1,
            event_id: 17,
            apic_id: None,
            time_utc: String::new(),
        };
        let a = c.on_whea(Ok(vec![e]), clock(1100));
        assert!(has_finished(&a, Outcome::Marginal));
        assert_eq!(count(&a, |x| *x == Action::SaveSession), 1);
        assert_eq!(c.session().outcome, Some(Outcome::Marginal));
    }

    #[test]
    fn whea_unreadable_warns_once() {
        let mut c = ctl(false, true);
        c.on_whea(Err(()), clock(1000));
        c.on_whea(Err(()), clock(2000));
        assert_eq!(c.status().warnings, ["wheaUnreadable"]);
        let n = c
            .status()
            .events
            .iter()
            .filter(|e| e.code == "whea_unreadable")
            .count();
        assert_eq!(n, 1);
        assert!(c.session().whea.unreadable);
    }

    #[test]
    fn errors_beyond_200_are_counted() {
        let mut c = ctl(false, true);
        for i in 0..205 {
            c.on_load(&error(Some(i % 4), ErrorKind::Mismatch), clock(1000));
        }
        assert_eq!(c.session().errors.len(), MAX_ERRORS);
        assert_eq!(c.session().errors_dropped, 5);
        assert_eq!(c.status().errors, 205);
        let a = c.on_load(&finished(FinishReason::FirstError), clock(2000));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::Errors));
        assert_eq!(verdict(&c), "errors");
    }

    #[test]
    fn samples_every_five_seconds() {
        let mut c = ctl(false, true);
        for t in (0..=12).map(|s| s * 1000) {
            c.on_sample(&sample(Some(60.0 + t as f64 / 1000.0)), true, clock(t));
        }
        let ts: Vec<u64> = c.session().samples.iter().map(|s| s.t_ms).collect();
        assert_eq!(ts, [0, 5000, 10_000]);
        let st = &c.session().stats;
        assert_eq!(st.temp_max_c, Some(72.0));
        assert_eq!(st.temp_avg_c, Some(66.0));
        assert_eq!(st.power_max_w, Some(100.0));
    }

    #[test]
    fn whea_is_polled_every_five_seconds() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(1000));
        assert_eq!(count(&c.on_clock(clock(4999)), is_poll), 0);
        assert_eq!(count(&c.on_clock(clock(5000)), is_poll), 1);
        c.on_load(&progress(0, None), clock(9000));
        assert_eq!(count(&c.on_clock(clock(9999)), is_poll), 0);
    }

    #[test]
    fn notices_set_warnings_and_events() {
        let mut c = ctl(false, true);
        c.on_load(
            &LoadMessage::Notice(Notice {
                phase: 2,
                code: "ram_reduced".into(),
                value: Some(7),
            }),
            clock(1000),
        );
        assert_eq!(c.status().warnings, ["ramReduced"]);
        assert_eq!(c.status().events[0].code, "ram_reduced");
        // Messages from the app side are ignored.
        let a = c.on_load(&LoadMessage::Stop(StopRequest {}), clock(1100));
        assert!(a.is_empty());
        assert_eq!(c.status().events.len(), 1);
    }

    fn notice(code: &str) -> LoadMessage {
        LoadMessage::Notice(Notice {
            phase: 0,
            code: code.into(),
            value: None,
        })
    }

    fn phase_done(phase: u32, skipped: Option<&str>) -> LoadMessage {
        LoadMessage::PhaseDone(PhaseDone {
            phase,
            checks: 0,
            errors: 0,
            duration_ms: 1000,
            skipped: skipped.map(str::to_owned),
            work_ms: None,
            workers: vec![],
            rates: vec![],

            disk: None,
        })
    }

    #[test]
    fn ram_insufficient_raises_a_warning() {
        let mut c = ctl(false, true);
        c.on_load(&notice("ram_insufficient"), clock(1000));
        assert_eq!(c.status().warnings, ["ramInsufficient"]);
    }

    #[test]
    fn completed_without_checks_is_failed_to_start() {
        let mut c = ctl(false, true);
        let a = c.on_load(
            &LoadMessage::Finished(Finished {
                reason: FinishReason::Completed,
                checks: 0,
                errors: 0,
            }),
            clock(1000),
        );
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::FailedToStart));
        let d = c.session().outcome_detail.as_ref().unwrap();
        assert_eq!(d.params["reason"], "performance.start.nothing_ran");
    }

    #[test]
    fn every_phase_skipped_is_failed_to_start() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(500));
        c.on_load(&phase_done(0, Some("unsupported")), clock(1000));
        c.on_load(&phase_done(1, Some("ram_insufficient")), clock(1500));
        let a = c.on_load(&finished(FinishReason::Completed), clock(2000));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::FailedToStart));
        // One phase that ran is enough.
        let mut c = ctl(false, true);
        c.on_load(&phase_done(0, None), clock(1000));
        c.on_load(&phase_done(1, Some("unsupported")), clock(1500));
        let a = c.on_load(&finished(FinishReason::Completed), clock(2000));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::Passed));
    }

    #[test]
    fn a_core_less_error_makes_errors_not_core_n() {
        let mut c = ctl(false, true);
        c.on_load(&error(Some(2), ErrorKind::Mismatch), clock(1000));
        c.on_load(&error(None, ErrorKind::ReferenceDisagreement), clock(1100));
        let a = c.on_load(&finished(FinishReason::Completed), clock(1200));
        settle(&mut c, a);
        assert_eq!(verdict(&c), "errors");
    }

    #[test]
    fn past_the_overall_deadline_is_hung() {
        // Two phases of 60 s: the run may last 120 s + 120 s of grace.
        let mut c = ctl(false, true);
        let mut t = 0;
        while t < 240_000 {
            t += 1000;
            c.on_load(&progress(0, None), clock(t));
            assert!(
                c.on_clock(clock(t)).iter().all(|a| *a != Action::Kill),
                "{t}"
            );
        }
        c.on_load(&progress(0, None), clock(240_001));
        let a = c.on_clock(clock(240_001));
        let a = settle(&mut c, a);
        assert_eq!(a[0], Action::Kill);
        assert!(has_finished(&a, Outcome::Hung));
    }

    #[test]
    fn usage_exit_without_finished_is_failed_to_start() {
        let mut c = ctl(false, true);
        let a = c.on_exit(Some(1), clock(1000));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::FailedToStart));
        let d = c.session().outcome_detail.as_ref().unwrap();
        assert_eq!(d.params["reason"], "performance.start.invalid_plan");
    }

    #[test]
    fn unknown_whea_baseline_is_set_by_the_first_good_poll() {
        let mut c = RunController::new(
            session(),
            RunConfig {
                whea_baseline_missing: true,
                ..ctl(false, true).config.clone()
            },
            clock(0),
        );
        let ev = |rec| WheaEvent {
            record_id: rec,
            event_id: 19,
            apic_id: None,
            time_utc: String::new(),
        };
        c.on_whea(Err(()), clock(500));
        // The history in the log: only the baseline.
        c.on_whea(Ok(vec![ev(40), ev(41)]), clock(1000));
        assert_eq!(c.status().whea_corrected, 0);
        assert_eq!(c.session().whea.last_record, Some(41));
        c.on_whea(Ok(vec![ev(41), ev(42)]), clock(2000));
        assert_eq!(c.status().whea_corrected, 1);
    }

    #[test]
    fn sleep_check_alone_ends_as_suspended() {
        let mut c = ctl(false, true);
        c.on_load(&progress(0, None), clock(1000));
        let awake = Clock {
            mono_ms: 1250,
            wall_ms: 0,
            asleep_ms: 0,
        };
        assert!(c.on_sleep_check(awake).is_empty());
        let a = c.on_sleep_check(Clock {
            mono_ms: 60_000,
            wall_ms: 0,
            asleep_ms: 50_000,
        });
        assert_eq!(a[..2], [Action::SendStop, Action::Kill]);
        // The Finished that follows changes nothing.
        let a2 = c.on_load(&finished(FinishReason::Completed), clock(60_001));
        assert!(a2.is_empty());
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::Suspended));
    }

    #[test]
    fn temp_missing_warns_without_thermal_stop_and_clears() {
        let mut c = ctl(false, true);
        for t in [0, 5000, 10_000] {
            c.on_sample(&sample(None), true, clock(t));
        }
        assert!(c.status().warnings.is_empty());
        c.on_sample(&sample(None), true, clock(10_001));
        assert_eq!(c.status().warnings, ["tempMissing"]);
        c.on_sample(&sample(Some(60.0)), true, clock(11_000));
        assert!(c.status().warnings.is_empty());
    }

    fn gpu_phase(kernel: KernelId, mode: LoadMode) -> Phase {
        Phase {
            isa: Isa::Sse2,
            mode,
            duration_s: 120,
            ..phase(kernel)
        }
    }

    /// A GPU run without the service (GPU sensors do not need it, DG12).
    fn gpu_ctl(objective: Objective) -> RunController {
        gpu_ctl_with(objective, KernelId::S1)
    }

    fn gpu_ctl_with(objective: Objective, kernel: KernelId) -> RunController {
        let mut s = session();
        s.component = Component::Gpu;
        s.objective = objective;
        s.plan.phases = vec![gpu_phase(kernel, LoadMode::Steady)];
        s.plan.gpu = Some(oma_ipc::load::GpuTarget {
            luid: 7,
            integrated: false,
        });
        RunController::new(
            s,
            RunConfig {
                threshold_c: 90.0,
                thermal_stop: true,
                service_available: false,
                cores: vec![],
                apic_to_core: BTreeMap::new(),
                whea_after: None,
                whea_baseline_missing: false,
            },
            clock(0),
        )
    }

    fn gpu_progress(t: u64, rate: f64, load_percent: Option<u8>) -> LoadMessage {
        LoadMessage::Progress(Progress {
            phase: 0,
            phase_elapsed_ms: t,
            elapsed_ms: t,
            checks: 10,
            errors: 0,
            current_core: None,
            cores: vec![],
            memory_bytes: 0,
            rate: Some(rate),
            load_percent,

            disk: None,
        })
    }

    fn gpu_error(kind: ErrorKind, actual: u64, load_percent: Option<u8>) -> LoadMessage {
        LoadMessage::Error(ComputeError {
            phase: 0,
            kernel: KernelId::S1,
            isa: Isa::Sse2,
            kind,
            logical: None,
            core: None,
            iteration: 4,
            expected: 1,
            actual,
            seed: 1,
            load_percent,

            transient: None,
        })
    }

    fn gpu_sample(temp: Option<f64>, throttling: Option<bool>) -> SensorSample {
        SensorSample {
            temp_c: temp,
            power_w: Some(300.0),
            clock_mhz: Some(2700.0),
            core_clock_mhz: vec![],
            throttling,
            thermal_throttling: None,
            ..Default::default()
        }
    }

    #[test]
    fn device_lost_error_then_exit_4_is_device_lost() {
        let mut c = gpu_ctl(Objective::Normal);
        c.on_load(&gpu_progress(1000, 25.0, None), clock(1000));
        c.on_load(
            &gpu_error(ErrorKind::DeviceLost, 0x887A_0006, None),
            clock(2000),
        );
        let a = c.on_exit(Some(LOAD_EXIT_DEVICE_LOST), clock(2100));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::DeviceLost));
        assert_eq!(verdict(&c), "device_lost");
        let d = c.session().outcome_detail.as_ref().unwrap();
        assert_eq!(d.params["code"], "0x887A0006");
        assert!(c.session().events.iter().all(|e| e.code != "crashed"));

        // A failed finish after the error is no crash either.
        let mut c = gpu_ctl(Objective::Normal);
        c.on_load(&gpu_progress(1000, 25.0, None), clock(1000));
        c.on_load(
            &gpu_error(ErrorKind::DeviceLost, 0x887A_0005, None),
            clock(2000),
        );
        let a = c.on_load(&finished(FinishReason::Failed), clock(2100));
        assert!(has_finished(&settle(&mut c, a), Outcome::DeviceLost));
    }

    #[test]
    fn device_lost_error_is_kept_with_the_gpu_clock() {
        let mut c = gpu_ctl(Objective::Normal);
        c.on_load(&gpu_progress(1000, 25.0, Some(55)), clock(1000));
        c.on_sample(&gpu_sample(Some(71.0), None), false, clock(1500));
        c.on_load(
            &gpu_error(ErrorKind::DeviceLost, 0x887A_0006, Some(55)),
            clock(2000),
        );
        let e = &c.session().errors[0];
        assert_eq!(e.error.kind, ErrorKind::DeviceLost);
        assert_eq!(e.error.actual, 0x887A_0006);
        assert_eq!(e.error.load_percent, Some(55));
        assert_eq!(e.clock_mhz, Some(2700.0));
        assert_eq!(e.temp_c, Some(71.0));
        // Not counted as a mismatch: the verdict stays `device_lost`.
        assert_eq!(c.status().errors, 0);
        let a = c.on_exit(Some(LOAD_EXIT_DEVICE_LOST), clock(2100));
        assert!(has_finished(&settle(&mut c, a), Outcome::DeviceLost));
    }

    #[test]
    fn gpu_mismatch_takes_the_gpu_clock() {
        let mut c = gpu_ctl(Objective::Normal);
        c.on_load(&gpu_progress(1000, 25.0, None), clock(1000));
        c.on_sample(&gpu_sample(Some(70.0), None), false, clock(1500));
        c.on_load(&gpu_error(ErrorKind::Mismatch, 2, None), clock(2000));
        assert_eq!(c.session().errors[0].clock_mhz, Some(2700.0));
    }

    #[test]
    fn gpu_error_notice_fails_to_start() {
        let mut c = gpu_ctl(Objective::Normal);
        c.on_load(&notice("gpu_error"), clock(100));
        let a = c.on_load(&finished(FinishReason::Failed), clock(200));
        assert!(has_finished(&settle(&mut c, a), Outcome::FailedToStart));
        let d = c.session().outcome_detail.as_ref().unwrap();
        assert_eq!(d.params["reason"], "performance.start.gpu_error");
    }

    #[test]
    fn gpu_reference_invalid_has_its_own_event() {
        let mut c = gpu_ctl(Objective::Normal);
        c.on_load(&gpu_error(ErrorKind::ReferenceInvalid, 0, None), clock(100));
        assert!(c
            .session()
            .events
            .iter()
            .any(|e| e.code == "reference_invalid_gpu"));
    }

    #[test]
    fn exit_4_alone_is_device_lost() {
        let mut c = gpu_ctl(Objective::Normal);
        c.on_load(&gpu_progress(1000, 25.0, None), clock(1000));
        let a = c.on_exit(Some(4), clock(2000));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::DeviceLost));
        assert!(c
            .session()
            .outcome_detail
            .as_ref()
            .unwrap()
            .params
            .is_empty());
        assert!(c.session().events.iter().all(|e| e.code != "crashed"));
    }

    #[test]
    fn gpu_missing_notice_fails_to_start() {
        let mut c = gpu_ctl(Objective::Normal);
        c.on_load(&notice("gpu_missing"), clock(100));
        let a = c.on_load(&finished(FinishReason::Failed), clock(200));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::FailedToStart));
        let d = c.session().outcome_detail.as_ref().unwrap();
        assert_eq!(d.params["reason"], "performance.start.no_gpu");

        // After the first phase it is only an event.
        let mut c = gpu_ctl(Objective::Normal);
        c.on_load(&gpu_progress(1000, 25.0, None), clock(1000));
        c.on_load(&notice("gpu_missing"), clock(1100));
        let a = c.on_load(&finished(FinishReason::Completed), clock(1200));
        assert!(has_finished(&settle(&mut c, a), Outcome::Passed));
    }

    #[test]
    fn gpu_without_temperature_warns_and_continues() {
        let mut c = gpu_ctl(Objective::Normal);
        assert!(
            c.status().warnings.is_empty(),
            "never noService for the GPU"
        );
        assert_eq!(c.status().stop_c, Some(90.0));
        c.on_load(&gpu_progress(1000, 25.0, None), clock(1000));
        for t in [1000, 6000, 11_500] {
            assert!(c
                .on_sample(&gpu_sample(None, None), false, clock(t))
                .is_empty());
        }
        assert_eq!(c.status().warnings, ["tempMissing"]);
        assert_eq!(c.status().state, RunState::Running);
        // The thermal stop works without the service.
        c.on_sample(&gpu_sample(Some(95.0), None), false, clock(12_000));
        let a = c.on_sample(&gpu_sample(Some(96.0), None), false, clock(13_000));
        assert_eq!(a, vec![Action::SendStop]);
        assert!(!c.status().warnings.iter().any(|w| w == "noService"));
    }

    #[test]
    fn completed_gpu_run_with_low_stability_is_low_stability() {
        let mut c = gpu_ctl(Objective::Overclock);
        // 60 s windows after the 30 s warm-up: 30-90, 90-150, 150-210, 210-270.
        for s in 1..=270u64 {
            let t = s * 1000;
            let rate = if (150..210).contains(&s) { 90.0 } else { 100.0 };
            c.on_load(&gpu_progress(t, rate, None), clock(t));
            // A throttled window is dropped in overclock: not the slow one here.
            c.on_sample(&gpu_sample(Some(70.0), Some(s == 100)), false, clock(t));
        }
        assert!((c.status().stability.unwrap() - 0.9).abs() < 1e-9);
        let a = c.on_load(&finished(FinishReason::Completed), clock(270_500));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::LowStability));
        assert_eq!(verdict(&c), "low_stability");
        let s = c.session();
        assert_eq!(
            s.outcome_detail.as_ref().unwrap().params["stability"],
            "90.0"
        );
        assert!((s.stability.unwrap() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn s3_rates_count_for_stability() {
        let mut c = gpu_ctl_with(Objective::Overclock, KernelId::S3);
        for s in 1..=270u64 {
            let t = s * 1000;
            let rate = if (150..210).contains(&s) { 90.0 } else { 100.0 };
            c.on_load(&gpu_progress(t, rate, None), clock(t));
        }
        assert!((c.status().stability.unwrap() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn pcie_replay_increase_warns_once() {
        let mut c = gpu_ctl(Objective::Normal);
        let replays = |c: &RunController| {
            c.session()
                .events
                .iter()
                .filter(|e| e.code == "pcie_replay")
                .count()
        };
        c.on_pcie_replay(5, clock(0));
        c.on_pcie_replay(5, clock(5000));
        assert!(c.status().warnings.is_empty());
        assert_eq!(replays(&c), 0);
        c.on_pcie_replay(7, clock(10_000));
        c.on_pcie_replay(9, clock(15_000));
        assert_eq!(c.status().warnings, ["pcieReplay"]);
        assert_eq!(replays(&c), 1);
        // Only a warning: the run still passes.
        c.on_load(&gpu_progress(16_000, 25.0, None), clock(16_000));
        let a = c.on_load(&finished(FinishReason::Completed), clock(17_000));
        assert!(has_finished(&settle(&mut c, a), Outcome::Passed));
    }

    #[test]
    fn status_carries_the_gpu_device_id_only_for_gpu_runs() {
        let config = || RunConfig {
            threshold_c: 90.0,
            thermal_stop: true,
            service_available: false,
            cores: vec![],
            apic_to_core: BTreeMap::new(),
            whea_after: None,
            whea_baseline_missing: false,
        };
        let mut gpu = session();
        gpu.component = Component::Gpu;
        gpu.gpu_device_id = Some("gpu/pci-0000:01:00.0".into());
        let gpu = RunController::new(gpu, config(), clock(0));
        assert_eq!(
            gpu.status().gpu_device_id.as_deref(),
            Some("gpu/pci-0000:01:00.0")
        );
        let cpu = RunController::new(session(), config(), clock(0));
        assert_eq!(cpu.status().gpu_device_id, None);
    }

    #[test]
    fn error_keeps_the_load_percent() {
        let mut c = gpu_ctl(Objective::Normal);
        c.on_load(&gpu_progress(1000, 25.0, Some(60)), clock(1000));
        assert_eq!(c.status().load_percent, Some(60));
        c.on_load(&gpu_error(ErrorKind::Mismatch, 2, Some(45)), clock(2000));
        assert_eq!(c.session().errors[0].error.load_percent, Some(45));
        let v = serde_json::to_value(&c.session().errors[0]).unwrap();
        assert_eq!(v["load_percent"], 45);
        assert_eq!(c.status().errors, 1);
    }

    #[test]
    fn vram_reduced_notice_warns() {
        let mut c = gpu_ctl(Objective::Overclock);
        c.on_load(&notice("vram_reduced"), clock(1000));
        c.on_load(&notice("vram_bits"), clock(1100));
        assert_eq!(c.status().warnings, ["vramReduced"]);
        assert!(c.session().events.iter().any(|e| e.code == "vram_bits"));
    }

    // ---- Disk (M8c) ----

    fn disk_ctl(thermal: bool) -> RunController {
        disk_ctl_with(thermal, KernelId::N2)
    }

    fn disk_ctl_with(thermal: bool, kernel: KernelId) -> RunController {
        let mut s = session();
        s.component = Component::Disk;
        s.request.component = Component::Disk;
        s.request.disk = Some(crate::load::plan::DiskStart {
            folder: "D:\\oma".into(),
            wake: false,
        });
        s.plan.phases = vec![Phase {
            isa: Isa::Sse2,
            stop_on_error: true,
            disk: Some(oma_ipc::load::DiskJob {
                block_bytes: 4096,
                seq_block_bytes: 1 << 20,
                random_percent: 0,
                read_percent: 0,
                queue: 4,
                threads: 1,
                write_cap_bytes: None,
                cycles: None,
                rate_limit_bps: None,
            }),
            ..phase(kernel)
        }];
        s.plan.disk = Some(oma_ipc::load::DiskTarget {
            dir: "D:\\oma".into(),
            file_bytes: 1 << 30,
            compressible: false,
            reserve_bytes: 1 << 30,
        });
        s.disk = Some(crate::load::session::DiskSession {
            device_id: "disk/nvme-0".into(),
            volume: "D:".into(),
            ..Default::default()
        });
        RunController::new(
            s,
            RunConfig {
                threshold_c: 70.0,
                thermal_stop: thermal,
                service_available: false,
                cores: vec![],
                apic_to_core: BTreeMap::new(),
                whea_after: None,
                whea_baseline_missing: false,
            },
            clock(0),
        )
    }

    fn disk_progress(t: u64, read: u64, written: u64, rbps: f64, wbps: f64) -> LoadMessage {
        LoadMessage::Progress(Progress {
            phase: 0,
            phase_elapsed_ms: t,
            elapsed_ms: t,
            checks: 10,
            errors: 0,
            current_core: None,
            cores: vec![],
            memory_bytes: 0,
            rate: None,
            load_percent: None,
            disk: Some(oma_ipc::load::DiskProgress {
                read_bps: rbps,
                write_bps: wbps,
                read_bytes: read,
                written_bytes: written,
                iops: 1.0,
            }),
        })
    }

    fn disk_error(kind: ErrorKind, transient: Option<bool>) -> LoadMessage {
        LoadMessage::Error(ComputeError {
            phase: 0,
            kernel: KernelId::N2,
            isa: Isa::Sse2,
            kind,
            logical: None,
            core: None,
            iteration: 9,
            expected: 1,
            actual: 2,
            seed: 1,
            load_percent: None,
            transient,
        })
    }

    fn disk_notice(code: &str, value: Option<u64>) -> LoadMessage {
        LoadMessage::Notice(Notice {
            phase: 0,
            code: code.into(),
            value,
        })
    }

    fn disk_sample(temp: Option<f64>, host: Option<f64>) -> SensorSample {
        SensorSample {
            temp_c: temp,
            io_bps: Some(1e6),
            host_written_gib: host,
            ..Default::default()
        }
    }

    #[test]
    fn disk_full_notice_is_stopped_disk_full() {
        let mut c = disk_ctl(true);
        c.on_load(&disk_progress(1000, 0, 5, 0.0, 5.0), clock(1000));
        c.on_load(&disk_notice("disk_full", None), clock(2000));
        let a = c.on_load(&finished(FinishReason::Failed), clock(2100));
        let a = settle(&mut c, a);
        assert!(has_finished(&a, Outcome::StoppedDiskFull));
        assert_eq!(verdict(&c), "stopped_disk_full");
        assert!(c.session().events.iter().all(|e| e.code != "crashed"));
        // No warning about the service for a disk run.
        assert!(!c.status().warnings.contains(&"noService".to_string()));
    }

    #[test]
    fn exit_5_after_io_error_is_errors_not_crashed() {
        for finish_first in [false, true] {
            let mut c = disk_ctl(true);
            c.on_load(&disk_progress(1000, 0, 5, 0.0, 5.0), clock(1000));
            c.on_load(&disk_error(ErrorKind::IoError, Some(false)), clock(2000));
            let a = if finish_first {
                c.on_load(&finished(FinishReason::Failed), clock(2100))
            } else {
                c.on_exit(Some(LOAD_EXIT_IO), clock(2100))
            };
            let a = settle(&mut c, a);
            assert!(has_finished(&a, Outcome::Errors), "{finish_first}");
            assert_eq!(verdict(&c), "errors");
            assert_eq!(c.status().errors, 1);
            assert!(c.session().events.iter().all(|e| e.code != "crashed"));
        }
        // Exit 5 with no message counted still ends as an error.
        let mut c = disk_ctl(true);
        c.on_load(&disk_progress(1000, 0, 5, 0.0, 5.0), clock(1000));
        let a = c.on_exit(Some(LOAD_EXIT_IO), clock(2000));
        assert!(has_finished(&settle(&mut c, a), Outcome::Errors));
    }

    #[test]
    fn data_errors_are_coreless_errors() {
        let mut c = disk_ctl(false);
        c.on_load(&disk_progress(1000, 0, 5, 0.0, 5.0), clock(1000));
        for k in [
            ErrorKind::BitFlip,
            ErrorKind::Misplaced,
            ErrorKind::Stale,
            ErrorKind::Zeros,
        ] {
            c.on_load(&disk_error(k, None), clock(1500));
        }
        let s = c.session();
        assert_eq!(s.errors.len(), 4);
        assert!(s.cores.is_empty());
        assert_eq!(c.status().errors, 4);
        let a = c.on_load(&finished(FinishReason::Stopped), clock(2000));
        assert!(has_finished(&settle(&mut c, a), Outcome::Errors));
        assert_eq!(verdict(&c), "errors");
    }

    #[test]
    fn a_data_error_stops_the_run_when_the_phase_says_so() {
        // oma-load keeps going after a data error: `stop_on_error` is ours (R8).
        let mut c = disk_ctl(false);
        c.on_load(&disk_progress(1000, 0, 5, 0.0, 5.0), clock(1000));
        let a = c.on_load(&disk_error(ErrorKind::Stale, Some(true)), clock(1500));
        assert!(a.contains(&Action::SendStop), "{a:?}");
        assert_eq!(c.status().state, RunState::Stopping);
        let a = c.on_load(&finished(FinishReason::Stopped), clock(1600));
        assert!(has_finished(&settle(&mut c, a), Outcome::Errors));
        // Once only.
        let mut c = disk_ctl(false);
        c.on_load(&disk_error(ErrorKind::Zeros, None), clock(1000));
        let a = c.on_load(&disk_error(ErrorKind::Zeros, None), clock(1100));
        assert!(!a.contains(&Action::SendStop));
    }

    #[test]
    fn access_denied_before_the_first_phase_fails_to_start() {
        let mut c = disk_ctl(true);
        c.on_load(&disk_notice("access_denied", None), clock(500));
        let a = c.on_load(&finished(FinishReason::Failed), clock(600));
        assert!(has_finished(&settle(&mut c, a), Outcome::FailedToStart));
        let d = c.session().outcome_detail.as_ref().unwrap();
        assert_eq!(d.params["reason"], "performance.start.access_denied");
    }

    #[test]
    fn disk_thermal_stop_works_without_the_service() {
        let mut c = disk_ctl(true);
        c.on_load(&disk_progress(1000, 0, 5, 0.0, 5.0), clock(1000));
        assert!(c
            .on_sample(&disk_sample(Some(75.0), Some(1.0)), false, clock(1000))
            .is_empty());
        let a = c.on_sample(&disk_sample(Some(76.0), Some(1.0)), false, clock(2000));
        assert!(a.contains(&Action::SendStop), "{a:?}");
        assert_eq!(c.status().stop_c, Some(70.0));
        let a = c.on_load(&finished(FinishReason::Stopped), clock(2500));
        assert!(has_finished(&settle(&mut c, a), Outcome::StoppedThermal));
        // Off when the user turned the guard off.
        let mut c = disk_ctl(false);
        c.on_sample(&disk_sample(Some(95.0), None), false, clock(1000));
        assert!(c
            .on_sample(&disk_sample(Some(95.0), None), false, clock(2000))
            .iter()
            .all(|a| *a != Action::SendStop));
    }

    #[test]
    fn missing_host_written_warns_smart_missing() {
        let mut c = disk_ctl(false);
        c.on_sample(&disk_sample(Some(40.0), None), false, clock(1000));
        assert!(c.status().warnings.contains(&"smartMissing".to_string()));
        // It reads later: the warning goes.
        c.on_sample(&disk_sample(Some(40.0), Some(3.0)), false, clock(2000));
        assert!(!c.status().warnings.contains(&"smartMissing".to_string()));
        // A later gap does not bring it back.
        c.on_sample(&disk_sample(Some(40.0), None), false, clock(3000));
        assert!(!c.status().warnings.contains(&"smartMissing".to_string()));
    }

    #[test]
    fn slc_cliff_after_a_hot_sample_is_thermal_suspect() {
        let mut c = disk_ctl(false);
        c.on_load(&disk_progress(1000, 0, 5, 0.0, 5.0), clock(1000));
        // 65 = threshold 70 - 5, 8 s before the notice.
        c.on_sample(&disk_sample(Some(65.0), None), false, clock(20_000));
        c.on_load(&disk_notice("slc_cliff", Some(4 << 30)), clock(28_000));
        c.on_load(&disk_notice("slc_steady", Some(600_000_000)), clock(60_000));
        let slc = c.session().disk.as_ref().unwrap().slc.clone().unwrap();
        assert_eq!(slc.cache_bytes, 4 << 30);
        assert_eq!(slc.steady_bps, Some(600_000_000.0));
        assert!(slc.thermal_suspect);
    }

    #[test]
    fn slc_cliff_when_cool_is_not() {
        // Cool sample, and a hot one more than 10 s before the notice.
        let mut c = disk_ctl(false);
        c.on_sample(&disk_sample(Some(69.0), None), false, clock(1000));
        c.on_sample(&disk_sample(Some(64.0), None), false, clock(15_000));
        c.on_load(&disk_notice("slc_cliff", Some(1)), clock(20_000));
        let slc = c.session().disk.as_ref().unwrap().slc.clone().unwrap();
        assert!(!slc.thermal_suspect);
        // A steady notice without a cliff records nothing.
        let mut c = disk_ctl(false);
        c.on_load(&disk_notice("slc_steady", Some(5)), clock(1000));
        assert!(c.session().disk.as_ref().unwrap().slc.is_none());
    }

    #[test]
    fn disk_bytes_and_smart_delta_reach_the_session() {
        let mut c = disk_ctl(false);
        c.on_sample(&disk_sample(Some(40.0), None), false, clock(500));
        c.on_sample(&disk_sample(Some(40.0), Some(100.0)), false, clock(1000));
        c.on_load(&disk_progress(1000, 10, 20, 1.0, 2.0), clock(1000));
        c.on_sample(&disk_sample(Some(40.0), Some(101.5)), false, clock(2000));
        c.on_load(&disk_progress(2000, 30, 70, 1.0, 2.0), clock(2000));
        c.on_sample(&disk_sample(Some(40.0), Some(103.0)), false, clock(3000));
        c.on_sample(&disk_sample(Some(40.0), None), false, clock(4000));
        let d = c.session().disk.clone().unwrap();
        assert_eq!((d.read_bytes, d.written_bytes), (30, 70));
        assert_eq!(d.host_written_before_gib, Some(100.0));
        assert_eq!(d.host_written_after_gib, Some(103.0));
    }

    #[test]
    fn disk_status_carries_the_rates() {
        let mut c = disk_ctl(false);
        assert_eq!(c.status().disk, None);
        c.on_load(&disk_progress(1000, 30, 70, 1.5e9, 2.5e9), clock(1000));
        let d = c.status().disk.unwrap();
        assert_eq!(
            (d.read_bps, d.write_bps, d.read_bytes, d.written_bytes),
            (1.5e9, 2.5e9, 30, 70)
        );
        // A CPU run has none.
        assert_eq!(ctl(false, true).status().disk, None);
    }

    #[test]
    fn the_journal_names_the_disk_folder() {
        let mut c = disk_ctl(false);
        let a = c.on_load(&disk_progress(1000, 0, 5, 0.0, 5.0), clock(1000));
        let j = a
            .iter()
            .find_map(|a| match a {
                Action::WriteJournal(j) => Some(j.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(j.disk_folder.as_deref(), Some("D:\\oma"));
    }

    fn disk_two_phase_ctl(first: KernelId, second: KernelId) -> RunController {
        let mut c = disk_ctl_with(false, first);
        let mut p2 = c.session.plan.phases[0].clone();
        p2.kernel = second;
        c.session.plan.phases.push(p2);
        c
    }

    fn disk_progress_in(phase: u32, in_phase: u64) -> LoadMessage {
        let LoadMessage::Progress(mut p) = disk_progress(0, 0, 5, 0.0, 5.0) else {
            unreachable!()
        };
        p.phase = phase;
        p.phase_elapsed_ms = in_phase;
        LoadMessage::Progress(p)
    }

    #[test]
    fn a_strict_disk_phase_is_timed_from_its_own_start() {
        // A fill 2000 s past its time, then an N1 phase (60 s) on time: not hung.
        let fill_end = 60_000 + 2_000_000;
        let mut c = disk_two_phase_ctl(KernelId::DiskFill, KernelId::N1);
        c.on_load(&disk_progress_in(0, 1000), clock(1000));
        c.on_load(&disk_progress_in(0, fill_end), clock(fill_end));
        c.on_load(&disk_progress_in(1, 1000), clock(fill_end + 1000));
        let t = fill_end + 30_000;
        c.on_load(&disk_progress_in(1, 30_000), clock(t));
        assert!(!c.on_clock(clock(t)).contains(&Action::Kill));
        // The same phase overrunning its own duration is hung.
        let t = fill_end + 60_000 + OVERRUN_MS + 5_000;
        c.on_load(&disk_progress_in(1, t - fill_end), clock(t));
        assert!(c.on_clock(clock(t)).contains(&Action::Kill));
        assert!(c.hung);
    }

    #[test]
    fn heartbeats_do_not_reset_a_strict_phase_clock() {
        let mut c = disk_ctl_with(false, KernelId::N2);
        c.on_load(&disk_progress_in(0, 1000), clock(1000));
        // Stuck in an open or a flush: only `phase_elapsed_ms = 0` heartbeats arrive.
        let mut t = 1000;
        while t + 900 <= 60_000 + OVERRUN_MS {
            t += 900;
            c.on_load(&disk_progress_in(0, 0), clock(t));
            assert!(!c.on_clock(clock(t)).contains(&Action::Kill), "{t}");
        }
        t += 5_000;
        c.on_load(&disk_progress_in(0, 0), clock(t));
        assert!(c.on_clock(clock(t)).contains(&Action::Kill));
        assert!(c.hung);
    }

    #[test]
    fn only_soft_disk_phases_may_outlast_their_nominal_duration() {
        // R9: a strict phase whose Progress ticker is alive is still hung past its time.
        let late = 60 * 1000 + OVERRUN_MS + 50_000;
        for (kernel, hung) in [
            (KernelId::N2, true),
            (KernelId::DiskBench, true),
            (KernelId::DiskFill, false),
            (KernelId::V1, false),
            (KernelId::V4, false),
        ] {
            let mut c = disk_ctl_with(false, kernel);
            c.on_load(&disk_progress(1000, 0, 5, 0.0, 5.0), clock(1000));
            c.on_load(
                &disk_progress(late - 100, 0, 5, 0.0, 5.0),
                clock(late - 100),
            );
            let a = c.on_clock(clock(late));
            assert_eq!(a.contains(&Action::Kill), hung, "{kernel:?}");
            assert_eq!(c.hung, hung, "{kernel:?}");
        }
    }
}
