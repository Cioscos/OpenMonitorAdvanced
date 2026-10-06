//! The stress test runner (M8a1): one test at a time (the CPU benchmark of
//! [`super::bench`] shares the slot, DB8), on its own thread
//! (`oma-perf-runner`), driving the pure `RunController` with the helper's
//! messages, the sampler's CPU readings, WHEA polls and a 250 ms clock, and
//! executing its `Action`s: pipe, Job, store, toast.
//!
//! The thread owns everything the test needs: the keep-awake request (it
//! belongs to the thread that makes it, DA15), the helper and the controller.
//! The rest of the app sees a [`RunStatus`] copy, refreshed by the thread.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, Once, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use oma_core::engine::TickOutput;
use oma_core::load::{
    build_plan, core_order, cpu_stop_threshold, decide, ram_budget, read_sample,
    resolve_cpu_sensors, Action, BuildError, BuildInput, Clock, Component, CpuSensorIds, Objective,
    OutcomeDetail, OutcomeFacts, Preset, RunConfig, RunController, RunState, RunStatus,
    SensorSample, Session, StartRequest, VerdictKey, WheaEvent, FORMAT,
};
use oma_core::model::Schema;
use oma_core::scores::BenchStatus;
use oma_ipc::load::{Isa, LoadHello, LoadMessage, Plan, RunRequest, StopRequest, Topology};

use super::host::{HostEvent, StartFailure};
use super::store::{to_rfc3339, PerformanceStore};
use crate::i18n::{t, Lang};
use crate::notifier::{launch_for_performance, ToastSink};
use crate::settings::SettingsStore;
use crate::tray::language_for;

/// Emitted to the main window on every state change and once a second during a
/// test, only while a window is open; the payload is a [`RunStatus`].
pub const EVENT_STATUS: &str = "performance-status";

pub(super) const TICK: Duration = Duration::from_millis(250);
const STATUS_EVERY: Duration = Duration::from_secs(1);
/// How long an `Exited` waits for the pipe's `Closed` (the messages before it).
pub(super) const EXIT_GRACE: Duration = Duration::from_secs(1);
/// Samples waiting for the thread; more are dropped, the sampler never waits.
pub(super) const SAMPLE_QUEUE: usize = 4;

/// The helper as the runner drives it: [`super::host::LoadHost`], or a script in the tests.
pub trait LoadLink {
    fn send(&self, msg: &LoadMessage) -> io::Result<()>;
    fn hello(&self) -> &LoadHello;
    fn topology(&self) -> &Topology;
    /// Closes the pipe and the Job, which kills the helper.
    fn kill(&mut self);
}

impl LoadLink for super::host::LoadHost {
    fn send(&self, msg: &LoadMessage) -> io::Result<()> {
        super::host::LoadHost::send(self, msg)
    }
    fn hello(&self) -> &LoadHello {
        super::host::LoadHost::hello(self)
    }
    fn topology(&self) -> &Topology {
        super::host::LoadHost::topology(self)
    }
    fn kill(&mut self) {
        super::host::LoadHost::kill(self);
    }
}

/// Starts the helper, which sends its events to the sender (on the runner's thread).
pub type Launcher =
    Box<dyn Fn(Sender<HostEvent>) -> Result<Box<dyn LoadLink>, StartFailure> + Send + Sync>;

/// `oma-load.exe` next to the app, with `OMA_LOAD_INJECT` in debug builds (DA18).
pub fn load_host_launcher() -> Launcher {
    Box::new(|events| {
        let exe = std::env::current_exe().map_err(StartFailure::Spawn)?;
        let host = super::host::LoadHost::start(
            &super::host::load_exe(&exe),
            super::host::inject_from_env(),
            events,
        )?;
        Ok(Box::new(host) as Box<dyn LoadLink>)
    })
}

/// What the runner reads from the machine; faked in the tests.
pub trait Machine: Send + Sync {
    fn topology(&self) -> io::Result<Topology>;
    /// The instruction sets this CPU runs, best first.
    fn isa(&self) -> Vec<Isa>;
    /// Total and available physical memory, in bytes.
    fn memory(&self) -> io::Result<(u64, u64)>;
    /// The newest WHEA record already in the System log.
    fn latest_whea(&self) -> io::Result<Option<u64>>;
    fn whea_after(&self, after: Option<u64>) -> io::Result<Vec<WheaEvent>>;
    /// Time the PC has slept since boot (DA15).
    fn asleep_ms(&self) -> u64;
    /// Whether the PC runs on battery; `None` when unknown.
    fn on_battery(&self) -> Option<bool>;
    /// Polls the other processes' share of the whole CPU (0-1); each call is
    /// one poll, the first may give `None`. Made and used on the runner's thread.
    fn busy_probe(&self, logical: u32) -> Box<dyn FnMut() -> Option<f64>>;
}

/// This PC.
pub struct WinMachine;

impl Machine for WinMachine {
    fn topology(&self) -> io::Result<Topology> {
        oma_win::topology::read()
    }

    /// As `oma-load` detects them.
    fn isa(&self) -> Vec<Isa> {
        oma_ipc::load::detected_isa()
    }

    fn memory(&self) -> io::Result<(u64, u64)> {
        oma_win::memory::memory_status()
    }

    fn latest_whea(&self) -> io::Result<Option<u64>> {
        oma_win::eventlog::latest_record_id()
    }

    fn whea_after(&self, after: Option<u64>) -> io::Result<Vec<WheaEvent>> {
        Ok(oma_win::eventlog::whea_after(after)?
            .iter()
            .filter_map(oma_win::eventlog::SystemEvent::whea)
            .collect())
    }

    fn asleep_ms(&self) -> u64 {
        oma_win::power::asleep_ms()
    }

    fn on_battery(&self) -> Option<bool> {
        oma_win::power::on_battery()
    }

    fn busy_probe(&self, logical: u32) -> Box<dyn FnMut() -> Option<f64>> {
        match oma_win::proc_cpu::OtherCpu::open() {
            Ok(mut other) => Box::new(move || other.sample(logical)),
            Err(err) => {
                tracing::warn!(%err, "other processes' CPU unreadable");
                Box::new(|| None)
            }
        }
    }
}

/// What the runner is given.
pub struct RunnerDeps {
    pub store: Arc<PerformanceStore>,
    pub settings: Arc<SettingsStore>,
    pub machine: Box<dyn Machine>,
    pub launcher: Launcher,
    pub toaster: Box<dyn ToastSink + Sync>,
    /// The engine's current schema (Tjmax at the start).
    pub schema: Box<dyn Fn() -> Schema + Send + Sync>,
    /// Whether the service is connected (the thermal stop needs it).
    pub service_available: Box<dyn Fn() -> bool + Send + Sync>,
    /// Whether the main window, the only one with the Performance view, is open.
    pub window_open: Box<dyn Fn() -> bool + Send + Sync>,
    /// Whether it is open, shown and not minimised (the benchmark toast, DB9).
    pub window_visible: Box<dyn Fn() -> bool + Send + Sync>,
    /// Sends [`EVENT_STATUS`]; called only while a window is open.
    pub emit: Box<dyn Fn(&RunStatus) + Send + Sync>,
    /// Every state change, window or not (the tray).
    pub on_state: Box<dyn Fn(&RunStatus) + Send + Sync>,
    /// Sends [`super::bench::EVENT_BENCH`]; called only while a window is open.
    pub emit_bench: Box<dyn Fn(&BenchStatus) + Send + Sync>,
    /// Every benchmark state change, window or not (the tray).
    pub on_bench: Box<dyn Fn(&BenchStatus) + Send + Sync>,
    pub app_version: String,
}

#[derive(Debug)]
pub enum StartError {
    /// A stress test or a benchmark is already running.
    Busy,
    Plan(BuildError),
    /// The topology, the session id or the thread could not be had.
    System(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => write!(f, "a test is already running"),
            Self::Plan(e) => write!(f, "{e}"),
            Self::System(e) => write!(f, "{e}"),
        }
    }
}

impl StartError {
    /// What the commands send to the UI: `busy`, `build:<code>` for a plan that cannot be built
    /// (the UI translates both), or the text of anything else.
    pub fn wire(&self) -> String {
        let code = match self {
            Self::Busy => return "busy".into(),
            Self::System(e) => return e.clone(),
            Self::Plan(BuildError::NoCores) => "no_cores",
            Self::Plan(BuildError::NoPhases) => "no_phases",
            Self::Plan(BuildError::TooLong) => "too_long",
            Self::Plan(BuildError::UnknownCore(_)) => "unknown_core",
            Self::Plan(BuildError::RamBudget) => "ram_budget",
        };
        format!("build:{code}")
    }
}

/// What the machine offers for a test (`performance_system`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub cpu_model: String,
    pub logical: u32,
    pub cores: u32,
    pub isa: Vec<Isa>,
    pub ram_total: u64,
    /// The RAM share a test may use now (DA10).
    pub ram_budget: u64,
    pub service_connected: bool,
    pub tjmax_c: Option<f64>,
    /// The thermal stop threshold (DA5).
    pub stop_c: f64,
    pub hypervisor: bool,
}

/// Asks the thread to stop; `deadline` bounds the wait for the helper (DA16).
#[derive(Default)]
pub(super) struct Control {
    pub(super) stop: AtomicBool,
    pub(super) deadline: Mutex<Option<Instant>>,
}

impl Control {
    pub(super) fn deadline(&self) -> Option<Instant> {
        *self.deadline.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The test or benchmark in progress (or the last one, its thread finished).
pub(super) struct Active {
    pub(super) thread: JoinHandle<()>,
    pub(super) control: Arc<Control>,
    pub(super) samples: SyncSender<(SensorSample, bool)>,
    /// Physical cores, for the per-core clocks.
    pub(super) cores: usize,
    /// The schema revision the ids were resolved on.
    pub(super) sensors: Option<(u64, CpuSensorIds)>,
    /// The CPU benchmark, not a stress test.
    pub(super) bench: bool,
}

impl Active {
    pub(super) fn running(&self) -> bool {
        !self.thread.is_finished()
    }
}

pub struct PerformanceRunner {
    pub(super) deps: Arc<RunnerDeps>,
    pub(super) active: Mutex<Option<Active>>,
    status: Arc<Mutex<RunStatus>>,
    /// The benchmark in progress or the last one; `None` before any.
    pub(super) bench_status: Arc<Mutex<Option<BenchStatus>>>,
    /// The journal of an earlier run is turned into a session before any start.
    recovery: Once,
}

pub(super) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The status without a test.
fn idle_status() -> RunStatus {
    RunStatus {
        state: RunState::Idle,
        session_id: String::new(),
        component: Component::Cpu,
        objective: Objective::Normal,
        preset: Preset::Quick,
        elapsed_ms: 0,
        total_ms: 0,
        phase_index: 0,
        phases: vec![],
        temp_c: None,
        temp_max_c: None,
        stop_c: None,
        power_w: None,
        clock_mhz: None,
        checks: 0,
        errors: 0,
        whea_corrected: 0,
        whea_fatal: 0,
        cores: vec![],
        current_core: None,
        events: vec![],
        warnings: vec![],
        outcome: None,
    }
}

/// Core numbers run from 0 (DA4): the highest plus one.
pub(super) fn core_count(topology: &Topology) -> usize {
    topology
        .logical
        .iter()
        .map(|l| l.core as usize + 1)
        .max()
        .unwrap_or(0)
}

/// The uuid's first 64 bits are random enough for the data seed.
pub(super) fn seed_of(id: &str) -> u64 {
    u64::from_str_radix(&id.replace('-', "")[..16], 16).unwrap_or(1)
}

/// The clock of a runner thread started at `epoch`.
pub(super) fn clock_at(epoch: Instant, machine: &dyn Machine) -> Clock {
    Clock {
        mono_ms: epoch.elapsed().as_millis() as u64,
        wall_ms: unix_now_ms(),
        asleep_ms: machine.asleep_ms(),
    }
}

/// The `Run` message of `plan`, refused when it would not pass the helper's validation.
pub(super) fn plan_message(plan: Plan) -> Result<LoadMessage, StartFailure> {
    let msg = LoadMessage::Run(RunRequest { plan });
    msg.validate().map_err(|err| {
        tracing::error!(%err, "the stress plan is not valid");
        StartFailure::InvalidPlan
    })?;
    Ok(msg)
}

/// A verdict without the place it happened.
fn bare_detail(key: VerdictKey) -> OutcomeDetail {
    OutcomeDetail {
        verdict: key.key.to_string(),
        params: key.params,
        phase: None,
        kernel: None,
        core: None,
        temp_c: None,
        clock_mhz: None,
        at_ms: None,
    }
}

/// The runner thread panicked: the session (as last saved) ends as `crashed`, and the
/// status as finished, so the tray mark goes and a new test can start.
fn after_panic(deps: &RunnerDeps, status: &Mutex<RunStatus>, started: Session) {
    tracing::error!("the stress test runner panicked");
    let store = &deps.store;
    let mut session = store.load(&started.id).ok().flatten().unwrap_or(started);
    if session.outcome.is_none() {
        let (outcome, key) = decide(&OutcomeFacts {
            crashed: true,
            ..Default::default()
        });
        session.outcome = Some(outcome);
        session.outcome_detail = Some(bare_detail(key));
        session.ended_at = Some(to_rfc3339(unix_now_ms()));
        if let Err(err) = store.save(&session) {
            tracing::warn!(%err, "cannot save the stress session");
        }
    }
    if let Err(err) = store.delete_journal() {
        tracing::warn!(%err, "cannot delete the stress journal");
    }
    let mut st = lock(status).clone();
    st.state = RunState::Finished;
    st.outcome = session.outcome;
    (deps.on_state)(&st);
    if (deps.window_open)() {
        (deps.emit)(&st);
    }
    *lock(status) = st;
}

/// The T3 text of a session's verdict.
fn verdict_text(lang: Lang, detail: &OutcomeDetail) -> String {
    // A `failed_to_start` reason is a text, or the key of one (`performance.start.*`).
    let texts: Vec<(&str, String)> = detail
        .params
        .iter()
        .map(|(k, v)| {
            let text = if v.starts_with("performance.start.") {
                t(lang, v, &[])
            } else {
                v.clone()
            };
            (k.as_str(), text)
        })
        .collect();
    let params: Vec<(&str, &str)> = texts.iter().map(|(k, v)| (*k, v.as_str())).collect();
    t(
        lang,
        &format!("performance.outcome.{}", detail.verdict),
        &params,
    )
}

impl PerformanceRunner {
    pub fn new(deps: RunnerDeps) -> Self {
        Self {
            deps: Arc::new(deps),
            active: Mutex::new(None),
            status: Arc::new(Mutex::new(idle_status())),
            bench_status: Arc::new(Mutex::new(None)),
            recovery: Once::new(),
        }
    }

    pub fn store(&self) -> &PerformanceStore {
        &self.deps.store
    }

    fn lang(&self) -> Lang {
        language_for(self.deps.settings.snapshot().general.language)
    }

    /// Prunes the history and recovers a test the last run left open (DA14),
    /// once: a toast opens its result. Starts wait for it.
    pub fn recover(&self) {
        // A panic in here must not poison the `Once` for every later start.
        self.recovery.call_once(|| {
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.recover_now()));
            if run.is_err() {
                tracing::error!("the stress test recovery panicked");
            }
        });
    }

    fn recover_now(&self) {
        let store = &self.deps.store;
        store.prune_now();
        let recovered = store.recover(
            oma_win::power::boot_time_unix_ms(),
            oma_win::eventlog::crash_evidence,
            &self.deps.app_version,
        );
        if let Some(summary) = recovered {
            let lang = self.lang();
            self.deps.toaster.show(
                t(lang, "performance.toast.title", &[]),
                t(lang, "performance.toast.recovered", &[]),
                launch_for_performance(&summary.id),
            );
        }
    }

    /// The plan for `request` on this machine, with its topology.
    fn plan(&self, request: &StartRequest, seed: u64) -> Result<(Topology, Plan), StartError> {
        let m = &self.deps.machine;
        let topology = m
            .topology()
            .map_err(|e| StartError::System(e.to_string()))?;
        let perf = self.deps.settings.snapshot().performance.clone();
        let available = m.memory().map_or_else(
            |err| {
                tracing::warn!(%err, "memory status unreadable: no RAM share");
                0
            },
            |(_, available)| available,
        );
        let plan = build_plan(&BuildInput {
            request,
            topology: &topology,
            isa: &m.isa(),
            ram_budget: ram_budget(available, perf.ram_share_percent),
            stop_override: perf.stop_on_first_error,
            seed,
        })
        .map_err(StartError::Plan)?;
        Ok((topology, plan))
    }

    /// The plan a start would run (the seed aside).
    pub fn preview(&self, request: &StartRequest) -> Result<Plan, StartError> {
        Ok(self.plan(request, 0)?.1)
    }

    pub fn system(&self) -> SystemInfo {
        let m = &self.deps.machine;
        let topology = m.topology().unwrap_or_else(|err| {
            tracing::warn!(%err, "CPU topology unreadable");
            Topology {
                logical: vec![],
                caches: oma_ipc::load::CacheSizes {
                    l1d_bytes: 0,
                    l2_bytes: 0,
                    l2_shared_by: 0,
                    l3_bytes: 0,
                    l3_total_bytes: 0,
                },
                hypervisor: false,
                vendor: String::new(),
                brand: String::new(),
            }
        });
        let (ram_total, available) = m.memory().unwrap_or((0, 0));
        let perf = self.deps.settings.snapshot().performance.clone();
        let tjmax_c = resolve_cpu_sensors(&(self.deps.schema)(), 0).tjmax_c;
        SystemInfo {
            cpu_model: topology.brand.clone(),
            logical: topology.logical.len() as u32,
            cores: core_count(&topology) as u32,
            isa: m.isa(),
            ram_total,
            ram_budget: ram_budget(available, perf.ram_share_percent),
            service_connected: (self.deps.service_available)(),
            tjmax_c,
            stop_c: cpu_stop_threshold(perf.cpu_stop_c, tjmax_c),
            hypervisor: topology.hypervisor,
        }
    }

    /// Builds the session and its plan and starts the `oma-perf-runner`
    /// thread; returns the session id. A helper that does not start still
    /// gives a saved `failed_to_start` session.
    pub fn start(&self, request: StartRequest) -> Result<String, StartError> {
        self.recover();
        let mut active = lock(&self.active);
        if active.as_ref().is_some_and(Active::running) {
            return Err(StartError::Busy);
        }
        if let Some(old) = active.take() {
            let _ = old.thread.join();
        }
        let id = oma_win::overlay_pipe::random_uuid_v4()
            .map_err(|e| StartError::System(e.to_string()))?;
        let seed = seed_of(&id);
        let (topology, plan) = self.plan(&request, seed)?;
        let perf = self.deps.settings.snapshot().performance.clone();
        let tjmax_c = resolve_cpu_sensors(&(self.deps.schema)(), 0).tjmax_c;
        let config = RunConfig {
            threshold_c: cpu_stop_threshold(perf.cpu_stop_c, tjmax_c),
            thermal_stop: perf.thermal_stop,
            service_available: (self.deps.service_available)(),
            cores: core_order(&topology),
            apic_to_core: BTreeMap::new(),
            whea_after: None,
            whea_baseline_missing: false,
        };
        let device = match request.component {
            Component::Cpu => topology.brand.clone(),
            Component::Ram => {
                let total = self.deps.machine.memory().map_or(0, |(t, _)| t);
                format!("{} GB RAM", (total + (1 << 29)) >> 30)
            }
        };
        let session = Session {
            format: FORMAT,
            id: id.clone(),
            started_at: to_rfc3339(unix_now_ms()),
            ended_at: None,
            component: request.component,
            device,
            objective: request.objective,
            preset: request.preset,
            request,
            plan,
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
            app_version: self.deps.app_version.clone(),
            load_version: None,
        };
        let zero = Clock {
            mono_ms: 0,
            wall_ms: 0,
            asleep_ms: 0,
        };
        let starting = RunController::new(session.clone(), config.clone(), zero).status();
        // Set before the thread runs, so a later status from it is never overwritten.
        let previous = std::mem::replace(&mut *lock(&self.status), starting.clone());

        let (samples, sample_rx) = mpsc::sync_channel(SAMPLE_QUEUE);
        let control = Arc::new(Control::default());
        let worker = Worker {
            deps: Arc::clone(&self.deps),
            control: Arc::clone(&control),
            samples: sample_rx,
            status: Arc::clone(&self.status),
            epoch: Instant::now(),
            last_state: Cell::new(None),
            last_emit: Cell::new(None),
        };
        let thread = std::thread::Builder::new()
            .name("oma-perf-runner".into())
            .spawn(move || {
                let (deps, status) = (Arc::clone(&worker.deps), Arc::clone(&worker.status));
                let started = session.clone();
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker.run(session, config, starting)
                }));
                if run.is_err() {
                    after_panic(&deps, &status, started);
                }
            })
            .map_err(|e| {
                // Nothing started: the status goes back to what it was.
                *lock(&self.status) = previous;
                StartError::System(e.to_string())
            })?;
        *active = Some(Active {
            thread,
            control,
            samples,
            cores: core_count(&topology),
            sensors: None,
            bench: false,
        });
        Ok(id)
    }

    /// Asks the test or benchmark in progress to stop (a test is saved as
    /// `stopped_user`, a benchmark not at all); the tray's «Stop the test».
    pub fn stop(&self) {
        if let Some(a) = lock(&self.active).as_ref() {
            a.control.stop.store(true, Ordering::Release);
        }
    }

    /// The test in progress or the last one; `idle` before any test.
    pub fn status(&self) -> RunStatus {
        lock(&self.status).clone()
    }

    /// A stress test or a benchmark is in progress.
    pub fn is_running(&self) -> bool {
        lock(&self.active).as_ref().is_some_and(Active::running)
    }

    /// A stress test (not a benchmark) is in progress: only it asks before quitting.
    pub fn stress_running(&self) -> bool {
        lock(&self.active)
            .as_ref()
            .is_some_and(|a| a.running() && !a.bench)
    }

    pub fn bench_running(&self) -> bool {
        lock(&self.active)
            .as_ref()
            .is_some_and(|a| a.running() && a.bench)
    }

    /// The CPU reading of a sampler tick for the test in progress. Never
    /// blocks: a busy runner skips the tick, a full queue drops the sample.
    pub fn on_tick(&self, out: &TickOutput, schema: &Schema) {
        let Ok(mut active) = self.active.try_lock() else {
            return;
        };
        let Some(a) = active.as_mut().filter(|a| a.running()) else {
            return;
        };
        if a.sensors
            .as_ref()
            .is_none_or(|(rev, _)| *rev != schema.revision)
        {
            a.sensors = Some((schema.revision, resolve_cpu_sensors(schema, a.cores)));
        }
        let Some((_, ids)) = &a.sensors else { return };
        let sample = read_sample(ids, &out.snapshot, &out.quality);
        let _ = a
            .samples
            .try_send((sample, (self.deps.service_available)()));
    }

    /// Stops the test or benchmark in progress and waits for its thread
    /// (a benchmark saves nothing):
    /// `Stop`, the helper's answer for at most `timeout`, then the Job kills
    /// it (DA16). No toast: the app is going away.
    ///
    /// The test stays the one in progress until its thread ends: a start
    /// meanwhile is `Busy`, and a second shutdown waits for the same thread.
    pub fn shutdown(&self, timeout: Duration) {
        if let Some(a) = lock(&self.active).as_ref().filter(|a| a.running()) {
            lock(&a.control.deadline).get_or_insert(Instant::now() + timeout);
            a.control.stop.store(true, Ordering::Release);
        }
        // The final WHEA poll and the save come after the deadline.
        let until = Instant::now() + timeout + Duration::from_secs(1);
        while self.is_running() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut active = lock(&self.active);
        if active.as_ref().is_some_and(Active::running) {
            tracing::warn!("the stress test did not end in time");
        } else if let Some(a) = active.take() {
            let _ = a.thread.join();
        }
    }
}

/// The `oma-perf-runner` thread.
struct Worker {
    deps: Arc<RunnerDeps>,
    control: Arc<Control>,
    samples: Receiver<(SensorSample, bool)>,
    status: Arc<Mutex<RunStatus>>,
    epoch: Instant,
    last_state: Cell<Option<RunState>>,
    last_emit: Cell<Option<Instant>>,
}

/// The controller and the helper it acts on.
struct Driver<'a> {
    worker: &'a Worker,
    ctl: RunController,
    host: Box<dyn LoadLink>,
    killed: bool,
    done: bool,
}

impl Worker {
    fn clock(&self) -> Clock {
        clock_at(self.epoch, self.deps.machine.as_ref())
    }

    fn lang(&self) -> Lang {
        language_for(self.deps.settings.snapshot().general.language)
    }

    /// Copies the status for the commands; tells the tray on a state change
    /// and the windows on a change or once a second, while one is open.
    fn publish(&self, status: RunStatus) {
        let changed = self.last_state.get() != Some(status.state);
        if changed {
            self.last_state.set(Some(status.state));
            (self.deps.on_state)(&status);
        }
        let due = self
            .last_emit
            .get()
            .is_none_or(|t| t.elapsed() >= STATUS_EVERY);
        if (changed || due) && (self.deps.window_open)() {
            self.last_emit.set(Some(Instant::now()));
            (self.deps.emit)(&status);
        }
        *lock(&self.status) = status;
    }

    fn toast(&self, session: &Session) {
        if self.control.deadline().is_some() {
            return;
        }
        let Some(detail) = &session.outcome_detail else {
            return;
        };
        let lang = self.lang();
        self.deps.toaster.show(
            t(lang, "performance.toast.title", &[]),
            verdict_text(lang, detail),
            launch_for_performance(&session.id),
        );
    }

    fn run(self, mut session: Session, mut config: RunConfig, starting: RunStatus) {
        #[cfg(windows)]
        let _awake = oma_win::power::KeepAwake::new();
        self.publish(starting);
        let run = match plan_message(session.plan.clone()) {
            Ok(run) => run,
            Err(failure) => return self.failed(session, config, &failure),
        };
        let whea_start = self.deps.machine.latest_whea();
        let (tx, rx) = mpsc::channel();
        let host = match (self.deps.launcher)(tx) {
            Ok(host) => host,
            Err(failure) => return self.failed(session, config, &failure),
        };
        config.whea_after = whea_start.as_ref().ok().copied().flatten();
        config.whea_baseline_missing = whea_start.is_err();
        config.apic_to_core = host
            .topology()
            .logical
            .iter()
            .filter_map(|l| Some((l.apic_id?, l.core)))
            .collect();
        session.load_version = Some(host.hello().version.clone());
        let ctl = RunController::new(session, config, self.clock());
        let mut d = Driver {
            worker: &self,
            ctl,
            host,
            killed: false,
            done: false,
        };
        if let Err(err) = &whea_start {
            tracing::warn!(%err, "WHEA log unreadable");
            let a = d.ctl.on_whea(Err(()), d.worker.clock());
            d.exec(a);
        }
        if d.worker.control.stop.swap(false, Ordering::AcqRel) {
            // Stopped during the handshake: the plan never runs.
            let a = d.ctl.on_user_stop(d.worker.clock());
            d.exec(a);
        } else if let Err(err) = d.host.send(&run) {
            // The pipe is gone: `Closed` and `Exited` follow.
            tracing::warn!(%err, "cannot send the plan to oma-load");
        }
        let status = d.drive(&rx);
        drop(d);
        self.publish(status);
    }

    /// The helper did not start: a `failed_to_start` session with the reason.
    fn failed(self, mut session: Session, config: RunConfig, failure: &StartFailure) {
        tracing::warn!(%failure, "the stress test did not start");
        let reason = t(self.lang(), failure.i18n_key(), &[]);
        let (outcome, key) = decide(&OutcomeFacts {
            failed_to_start: Some(reason),
            ..Default::default()
        });
        session.outcome = Some(outcome);
        session.outcome_detail = Some(bare_detail(key));
        session.ended_at = Some(to_rfc3339(unix_now_ms()));
        if let Err(err) = self.deps.store.save(&session) {
            tracing::warn!(%err, "cannot save the stress session");
        }
        let mut status = RunController::new(session.clone(), config, self.clock()).status();
        status.state = RunState::Finished;
        self.publish(status);
        self.toast(&session);
    }
}

impl Driver<'_> {
    /// Runs the actions in order; a WHEA poll feeds its result to the
    /// controller before the next action.
    fn exec(&mut self, actions: Vec<Action>) {
        let store = &self.worker.deps.store;
        for action in actions {
            match action {
                Action::SendStop => {
                    if !self.killed {
                        if let Err(err) = self.host.send(&LoadMessage::Stop(StopRequest {})) {
                            tracing::warn!(%err, "cannot ask oma-load to stop");
                        }
                    }
                }
                Action::Kill => {
                    self.host.kill();
                    self.killed = true;
                }
                Action::WriteJournal(j) => {
                    if let Err(err) = store.write_journal(&j) {
                        tracing::warn!(%err, "cannot write the stress journal");
                    }
                }
                Action::SaveSession => {
                    if let Err(err) = store.save(self.ctl.session()) {
                        tracing::warn!(%err, "cannot save the stress session");
                    }
                }
                Action::DeleteJournal => {
                    if let Err(err) = store.delete_journal() {
                        tracing::warn!(%err, "cannot delete the stress journal");
                    }
                }
                Action::PollWhea { after_record } => {
                    let result = self
                        .worker
                        .deps
                        .machine
                        .whea_after(after_record)
                        .map_err(|err| tracing::warn!(%err, "WHEA log unreadable"));
                    let next = self.ctl.on_whea(result, self.worker.clock());
                    self.exec(next);
                }
                Action::Toast => self.worker.toast(self.ctl.session()),
                Action::Finished(_) => self.done = true,
            }
        }
    }

    /// The 250 ms loop, until the controller's verdict; returns the final status.
    fn drive(&mut self, rx: &Receiver<HostEvent>) -> RunStatus {
        let mut exited: Option<(Option<i32>, Instant)> = None;
        let mut closed = false;
        let mut exit_seen = false;
        let mut forced = false;
        loop {
            let mut events = Vec::new();
            match rx.recv_timeout(TICK) {
                Ok(e) => events.push(e),
                Err(RecvTimeoutError::Timeout) => {}
                // Every sender gone: only the clock is left.
                Err(RecvTimeoutError::Disconnected) => std::thread::sleep(TICK),
            }
            events.extend(rx.try_iter());
            // A wake first: the messages queued during a sleep must not end the session.
            let a = self.ctl.on_sleep_check(self.worker.clock());
            self.exec(a);
            for event in events {
                // The events of a helper we killed are of no interest.
                if self.killed || self.done {
                    break;
                }
                match event {
                    HostEvent::Message(m) => {
                        let a = self.ctl.on_load(&m, self.worker.clock());
                        self.exec(a);
                    }
                    HostEvent::Closed => closed = true,
                    HostEvent::Exited(code) => {
                        exited.get_or_insert((code, Instant::now()));
                    }
                }
            }
            // `Closed` comes after the last message: an exit waits for it, so
            // a queued `Finished` is never lost.
            if let Some((code, at)) = exited {
                if !exit_seen && !self.killed && (closed || at.elapsed() >= EXIT_GRACE) {
                    exit_seen = true;
                    let a = self.ctl.on_exit(code, self.worker.clock());
                    self.exec(a);
                }
            }
            while let Ok((sample, service)) = self.worker.samples.try_recv() {
                let a = self.ctl.on_sample(&sample, service, self.worker.clock());
                self.exec(a);
            }
            if self.worker.control.stop.swap(false, Ordering::AcqRel) {
                let a = self.ctl.on_user_stop(self.worker.clock());
                self.exec(a);
            }
            let late = self
                .worker
                .control
                .deadline()
                .is_some_and(|d| Instant::now() >= d);
            if late && !forced && !self.done {
                // The app is leaving: the helper goes, the stop counts as clean.
                forced = true;
                self.exec(vec![Action::Kill]);
                let a = self.ctl.on_exit(Some(0), self.worker.clock());
                self.exec(a);
            }
            let a = self.ctl.on_clock(self.worker.clock());
            self.exec(a);
            let status = self.ctl.status();
            if self.done {
                return status;
            }
            self.worker.publish(status);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::mpsc;

    use oma_core::load::{Component, Objective, Outcome, Preset, RunState};
    use oma_core::settings::Language;
    use oma_ipc::load::{
        CacheSizes, ComputeError, ErrorKind, FinishReason, Finished, KernelId, LogicalCpu, Progress,
    };

    use super::*;
    use crate::settings::fake_fs::FakeFs;

    #[test]
    fn start_errors_reach_the_ui_as_stable_codes() {
        let wire = |e: BuildError| StartError::Plan(e).wire();
        assert_eq!(wire(BuildError::NoCores), "build:no_cores");
        assert_eq!(wire(BuildError::NoPhases), "build:no_phases");
        assert_eq!(wire(BuildError::TooLong), "build:too_long");
        assert_eq!(wire(BuildError::UnknownCore(3)), "build:unknown_core");
        assert_eq!(wire(BuildError::RamBudget), "build:ram_budget");
        assert_eq!(StartError::Busy.wire(), "busy");
        assert_eq!(StartError::System("no pipe".into()).wire(), "no pipe");
    }

    pub(crate) const CORES: u32 = 4;

    pub(crate) fn topology() -> Topology {
        Topology {
            logical: (0..CORES)
                .map(|core| LogicalCpu {
                    index: core,
                    group: 0,
                    number: core as u8,
                    core,
                    core_index: core,
                    efficiency_class: 0,
                    llc: 0,
                    parked: false,
                    apic_id: Some(core * 2),
                })
                .collect(),
            caches: CacheSizes {
                l1d_bytes: 32 << 10,
                l2_bytes: 1 << 20,
                l2_shared_by: 1,
                l3_bytes: 32 << 20,
                l3_total_bytes: 32 << 20,
            },
            hypervisor: false,
            vendor: "AuthenticAMD".into(),
            brand: "Test CPU".into(),
        }
    }

    #[derive(Default)]
    pub(crate) struct FakeMachine {
        /// The time asleep it reports; a script moves it on.
        pub(crate) asleep: Arc<AtomicU64>,
        pub(crate) battery: Option<bool>,
        /// What each poll of the other processes' CPU share gives.
        pub(crate) busy: Option<f64>,
        /// The first poll gives `None`, like the real PDH counter.
        pub(crate) busy_primes: bool,
    }

    impl Machine for FakeMachine {
        fn topology(&self) -> io::Result<Topology> {
            Ok(topology())
        }
        fn isa(&self) -> Vec<Isa> {
            vec![Isa::Avx2, Isa::Sse2]
        }
        fn memory(&self) -> io::Result<(u64, u64)> {
            Ok((16 << 30, 8 << 30))
        }
        fn latest_whea(&self) -> io::Result<Option<u64>> {
            Ok(Some(7))
        }
        fn whea_after(&self, _after: Option<u64>) -> io::Result<Vec<WheaEvent>> {
            Ok(vec![])
        }
        fn asleep_ms(&self) -> u64 {
            self.asleep.load(Ordering::SeqCst)
        }
        fn on_battery(&self) -> Option<bool> {
            self.battery
        }
        fn busy_probe(&self, _logical: u32) -> Box<dyn FnMut() -> Option<f64>> {
            let (busy, mut first) = (self.busy, self.busy_primes);
            Box::new(move || {
                if std::mem::take(&mut first) {
                    None
                } else {
                    busy
                }
            })
        }
    }

    /// The helper's answers: what it sends after `Run`, and after `Stop`.
    #[derive(Clone, Default)]
    pub(crate) struct Script {
        pub(crate) on_run: Vec<LoadMessage>,
        pub(crate) on_stop: Vec<LoadMessage>,
        /// After the messages: `Closed`, then `Exited(Some(0))`.
        pub(crate) exit_after_run: bool,
        pub(crate) exit_after_stop: bool,
        /// Plays the `Run` answer from a thread, one message per interval.
        pub(crate) spread: Option<Duration>,
        /// What the helper received, by name.
        pub(crate) received: Arc<Mutex<Vec<&'static str>>>,
        /// Set to this much time asleep when `Run` arrives, before the answer.
        pub(crate) sleep_on_run: Option<(Arc<AtomicU64>, u64)>,
    }

    struct FakeLink {
        script: Script,
        tx: Sender<HostEvent>,
        hello: LoadHello,
        topology: Topology,
    }

    fn play(tx: &Sender<HostEvent>, msgs: &[LoadMessage], exit: bool, every: Option<Duration>) {
        for m in msgs {
            if let Some(every) = every {
                std::thread::sleep(every);
            }
            let _ = tx.send(HostEvent::Message(m.clone()));
        }
        if exit {
            let _ = tx.send(HostEvent::Closed);
            let _ = tx.send(HostEvent::Exited(Some(0)));
        }
    }

    impl LoadLink for FakeLink {
        fn send(&self, msg: &LoadMessage) -> io::Result<()> {
            let s = &self.script;
            match msg {
                LoadMessage::Run(_) => {
                    s.received.lock().unwrap().push("run");
                    if let Some((asleep, ms)) = &s.sleep_on_run {
                        asleep.store(*ms, Ordering::SeqCst);
                    }
                    let (tx, msgs, exit, every) = (
                        self.tx.clone(),
                        s.on_run.clone(),
                        s.exit_after_run,
                        s.spread,
                    );
                    if every.is_some() {
                        std::thread::spawn(move || play(&tx, &msgs, exit, every));
                    } else {
                        play(&tx, &msgs, exit, None);
                    }
                }
                LoadMessage::Stop(_) => {
                    s.received.lock().unwrap().push("stop");
                    play(&self.tx, &s.on_stop, s.exit_after_stop, None);
                }
                _ => {}
            }
            Ok(())
        }
        fn hello(&self) -> &LoadHello {
            &self.hello
        }
        fn topology(&self) -> &Topology {
            &self.topology
        }
        fn kill(&mut self) {}
    }

    #[derive(Clone, Default)]
    pub(crate) struct Toasts(pub(crate) Arc<Mutex<Vec<(String, String, String)>>>);

    impl ToastSink for Toasts {
        fn show(&self, title: String, body: String, launch: String) {
            self.0.lock().unwrap().push((title, body, launch));
        }
    }

    pub(crate) struct Rig {
        pub(crate) runner: PerformanceRunner,
        pub(crate) toasts: Toasts,
        pub(crate) emitted: Arc<AtomicUsize>,
        pub(crate) window: Arc<AtomicBool>,
        /// Whether the main window is visible (the bench toast).
        pub(crate) visible: Arc<AtomicBool>,
        /// Benchmark events sent to the window.
        pub(crate) bench_emitted: Arc<AtomicUsize>,
        /// The tray's benchmark mark, as the last state change left it.
        pub(crate) bench_mark: Arc<AtomicBool>,
        /// Last: removed after the runner is gone.
        _dir: TempDir,
    }

    /// A test's store folder, removed when the test ends.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "oma-perf-runner-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn rig_with(name: &str, launcher: Launcher) -> Rig {
        rig_on(name, launcher, FakeMachine::default())
    }

    pub(crate) fn rig_on(name: &str, launcher: Launcher, machine: FakeMachine) -> Rig {
        let settings = Arc::new(SettingsStore::open(None, FakeFs::new()));
        settings.update_with(|s| s.general.language = Language::En);
        let toasts = Toasts::default();
        let emitted = Arc::new(AtomicUsize::new(0));
        let window = Arc::new(AtomicBool::new(true));
        let visible = Arc::new(AtomicBool::new(true));
        let bench_emitted = Arc::new(AtomicUsize::new(0));
        let bench_mark = Arc::new(AtomicBool::new(false));
        let (e, w, v) = (emitted.clone(), window.clone(), visible.clone());
        let (be, bm) = (bench_emitted.clone(), bench_mark.clone());
        let dir = TempDir::new(name);
        let runner = PerformanceRunner::new(RunnerDeps {
            store: Arc::new(PerformanceStore::new(dir.0.clone())),
            settings,
            machine: Box::new(machine),
            launcher,
            toaster: Box::new(toasts.clone()),
            schema: Box::new(|| Schema {
                revision: 1,
                devices: vec![],
                sensors: vec![],
            }),
            service_available: Box::new(|| false),
            window_open: Box::new(move || w.load(Ordering::SeqCst)),
            window_visible: Box::new(move || v.load(Ordering::SeqCst)),
            emit: Box::new(move |_| {
                e.fetch_add(1, Ordering::SeqCst);
            }),
            on_state: Box::new(|_| {}),
            emit_bench: Box::new(move |_| {
                be.fetch_add(1, Ordering::SeqCst);
            }),
            on_bench: Box::new(move |status| {
                bm.store(
                    crate::tray::TestMark::from_bench(status).is_some(),
                    Ordering::SeqCst,
                );
            }),
            app_version: "0.0.0-test".into(),
        });
        Rig {
            runner,
            toasts,
            emitted,
            window,
            visible,
            bench_emitted,
            bench_mark,
            _dir: dir,
        }
    }

    pub(crate) fn scripted(script: Script) -> Launcher {
        Box::new(move |tx| {
            Ok(Box::new(FakeLink {
                script: script.clone(),
                tx,
                hello: LoadHello {
                    protocol_version: oma_ipc::load::LOAD_PROTOCOL_VERSION,
                    version: "9.9.9".into(),
                    isa: vec![Isa::Avx2, Isa::Sse2],
                },
                topology: topology(),
            }) as Box<dyn LoadLink>)
        })
    }

    pub(crate) fn request() -> StartRequest {
        StartRequest {
            component: Component::Cpu,
            objective: Objective::Normal,
            preset: Preset::Quick,
            custom: None,
            retry_core: None,
        }
    }

    pub(crate) fn progress(phase: u32) -> LoadMessage {
        LoadMessage::Progress(Progress {
            phase,
            phase_elapsed_ms: 0,
            elapsed_ms: 0,
            checks: 3,
            errors: 0,
            current_core: None,
            cores: vec![],
            memory_bytes: 0,
            rate: None,
        })
    }

    pub(crate) fn finished(reason: FinishReason, errors: u64) -> LoadMessage {
        LoadMessage::Finished(Finished {
            reason,
            checks: 10,
            errors,
        })
    }

    /// Waits (at most 3 s) for the runner's thread to end.
    pub(crate) fn wait_idle(runner: &PerformanceRunner) {
        let until = Instant::now() + Duration::from_secs(3);
        while runner.is_running() {
            assert!(Instant::now() < until, "the runner did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_running(runner: &PerformanceRunner) {
        let until = Instant::now() + Duration::from_secs(3);
        while runner.status().state != RunState::Running {
            assert!(Instant::now() < until, "never running");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn only_session(runner: &PerformanceRunner) -> oma_core::load::Session {
        let list = runner.store().list();
        assert_eq!(list.len(), 1, "{list:?}");
        runner.store().load(&list[0].id).unwrap().unwrap()
    }

    /// A launcher that waits until `release` is sent, then plays `script`.
    pub(crate) fn gated(script: Script) -> (Launcher, mpsc::Sender<()>) {
        let (release, wait) = mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let inner = scripted(script);
        let launcher: Launcher = Box::new(move |tx| {
            let _ = wait.lock().unwrap().recv_timeout(Duration::from_secs(3));
            inner(tx)
        });
        (launcher, release)
    }

    /// A launcher that waits until `release` is sent, then fails.
    pub(crate) fn blocked() -> (Launcher, mpsc::Sender<()>) {
        let (release, wait) = mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let launcher: Launcher = Box::new(move |_tx| {
            let _ = wait.lock().unwrap().recv_timeout(Duration::from_secs(3));
            Err(StartFailure::Timeout)
        });
        (launcher, release)
    }

    #[test]
    fn start_while_running_is_busy() {
        let (launcher, release) = blocked();
        let rig = rig_with("busy", launcher);
        assert_eq!(rig.runner.status().state, RunState::Idle);
        let id = rig.runner.start(request()).unwrap();
        assert!(rig.runner.is_running());
        assert_eq!(rig.runner.status().session_id, id);
        assert!(matches!(rig.runner.start(request()), Err(StartError::Busy)));
        release.send(()).unwrap();
        wait_idle(&rig.runner);
    }

    #[test]
    fn scripted_run_saves_a_passed_session_and_toasts() {
        let rig = rig_with(
            "passed",
            scripted(Script {
                on_run: vec![progress(0), finished(FinishReason::Completed, 0)],
                exit_after_run: true,
                ..Default::default()
            }),
        );
        let id = rig.runner.start(request()).unwrap();
        wait_idle(&rig.runner);
        let s = only_session(&rig.runner);
        assert_eq!(s.id, id);
        assert_eq!(s.outcome, Some(Outcome::Passed));
        assert_eq!(s.load_version.as_deref(), Some("9.9.9"));
        assert_eq!(s.whea.last_record, Some(7));
        assert_eq!(s.device, "Test CPU");
        assert!(rig.runner.store().read_journal().is_none());
        assert_eq!(
            *rig.toasts.0.lock().unwrap(),
            [(
                "Stress test finished".to_string(),
                "Passed".to_string(),
                crate::notifier::launch_for_performance(&id)
            )]
        );
        let st = rig.runner.status();
        assert_eq!(st.state, RunState::Finished);
        assert_eq!(st.outcome, Some(Outcome::Passed));
    }

    #[test]
    fn scripted_error_on_core_2_saves_unstable_core_2() {
        let error = LoadMessage::Error(ComputeError {
            phase: 0,
            kernel: KernelId::K2,
            isa: Isa::Avx2,
            kind: ErrorKind::Mismatch,
            logical: Some(2),
            core: Some(2),
            iteration: 3,
            expected: 1,
            actual: 2,
            seed: 1,
        });
        let rig = rig_with(
            "core2",
            scripted(Script {
                on_run: vec![progress(0), error, finished(FinishReason::Completed, 1)],
                exit_after_run: true,
                ..Default::default()
            }),
        );
        rig.runner.start(request()).unwrap();
        wait_idle(&rig.runner);
        let s = only_session(&rig.runner);
        assert_eq!(s.outcome, Some(Outcome::Errors));
        let d = s.outcome_detail.unwrap();
        assert_eq!(d.verdict, "errors_core");
        assert_eq!(d.core, Some(2));
        assert_eq!(rig.toasts.0.lock().unwrap()[0].1, "Unstable · core 2");
    }

    #[test]
    fn start_failure_saves_failed_to_start_with_the_reason() {
        let rig = rig_with("failed", Box::new(|_| Err(StartFailure::Missing)));
        let id = rig.runner.start(request()).unwrap();
        wait_idle(&rig.runner);
        let s = only_session(&rig.runner);
        assert_eq!(s.id, id);
        assert_eq!(s.outcome, Some(Outcome::FailedToStart));
        let d = s.outcome_detail.unwrap();
        assert_eq!(d.verdict, "failed_to_start");
        assert_eq!(
            d.params["reason"],
            "test component (oma-load.exe) not found"
        );
        assert_eq!(
            rig.toasts.0.lock().unwrap()[0].1,
            "Not started: test component (oma-load.exe) not found"
        );
        let st = rig.runner.status();
        assert_eq!(st.state, RunState::Finished);
        assert_eq!(st.outcome, Some(Outcome::FailedToStart));
    }

    #[test]
    fn status_events_only_with_a_window_open() {
        let script = Script {
            on_run: vec![progress(0), finished(FinishReason::Completed, 0)],
            exit_after_run: true,
            ..Default::default()
        };
        let rig = rig_with("events", scripted(script));
        rig.window.store(false, Ordering::SeqCst);
        rig.runner.start(request()).unwrap();
        wait_idle(&rig.runner);
        assert_eq!(rig.emitted.load(Ordering::SeqCst), 0);
        rig.window.store(true, Ordering::SeqCst);
        rig.runner.start(request()).unwrap();
        wait_idle(&rig.runner);
        // Starting and finished (the script's messages arrive in one batch).
        assert!(rig.emitted.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    fn on_tick_never_blocks_when_the_channel_is_full() {
        let (launcher, release) = blocked();
        let rig = rig_with("tick", launcher);
        rig.runner.start(request()).unwrap();
        let out = TickOutput {
            snapshot: oma_core::model::Snapshot {
                revision: 1,
                seq: 0,
                timestamp_ms: 0,
                values: vec![],
            },
            schema: None,
            quality: vec![],
            health: None,
            entries: vec![],
            monotonic_ms: 0,
        };
        let schema = Schema {
            revision: 1,
            devices: vec![],
            sensors: vec![],
        };
        // The runner's thread is stuck in the launcher: nobody drains the samples.
        let t0 = Instant::now();
        for _ in 0..1000 {
            rig.runner.on_tick(&out, &schema);
        }
        assert!(t0.elapsed() < Duration::from_millis(500));
        release.send(()).unwrap();
        wait_idle(&rig.runner);
    }

    #[test]
    fn shutdown_stops_and_saves_stopped_user() {
        // The helper ignores `Stop`: the runner gives up after the timeout.
        let rig = rig_with(
            "shutdown",
            scripted(Script {
                on_run: vec![progress(0)],
                ..Default::default()
            }),
        );
        rig.runner.start(request()).unwrap();
        wait_running(&rig.runner);
        let t0 = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| rig.runner.shutdown(Duration::from_millis(300)));
            std::thread::sleep(Duration::from_millis(100));
            // Still the test in progress until its thread ends.
            assert!(rig.runner.is_running());
            assert!(matches!(rig.runner.start(request()), Err(StartError::Busy)));
        });
        assert!(t0.elapsed() < Duration::from_secs(2));
        assert!(!rig.runner.is_running());
        let s = only_session(&rig.runner);
        assert_eq!(s.outcome, Some(Outcome::StoppedUser));
        assert!(rig.runner.store().read_journal().is_none());
        // The app is going away: no toast.
        assert!(rig.toasts.0.lock().unwrap().is_empty());
    }

    #[test]
    fn user_stop_answered_by_the_helper_is_stopped_user() {
        let rig = rig_with(
            "stop",
            scripted(Script {
                on_run: vec![progress(0)],
                on_stop: vec![finished(FinishReason::Stopped, 0)],
                exit_after_stop: true,
                ..Default::default()
            }),
        );
        rig.runner.start(request()).unwrap();
        wait_running(&rig.runner);
        rig.runner.stop();
        wait_idle(&rig.runner);
        assert_eq!(
            only_session(&rig.runner).outcome,
            Some(Outcome::StoppedUser)
        );
        assert_eq!(rig.toasts.0.lock().unwrap()[0].1, "Stopped by you");
    }

    #[test]
    fn running_status_events_are_at_most_one_a_second() {
        // About 2.2 s of progress, one message every 50 ms.
        let mut on_run: Vec<LoadMessage> = (0..44).map(|_| progress(0)).collect();
        on_run.push(finished(FinishReason::Completed, 0));
        let rig = rig_with(
            "throttle",
            scripted(Script {
                on_run,
                exit_after_run: true,
                spread: Some(Duration::from_millis(50)),
                ..Default::default()
            }),
        );
        rig.runner.start(request()).unwrap();
        wait_idle(&rig.runner);
        let n = rig.emitted.load(Ordering::SeqCst);
        // Starting, running, finished, and one or two once-a-second updates.
        assert!((3..=6).contains(&n), "{n} events");
    }

    #[test]
    fn a_finished_queued_during_a_sleep_still_ends_suspended() {
        let machine = FakeMachine::default();
        let rig = rig_on(
            "sleep",
            scripted(Script {
                on_run: vec![progress(0), finished(FinishReason::Completed, 0)],
                exit_after_run: true,
                sleep_on_run: Some((Arc::clone(&machine.asleep), 50_000)),
                ..Default::default()
            }),
            machine,
        );
        rig.runner.start(request()).unwrap();
        wait_idle(&rig.runner);
        assert_eq!(only_session(&rig.runner).outcome, Some(Outcome::Suspended));
    }

    #[test]
    fn unreadable_whea_baseline_counts_nothing_from_the_history() {
        struct HistoryMachine(FakeMachine);
        impl Machine for HistoryMachine {
            fn topology(&self) -> io::Result<Topology> {
                self.0.topology()
            }
            fn isa(&self) -> Vec<Isa> {
                self.0.isa()
            }
            fn memory(&self) -> io::Result<(u64, u64)> {
                self.0.memory()
            }
            fn latest_whea(&self) -> io::Result<Option<u64>> {
                Err(io::Error::other("no log"))
            }
            /// The whole history, every time.
            fn whea_after(&self, _after: Option<u64>) -> io::Result<Vec<WheaEvent>> {
                Ok(vec![WheaEvent {
                    record_id: 3,
                    event_id: 19,
                    apic_id: None,
                    time_utc: String::new(),
                }])
            }
            fn asleep_ms(&self) -> u64 {
                0
            }
            fn on_battery(&self) -> Option<bool> {
                None
            }
            fn busy_probe(&self, logical: u32) -> Box<dyn FnMut() -> Option<f64>> {
                self.0.busy_probe(logical)
            }
        }
        let settings = Arc::new(SettingsStore::open(None, FakeFs::new()));
        let dir = TempDir::new("baseline");
        let runner = PerformanceRunner::new(RunnerDeps {
            store: Arc::new(PerformanceStore::new(dir.0.clone())),
            settings,
            machine: Box::new(HistoryMachine(FakeMachine::default())),
            launcher: scripted(Script {
                on_run: vec![progress(0), finished(FinishReason::Completed, 0)],
                exit_after_run: true,
                ..Default::default()
            }),
            toaster: Box::new(Toasts::default()),
            schema: Box::new(|| Schema {
                revision: 1,
                devices: vec![],
                sensors: vec![],
            }),
            service_available: Box::new(|| false),
            window_open: Box::new(|| false),
            window_visible: Box::new(|| false),
            emit: Box::new(|_| {}),
            on_state: Box::new(|_| {}),
            emit_bench: Box::new(|_| {}),
            on_bench: Box::new(|_| {}),
            app_version: "0.0.0-test".into(),
        });
        runner.start(request()).unwrap();
        wait_idle(&runner);
        let s = only_session(&runner);
        assert_eq!(
            s.outcome,
            Some(Outcome::Passed),
            "the old WHEA 19 is not counted"
        );
        assert_eq!(s.whea.last_record, Some(3));
    }

    #[test]
    fn an_invalid_plan_never_reaches_the_helper() {
        let plan = Plan {
            seed: 1,
            ram_bytes: 0,
            phases: vec![],
        };
        let failure = plan_message(plan).unwrap_err();
        assert_eq!(failure.i18n_key(), "performance.start.invalid_plan");
    }

    #[test]
    fn a_runner_panic_saves_a_crashed_session_and_ends_the_status() {
        struct PanicLink(LoadHello, Topology);
        impl LoadLink for PanicLink {
            fn send(&self, _msg: &LoadMessage) -> io::Result<()> {
                panic!("a bug in the link");
            }
            fn hello(&self) -> &LoadHello {
                &self.0
            }
            fn topology(&self) -> &Topology {
                &self.1
            }
            fn kill(&mut self) {}
        }
        let launcher: Launcher = Box::new(|_| {
            Ok(Box::new(PanicLink(
                LoadHello {
                    protocol_version: oma_ipc::load::LOAD_PROTOCOL_VERSION,
                    version: "9.9.9".into(),
                    isa: vec![],
                },
                topology(),
            )) as Box<dyn LoadLink>)
        });
        let rig = rig_with("panic", launcher);
        rig.runner.start(request()).unwrap();
        wait_idle(&rig.runner);
        let s = only_session(&rig.runner);
        assert_eq!(s.outcome, Some(Outcome::Crashed));
        assert_eq!(s.outcome_detail.unwrap().verdict, "crashed");
        assert!(rig.runner.store().read_journal().is_none());
        let st = rig.runner.status();
        assert_eq!(
            (st.state, st.outcome),
            (RunState::Finished, Some(Outcome::Crashed))
        );
        // The next test can start.
        assert!(!rig.runner.is_running());
    }

    #[test]
    fn verdict_text_translates_a_reason_key() {
        let detail = OutcomeDetail {
            verdict: "failed_to_start".into(),
            params: [(
                "reason".to_string(),
                "performance.start.nothing_ran".to_string(),
            )]
            .into(),
            phase: None,
            kernel: None,
            core: None,
            temp_c: None,
            clock_mhz: None,
            at_ms: None,
        };
        assert_eq!(verdict_text(Lang::En, &detail), "Not started: no phase ran");
    }

    #[test]
    fn stop_during_the_handshake_never_sends_the_plan() {
        let script = Script {
            on_stop: vec![finished(FinishReason::Stopped, 0)],
            exit_after_stop: true,
            ..Default::default()
        };
        let received = script.received.clone();
        let (launcher, release) = gated(script);
        let rig = rig_with("handshake", launcher);
        rig.runner.start(request()).unwrap();
        rig.runner.stop();
        release.send(()).unwrap();
        wait_idle(&rig.runner);
        assert_eq!(*received.lock().unwrap(), ["stop"]);
        assert_eq!(
            only_session(&rig.runner).outcome,
            Some(Outcome::StoppedUser)
        );
    }
}
