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
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, Once, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use oma_core::engine::TickOutput;
use oma_core::load::{
    build_plan, core_order, cpu_stop_threshold, decide, disk_reserve, disk_stop_threshold,
    gpu_stop_threshold, ram_budget, read_disk_sample, read_gpu_sample, read_sample,
    resolve_cpu_sensors, resolve_disk_sensors, resolve_gpu_sensors, stress_file_bytes, Action,
    BuildError, BuildInput, Clock, Component, CpuSensorIds, DiskPlanInput, DiskSensorIds,
    DiskSession, GpuSensorIds, Objective, OutcomeDetail, OutcomeFacts, Preset, RunConfig,
    RunController, RunState, RunStatus, SensorSample, Session, StartRequest, VerdictKey, WheaEvent,
    FORMAT,
};
use oma_core::model::Schema;
use oma_core::scores::{BenchStatus, DISK_BENCH_FILE};
use oma_ipc::load::{
    is_plain_disk_dir, GpuTarget, Isa, LoadHello, LoadMessage, Plan, RunRequest, StopRequest,
    Topology,
};
use oma_win::gpu::{GpuProcess, GpuProcessTable, StressAdapter};
use oma_win::storage::{
    DiskClass, DiskKind, DiskPower, DiskStateTable, DiskTraits, DriveEntry, DriveIdTable,
};
use oma_win::volumes::{is_remote_path, DriveKind, FolderProbe, VolumeInfo};

use super::host::{HostEvent, StartFailure};
use super::orphans;
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
/// How often a GPU test reads the PCIe replay counter (DG14).
const PCIE_EVERY_MS: u64 = 5_000;

/// Whether the PCIe replay counter is due, last read at `last` (mono ms).
fn pcie_due(last: Option<u64>, now_ms: u64) -> bool {
    last.is_none_or(|t| now_ms.saturating_sub(t) >= PCIE_EVERY_MS)
}

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
    /// The hardware GPUs a test can target (DG13); read only when asked, never on a timer.
    fn gpus(&self) -> Vec<StressAdapter>;
    /// The GPU's PCIe replay counter; `None` when it cannot be read (DG14).
    fn pcie_replay(&self, device_id: &str) -> Option<u32>;
    /// The largest share (0-1) of one engine of GPU `device_id` used by another
    /// program (DH10); `None` when the process table is empty or stale.
    fn gpu_busy_share(&self, device_id: &str) -> Option<f64>;
    /// The local volumes, from metadata only: a sleeping HDD stays asleep (DC6). Read
    /// only when asked, never on a timer.
    fn volumes(&self) -> Vec<VolumeInfo>;
    /// Writes and deletes one tiny file: only at a pick, a confirm or a start (DC6).
    fn probe_folder(&self, path: &Path) -> FolderProbe;
    fn disk_traits(&self, index: u32) -> DiskTraits;
    /// The identified disks: device id and model by disk index.
    fn drives(&self) -> Vec<DriveEntry>;
    /// Disk `device_id`'s power state as the storage provider last saw it.
    fn disk_power(&self, device_id: &str) -> Option<DiskPower>;
    /// The start time (FILETIME) of process `pid`; `None` when it is gone.
    fn process_started_at(&self, pid: u32) -> Option<u64>;
    /// `%LOCALAPPDATA%\Temp`, the test folder on the system volume (DC6).
    fn local_temp_dir(&self) -> PathBuf;
    /// Whether the folder or one above it is a junction, mount point or symbolic link
    /// (R13), read from the parents' listings without following any.
    fn has_link(&self, path: &Path) -> bool;
}

/// The other programs' share of a GPU from its process table rows (DH10): the
/// largest `load_percent / 100`, leaving out PIDs 0 and 4, `dwm.exe` (the
/// compositor, working for our window too) and `ours` (the app's process tree).
pub(super) fn other_gpu_share(rows: &[GpuProcess], ours: &HashSet<u32>) -> Option<f64> {
    if rows.is_empty() {
        return None;
    }
    let share = rows
        .iter()
        .filter(|p| {
            !matches!(p.pid, 0 | 4)
                && !ours.contains(&p.pid)
                && !p.name.eq_ignore_ascii_case("dwm.exe")
        })
        .filter_map(|p| p.load_percent)
        .fold(0.0, f64::max);
    Some(share / 100.0)
}

/// This PC; the GPU process table is the GPU provider's (DH10), the drive and disk
/// state tables the storage provider's (DC6).
pub struct WinMachine {
    processes: GpuProcessTable,
    drives: DriveIdTable,
    disk_states: DiskStateTable,
}

impl WinMachine {
    pub fn new(
        processes: GpuProcessTable,
        drives: DriveIdTable,
        disk_states: DiskStateTable,
    ) -> Self {
        Self {
            processes,
            drives,
            disk_states,
        }
    }
}

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
            // One Toolhelp snapshot per poll, as for the GPU: our WebView2 is not "another program".
            Ok(mut other) => Box::new(move || other.sample(logical, &our_tree())),
            Err(err) => {
                tracing::warn!(%err, "other processes' CPU unreadable");
                Box::new(|| None)
            }
        }
    }

    fn gpus(&self) -> Vec<StressAdapter> {
        oma_win::gpu::stress_adapters()
    }

    fn pcie_replay(&self, device_id: &str) -> Option<u32> {
        oma_win::gpu::pcie_replay_count(device_id)
    }

    /// Read only during the GPU benchmark, every 5 s: one Toolhelp snapshot each time.
    fn gpu_busy_share(&self, device_id: &str) -> Option<f64> {
        other_gpu_share(&self.processes.processes(device_id), &our_tree())
    }

    fn volumes(&self) -> Vec<VolumeInfo> {
        oma_win::volumes::volumes()
    }

    fn probe_folder(&self, path: &Path) -> FolderProbe {
        oma_win::volumes::probe_folder(path)
    }

    fn disk_traits(&self, index: u32) -> DiskTraits {
        oma_win::storage::disk_traits(index)
    }

    fn drives(&self) -> Vec<DriveEntry> {
        self.drives.get().drives
    }

    fn disk_power(&self, device_id: &str) -> Option<DiskPower> {
        let (_, states) = self.disk_states.get();
        states
            .into_iter()
            .find_map(|(id, power)| (id == device_id).then_some(power))
    }

    fn process_started_at(&self, pid: u32) -> Option<u64> {
        oma_win::process_tree::process_started_at(pid)
    }

    fn local_temp_dir(&self) -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map_or_else(std::env::temp_dir, |d| PathBuf::from(d).join("Temp"))
    }

    fn has_link(&self, path: &Path) -> bool {
        oma_win::volumes::has_link(path)
    }
}

/// The app and its descendants (WebView2, `oma-load`), from one Toolhelp snapshot.
fn our_tree() -> HashSet<u32> {
    let me = std::process::id();
    oma_win::process_tree::descendants(me).unwrap_or_else(|err| {
        tracing::warn!(%err, "process tree unreadable: only the app itself is left out");
        HashSet::from([me])
    })
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
    /// The disk target cannot be used (DC6): `remote`, `not_found`, `not_writable`,
    /// `no_space`, `standby` (a spun-down HDD, without the user's consent) or `link`
    /// (the folder leads to another place through a junction or a link, R13).
    Disk(&'static str),
    /// The topology, the session id or the thread could not be had.
    System(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => write!(f, "a test is already running"),
            Self::Plan(e) => write!(f, "{e}"),
            Self::Disk(code) => write!(f, "disk target: {code}"),
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
            Self::Disk(code) => return format!("disk:{code}"),
            Self::Plan(BuildError::NoCores) => "no_cores",
            Self::Plan(BuildError::NoPhases) => "no_phases",
            Self::Plan(BuildError::TooLong) => "too_long",
            Self::Plan(BuildError::UnknownCore(_)) => "unknown_core",
            Self::Plan(BuildError::RamBudget) => "ram_budget",
            Self::Plan(BuildError::NoGpu) => "no_gpu",
            Self::Plan(BuildError::NoDisk) => "no_disk",
            Self::Plan(BuildError::NoSpace) => "no_space",
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
    pub gpus: Vec<GpuChoice>,
    /// The local volumes a disk test can target (DC6); network drives are left out.
    pub volumes: Vec<VolumeChoice>,
}

/// A volume the disk pages offer, with the folder a test would use (DC6): on the
/// system volume `%LOCALAPPDATA%\Temp`, elsewhere the root.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeChoice {
    /// `X:\`.
    pub root: String,
    pub label: String,
    pub fs: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub folder: String,
    /// The physical disk, from the drive table; `None` for a multi-disk volume.
    pub device_id: Option<String>,
    pub model: Option<String>,
    pub kind: DiskKind,
    /// A removable drive or a USB disk: a warning (DC6).
    pub removable: bool,
    pub system: bool,
    /// A virtual disk (bus 14 or 15): a warning.
    pub virtual_disk: bool,
    /// A spun-down HDD: the test asks before waking it.
    pub standby: bool,
    /// The folder is synced to the cloud: a warning. From the path alone in the list,
    /// with the folders' attributes too after a probe.
    pub sync: bool,
    /// The disk's class (the startup sweep skips rotational disks, DC11).
    #[serde(skip)]
    pub(super) solid: bool,
}

/// `X:\` of a plain `X:\...` folder.
fn root_of(folder: &str) -> Option<String> {
    let b = folder.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        .then(|| format!("{}:\\", (b[0] as char).to_ascii_uppercase()))
}

/// The DC13 name of a disk kind (`nvme`, `sata_ssd`…).
pub(super) fn kind_name(kind: DiskKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The drive's warning temperature (WCTEMP), the `tempWarningC` property of its device.
fn wctemp(schema: &Schema, device_id: &str) -> Option<f64> {
    schema
        .devices
        .iter()
        .find(|d| d.id == device_id)?
        .properties
        .get("tempWarningC")?
        .parse()
        .ok()
}

/// What a disk target must leave free for the test file (DC6).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DiskNeed {
    /// The benchmark's 1 GiB file.
    Bench,
    /// At least 1 GiB for the stress file.
    Stress,
}

/// A GPU the wizard offers, chosen by its stable `device_id` (DG13).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuChoice {
    pub device_id: String,
    pub name: String,
    pub integrated: bool,
    pub dedicated_bytes: u64,
}

/// The sensor ids a test reads, resolved for one schema revision.
pub(super) enum SensorIds {
    Cpu(CpuSensorIds),
    Gpu(GpuSensorIds),
    Disk(DiskSensorIds),
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
    pub(super) sensors: Option<(u64, SensorIds)>,
    /// The GPU under test (its `device_id`), `None` for CPU and RAM tests.
    pub(super) gpu: Option<String>,
    /// The disk under test (its `device_id`, empty for a multi-disk volume).
    pub(super) disk: Option<String>,
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
        load_percent: None,
        stability: None,
        gpu_device_id: None,
        disk: None,
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
            } else if k == "stability" && lang == Lang::It {
                // A percent with a decimal point ("95.3"), as the UI shows it in Italian.
                v.replace('.', ",")
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
        // Read before `recover` removes the journal.
        let journal_folder = store
            .read_journal()
            .and_then(Result::ok)
            .and_then(|j| j.disk_folder);
        self.sweep_at_startup(journal_folder);
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

    /// The orphaned test files at startup (DC11): the folder of the journal a crash left,
    /// and the last disk folder only on a solid-state disk, so an HDD is never woken.
    fn sweep_at_startup(&self, journal_folder: Option<String>) {
        let last = self
            .deps
            .settings
            .snapshot()
            .performance
            .disk_folder
            .clone();
        if journal_folder.is_none() && last.is_none() {
            return;
        }
        let m = &self.deps.machine;
        let sweep = |folder: &str| {
            orphans::sweep(Path::new(folder), |pid| m.process_started_at(pid));
        };
        // A plain local folder that leads nowhere else (R13); the journal's is swept even
        // on a sleeping HDD (a crash left files there), so its links are read there too.
        if let Some(folder) = &journal_folder {
            let ok = self
                .disk_volume(folder)
                .and_then(|c| self.check_link(&c))
                .is_ok();
            if ok {
                sweep(folder);
            }
        }
        if let Some(folder) = last.filter(|f| Some(f) != journal_folder.as_ref()) {
            // Solid state only: a rotational disk is never touched, not even for its links.
            if self.disk_volume(&folder).is_ok_and(|c| c.solid) {
                sweep(&folder);
            }
        }
    }

    /// The choice for volume `v` with `folder`, from metadata only (no disk I/O).
    fn volume_choice(&self, v: &VolumeInfo, drives: &[DriveEntry], folder: String) -> VolumeChoice {
        let m = &self.deps.machine;
        let drive = v
            .disk_index
            .and_then(|i| drives.iter().find(|d| d.index == i));
        let traits = v.disk_index.map(|i| m.disk_traits(i));
        let kind = traits.map_or(DiskKind::Other, |t| t.kind);
        let device_id = drive.map(|d| d.device_id.clone());
        let rotational = traits.is_none_or(|t| t.class == DiskClass::RotationalOrUnknown);
        let standby = rotational
            && device_id
                .as_deref()
                .and_then(|id| m.disk_power(id))
                .is_some_and(|p| p == DiskPower::Standby);
        VolumeChoice {
            root: v.root.clone(),
            label: v.label.clone(),
            fs: v.fs.clone(),
            total_bytes: v.total_bytes,
            free_bytes: v.free_bytes,
            sync: oma_win::volumes::is_sync_path(&folder),
            folder,
            device_id,
            model: drive.and_then(|d| d.model.clone()),
            kind,
            removable: v.drive == DriveKind::Removable || kind == DiskKind::Usb,
            system: v.system,
            virtual_disk: kind == DiskKind::Virtual,
            standby,
            solid: !rotational,
        }
    }

    /// The test folder of a volume (DC6): `%LOCALAPPDATA%\Temp` on the system one, else the root.
    fn default_folder(&self, v: &VolumeInfo) -> String {
        if v.system {
            self.deps
                .machine
                .local_temp_dir()
                .to_string_lossy()
                .into_owned()
        } else {
            v.root.clone()
        }
    }

    /// The volume of a folder from the UI, from metadata only: a network folder is
    /// `remote`, anything but a plain local `X:\...` folder `not_found`.
    fn disk_volume(&self, folder: &str) -> Result<VolumeChoice, StartError> {
        if is_remote_path(folder) {
            return Err(StartError::Disk("remote"));
        }
        let root = root_of(folder)
            .filter(|_| is_plain_disk_dir(folder))
            .ok_or(StartError::Disk("not_found"))?;
        let m = &self.deps.machine;
        let v = m
            .volumes()
            .into_iter()
            .find(|v| v.root.eq_ignore_ascii_case(&root))
            .ok_or(StartError::Disk("not_found"))?;
        if v.drive == DriveKind::Remote {
            return Err(StartError::Disk("remote"));
        }
        let c = self.volume_choice(&v, &m.drives(), folder.to_owned());
        // The links are read from the folders' listings: not on a sleeping HDD before the
        // user agrees to wake it (`disk_target` reads them then).
        if !c.standby {
            self.check_link(&c)?;
        }
        Ok(c)
    }

    /// `link` when the folder leads to another place (R13): the volume, the disk, its
    /// standby and its sensors would all be another disk's.
    fn check_link(&self, c: &VolumeChoice) -> Result<(), StartError> {
        if self.deps.machine.has_link(Path::new(&c.folder)) {
            return Err(StartError::Disk("link"));
        }
        Ok(())
    }

    /// `performance.diskFolder` (DC15), once the test is sure to start.
    pub(super) fn remember_disk_folder(&self, folder: &str) {
        let folder = folder.to_owned();
        self.deps
            .settings
            .update_with(|s| s.performance.disk_folder = Some(folder));
    }

    /// Probes the folder (one tiny file, DC6) into `c`: free space, sync attributes.
    fn probe_into(&self, c: &mut VolumeChoice) -> Result<(), StartError> {
        let p = self.deps.machine.probe_folder(Path::new(&c.folder));
        if p.remote {
            return Err(StartError::Disk("remote"));
        }
        if !p.exists {
            return Err(StartError::Disk("not_found"));
        }
        if !p.writable {
            return Err(StartError::Disk("not_writable"));
        }
        c.free_bytes = p.free_bytes;
        if p.total_bytes > 0 {
            c.total_bytes = p.total_bytes;
        }
        c.sync |= p.sync;
        Ok(())
    }

    /// `performance_disk_probe`: the volume of a folder the user picked or confirmed. A
    /// spun-down HDD is not probed (the probe file would wake it): it shows `standby`.
    pub fn disk_probe(&self, folder: &str) -> Result<VolumeChoice, StartError> {
        let mut c = self.disk_volume(folder)?;
        if !c.standby {
            self.probe_into(&mut c)?;
        }
        Ok(c)
    }

    /// The target of a disk test about to start (DC6, DC11): a spun-down HDD only with
    /// `wake`, then the links (R13), the orphaned files of the folder, the probe and
    /// the space. The caller remembers the folder once its plan is built.
    pub(super) fn disk_target(
        &self,
        folder: &str,
        wake: bool,
        need: DiskNeed,
    ) -> Result<VolumeChoice, StartError> {
        let mut c = self.disk_volume(folder)?;
        if c.standby && !wake {
            return Err(StartError::Disk("standby"));
        }
        if c.standby {
            self.check_link(&c)?;
        }
        let m = &self.deps.machine;
        orphans::sweep(Path::new(&c.folder), |pid| m.process_started_at(pid));
        self.probe_into(&mut c)?;
        let enough = match need {
            DiskNeed::Bench => {
                c.free_bytes.saturating_sub(disk_reserve(c.total_bytes)) >= DISK_BENCH_FILE
            }
            DiskNeed::Stress => stress_file_bytes(c.free_bytes, c.total_bytes).is_some(),
        };
        if !enough {
            return Err(StartError::Disk("no_space"));
        }
        Ok(c)
    }

    /// The disk's stop threshold (DC10): `diskStopC`, else its WCTEMP, else 70 °C.
    pub(super) fn disk_threshold(&self, device_id: &str) -> f64 {
        let setting = self.deps.settings.snapshot().performance.disk_stop_c;
        disk_stop_threshold(setting, wctemp(&(self.deps.schema)(), device_id))
    }

    /// The plan for `request` on this machine, with its topology and, for a GPU test,
    /// the GPU of `request.gpu` (its LUID changes at every boot, DG13); a disk test
    /// gets its volume in `disk`.
    fn plan(
        &self,
        request: &StartRequest,
        seed: u64,
        disk: Option<&VolumeChoice>,
    ) -> Result<(Topology, Plan, Option<StressAdapter>), StartError> {
        let m = &self.deps.machine;
        let adapter = match (&request.component, &request.gpu) {
            (Component::Gpu, Some(id)) => m.gpus().into_iter().find(|g| &g.device_id == id),
            _ => None,
        };
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
            // None for a GPU request gives `NoGpu`.
            gpu: adapter.as_ref().map(|g| GpuTarget {
                luid: g.luid,
                integrated: g.integrated,
            }),
            // None for a disk request gives `NoDisk`.
            disk: disk
                .filter(|_| request.component == Component::Disk)
                .map(|c| DiskPlanInput {
                    dir: c.folder.clone(),
                    free_bytes: c.free_bytes,
                    volume_bytes: c.total_bytes,
                    removable: c.removable,
                }),
        })
        .map_err(StartError::Plan)?;
        Ok((topology, plan, adapter))
    }

    /// The plan a start would run (the seed aside). A disk request reads its volume
    /// from metadata only: no probe, no sweep, a spun-down HDD stays asleep.
    pub fn preview(&self, request: &StartRequest) -> Result<Plan, StartError> {
        let disk = match (&request.component, &request.disk) {
            (Component::Disk, Some(d)) => Some(self.disk_volume(&d.folder)?),
            _ => None,
        };
        Ok(self.plan(request, 0, disk.as_ref())?.1)
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
            gpus: m
                .gpus()
                .into_iter()
                .map(|g| GpuChoice {
                    device_id: g.device_id,
                    name: g.name,
                    integrated: g.integrated,
                    dedicated_bytes: g.dedicated_bytes,
                })
                .collect(),
            volumes: {
                let drives = m.drives();
                m.volumes()
                    .into_iter()
                    .filter(|v| v.drive != DriveKind::Remote)
                    .map(|v| {
                        let folder = self.default_folder(&v);
                        self.volume_choice(&v, &drives, folder)
                    })
                    .collect()
            },
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
        let target = match (&request.component, &request.disk) {
            (Component::Disk, Some(d)) => {
                Some(self.disk_target(&d.folder, d.wake, DiskNeed::Stress)?)
            }
            _ => None,
        };
        let (topology, plan, adapter) = self.plan(&request, seed, target.as_ref())?;
        if let Some(c) = &target {
            self.remember_disk_folder(&c.folder);
        }
        let perf = self.deps.settings.snapshot().performance.clone();
        let gpu = adapter.is_some();
        let disk_id = target
            .as_ref()
            .map(|c| c.device_id.clone().unwrap_or_default());
        let config = RunConfig {
            threshold_c: if let Some(id) = &disk_id {
                self.disk_threshold(id)
            } else if gpu {
                gpu_stop_threshold(perf.gpu_stop_c)
            } else {
                let tjmax_c = resolve_cpu_sensors(&(self.deps.schema)(), 0).tjmax_c;
                cpu_stop_threshold(perf.cpu_stop_c, tjmax_c)
            },
            thermal_stop: perf.thermal_stop,
            // GPU and disk readings come without the service (DG12, DC10).
            service_available: gpu || disk_id.is_some() || (self.deps.service_available)(),
            cores: if gpu || disk_id.is_some() {
                vec![]
            } else {
                core_order(&topology)
            },
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
            Component::Gpu => adapter
                .as_ref()
                .map_or_else(String::new, |g| g.name.clone()),
            Component::Disk => target.as_ref().map_or_else(String::new, |c| {
                c.model.clone().unwrap_or_else(|| c.root.clone())
            }),
        };
        let disk = target.as_ref().map(|c| DiskSession {
            device_id: c.device_id.clone().unwrap_or_default(),
            volume: c.root.trim_end_matches('\\').to_owned(),
            kind: kind_name(c.kind),
            file_bytes: plan.disk.as_ref().map_or(0, |d| d.file_bytes),
            ..Default::default()
        });
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
            stability: None,
            gpu_device_id: adapter.as_ref().map(|g| g.device_id.clone()),
            disk,
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
            gpu: adapter.map(|g| g.device_id),
            disk: disk_id,
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

    /// The CPU (or, for a GPU or disk test, the GPU or disk) reading of a sampler tick for the test in
    /// progress. Never blocks: a busy runner skips the tick, a full queue drops the sample.
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
            let ids = match (&a.gpu, &a.disk) {
                (Some(id), _) => SensorIds::Gpu(resolve_gpu_sensors(schema, id)),
                (None, Some(id)) => SensorIds::Disk(resolve_disk_sensors(schema, id)),
                (None, None) => SensorIds::Cpu(resolve_cpu_sensors(schema, a.cores)),
            };
            a.sensors = Some((schema.revision, ids));
        }
        let Some((_, ids)) = &a.sensors else { return };
        let (sample, service) = match ids {
            SensorIds::Cpu(ids) => (
                read_sample(ids, &out.snapshot, &out.quality),
                (self.deps.service_available)(),
            ),
            // The GPU readings never need the service (DG12).
            SensorIds::Gpu(ids) => (read_gpu_sample(ids, &out.snapshot, &out.quality), true),
            // Nor do the disk's (DC10).
            SensorIds::Disk(ids) => (read_disk_sample(ids, &out.snapshot, &out.quality), true),
        };
        let _ = a.samples.try_send((sample, service));
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
    /// When the PCIe replay counter was last read (mono ms); GPU tests only.
    pcie_at: Option<u64>,
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
            pcie_at: None,
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

    /// Reads the GPU's PCIe replay counter at the start and every 5 s (DG14), on this
    /// thread: the sampler's tick never waits for NVML.
    fn poll_pcie(&mut self) {
        let Some(id) = self.ctl.session().gpu_device_id.clone() else {
            return;
        };
        let now = self.worker.clock();
        if self.done || !pcie_due(self.pcie_at, now.mono_ms) {
            return;
        }
        self.pcie_at = Some(now.mono_ms);
        if let Some(count) = self.worker.deps.machine.pcie_replay(&id) {
            let a = self.ctl.on_pcie_replay(count, now);
            self.exec(a);
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
            self.poll_pcie();
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
        assert_eq!(wire(BuildError::NoGpu), "build:no_gpu");
        assert_eq!(wire(BuildError::NoDisk), "build:no_disk");
        assert_eq!(wire(BuildError::NoSpace), "build:no_space");
        for code in [
            "remote",
            "not_found",
            "not_writable",
            "no_space",
            "standby",
            "link",
        ] {
            assert_eq!(StartError::Disk(code).wire(), format!("disk:{code}"));
        }
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
        pub(crate) gpus: Vec<oma_win::gpu::StressAdapter>,
        /// The PCIe replay counts the polls give, in order; the last one repeats.
        pub(crate) pcie: Arc<Mutex<Vec<u32>>>,
        /// The device ids the PCIe replay counter was read for.
        pub(crate) pcie_polls: Arc<Mutex<Vec<String>>>,
        /// What each read of the other processes' GPU share gives.
        pub(crate) gpu_busy: Option<f64>,
        /// The device ids the GPU share was read for.
        pub(crate) gpu_busy_polls: Arc<Mutex<Vec<String>>>,
        pub(crate) volumes: Vec<oma_win::volumes::VolumeInfo>,
        /// What every probe gives; `None`: the folder exists and is writable, with its
        /// volume's space.
        pub(crate) probe: Option<FolderProbe>,
        /// The folders probed.
        pub(crate) probes: Arc<Mutex<Vec<PathBuf>>>,
        /// Every disk's traits; `None`: an NVMe disk.
        pub(crate) traits: Option<DiskTraits>,
        pub(crate) drives: Vec<DriveEntry>,
        /// Every disk's power state.
        pub(crate) power: Option<DiskPower>,
        /// Live processes and their start times.
        pub(crate) started: Vec<(u32, u64)>,
        pub(crate) temp: PathBuf,
        /// Every folder is behind a junction (R13).
        pub(crate) linked: bool,
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
        fn gpus(&self) -> Vec<oma_win::gpu::StressAdapter> {
            self.gpus.clone()
        }
        fn pcie_replay(&self, device_id: &str) -> Option<u32> {
            self.pcie_polls.lock().unwrap().push(device_id.to_owned());
            let mut values = self.pcie.lock().unwrap();
            match values.len() {
                0 => None,
                1 => Some(values[0]),
                _ => Some(values.remove(0)),
            }
        }
        fn gpu_busy_share(&self, device_id: &str) -> Option<f64> {
            self.gpu_busy_polls
                .lock()
                .unwrap()
                .push(device_id.to_owned());
            self.gpu_busy
        }
        fn volumes(&self) -> Vec<oma_win::volumes::VolumeInfo> {
            self.volumes.clone()
        }
        fn probe_folder(&self, path: &Path) -> FolderProbe {
            self.probes.lock().unwrap().push(path.to_owned());
            if let Some(p) = &self.probe {
                return p.clone();
            }
            let root = root_of(&path.to_string_lossy());
            let v = self.volumes.iter().find(|v| Some(&v.root) == root.as_ref());
            FolderProbe {
                volume_root: root.clone(),
                remote: v.is_some_and(|v| v.drive == DriveKind::Remote),
                exists: true,
                writable: true,
                sync: false,
                free_bytes: v.map_or(0, |v| v.free_bytes),
                total_bytes: v.map_or(0, |v| v.total_bytes),
            }
        }
        fn disk_traits(&self, _index: u32) -> DiskTraits {
            self.traits.unwrap_or(DiskTraits {
                bus: Some(17),
                class: DiskClass::NonRotational,
                kind: DiskKind::Nvme,
            })
        }
        fn drives(&self) -> Vec<DriveEntry> {
            self.drives.clone()
        }
        fn disk_power(&self, _device_id: &str) -> Option<DiskPower> {
            self.power
        }
        fn process_started_at(&self, pid: u32) -> Option<u64> {
            self.started
                .iter()
                .find(|(p, _)| *p == pid)
                .map(|(_, t)| *t)
        }
        fn local_temp_dir(&self) -> PathBuf {
            self.temp.clone()
        }
        fn has_link(&self, _path: &Path) -> bool {
            self.linked
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

    impl Rig {
        /// The disk tests' folder, inside the store's (removed with it).
        pub(crate) fn disk_dir(&self) -> PathBuf {
            let dir = self._dir.0.join("disk");
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }
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
        let schema = Schema {
            revision: 1,
            devices: vec![],
            sensors: vec![],
        };
        rig_schema(name, launcher, machine, schema)
    }

    /// A rig whose engine has `schema` (WCTEMP, Tjmax).
    pub(crate) fn rig_schema(
        name: &str,
        launcher: Launcher,
        machine: FakeMachine,
        schema: Schema,
    ) -> Rig {
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
            schema: Box::new(move || schema.clone()),
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
                    shader_digest: None,
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
            gpu: None,
            disk: None,
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
            load_percent: None,

            disk: None,
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
            load_percent: None,

            transient: None,
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
            fn gpus(&self) -> Vec<oma_win::gpu::StressAdapter> {
                self.0.gpus()
            }
            fn pcie_replay(&self, device_id: &str) -> Option<u32> {
                self.0.pcie_replay(device_id)
            }
            fn gpu_busy_share(&self, device_id: &str) -> Option<f64> {
                self.0.gpu_busy_share(device_id)
            }
            fn volumes(&self) -> Vec<oma_win::volumes::VolumeInfo> {
                self.0.volumes()
            }
            fn probe_folder(&self, path: &Path) -> FolderProbe {
                self.0.probe_folder(path)
            }
            fn disk_traits(&self, index: u32) -> DiskTraits {
                self.0.disk_traits(index)
            }
            fn drives(&self) -> Vec<DriveEntry> {
                self.0.drives()
            }
            fn disk_power(&self, device_id: &str) -> Option<DiskPower> {
                self.0.disk_power(device_id)
            }
            fn process_started_at(&self, pid: u32) -> Option<u64> {
                self.0.process_started_at(pid)
            }
            fn local_temp_dir(&self) -> PathBuf {
                self.0.local_temp_dir()
            }
            fn has_link(&self, path: &Path) -> bool {
                self.0.has_link(path)
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
            gpu: None,

            disk: None,
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
                    shader_digest: None,
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
    fn verdict_text_localizes_the_stability() {
        let detail = OutcomeDetail {
            verdict: "low_stability".into(),
            params: [("stability".to_string(), "95.3".to_string())].into(),
            phase: None,
            kernel: None,
            core: None,
            temp_c: None,
            clock_mhz: None,
            at_ms: None,
        };
        assert!(verdict_text(Lang::It, &detail).contains("95,3"));
        assert!(verdict_text(Lang::En, &detail).contains("95.3"));
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

    pub(crate) const GPU_ID: &str = "gpu/pci-10de-2704";

    pub(crate) fn gpu_machine() -> FakeMachine {
        FakeMachine {
            gpus: vec![
                oma_win::gpu::StressAdapter {
                    luid: 0x77,
                    device_id: "gpu/0".into(),
                    name: "Test iGPU".into(),
                    vendor_id: 0x1002,
                    integrated: true,
                    dedicated_bytes: 512 << 20,
                },
                oma_win::gpu::StressAdapter {
                    luid: 0xABCD,
                    device_id: GPU_ID.into(),
                    name: "Test RTX".into(),
                    vendor_id: 0x10de,
                    integrated: false,
                    dedicated_bytes: 16 << 30,
                },
            ],
            ..Default::default()
        }
    }

    fn gpu_request(device: Option<&str>) -> StartRequest {
        StartRequest {
            component: Component::Gpu,
            gpu: device.map(str::to_owned),
            ..request()
        }
    }

    pub(crate) fn gpu_schema() -> Schema {
        use oma_core::model::{Label, Sensor, SensorKind, Source, Unit};
        let sensor = |device: &str, kind, name: &str, unit| {
            Sensor::new(device, kind, name, unit, Label::new(name), Source::Nvml)
        };
        Schema {
            revision: 1,
            devices: vec![],
            sensors: vec![
                sensor(GPU_ID, SensorKind::Temperature, "core", Unit::Celsius),
                sensor(GPU_ID, SensorKind::Power, "board", Unit::Watt),
                sensor(GPU_ID, SensorKind::Clock, "core", Unit::Megahertz),
                sensor(GPU_ID, SensorKind::Flag, "throttle-power", Unit::Boolean),
                // A CPU temperature the GPU test must not read.
                sensor("cpu/0", SensorKind::Temperature, "tdie", Unit::Celsius),
            ],
        }
    }

    pub(crate) fn gpu_tick() -> TickOutput {
        use oma_core::provider::Quality;
        TickOutput {
            snapshot: oma_core::model::Snapshot {
                revision: 1,
                seq: 0,
                timestamp_ms: 0,
                values: vec![Some(71.0), Some(250.0), Some(2600.0), Some(0.0), Some(40.0)],
            },
            schema: None,
            quality: vec![Quality::Fresh; 5],
            health: None,
            entries: vec![],
            monotonic_ms: 0,
        }
    }

    fn running_script() -> Script {
        Script {
            on_run: vec![progress(0)],
            on_stop: vec![finished(FinishReason::Stopped, 0)],
            exit_after_stop: true,
            ..Default::default()
        }
    }

    /// Ticks until a GPU sample reaches the status.
    fn feed_gpu_sample(runner: &PerformanceRunner) {
        let until = Instant::now() + Duration::from_secs(3);
        while runner.status().temp_c.is_none() {
            assert!(Instant::now() < until, "no GPU sample arrived");
            runner.on_tick(&gpu_tick(), &gpu_schema());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn system_lists_gpus() {
        let rig = rig_on("sysgpu", scripted(Script::default()), gpu_machine());
        let gpus = rig.runner.system().gpus;
        assert_eq!(
            gpus,
            [
                GpuChoice {
                    device_id: "gpu/0".into(),
                    name: "Test iGPU".into(),
                    integrated: true,
                    dedicated_bytes: 512 << 20,
                },
                GpuChoice {
                    device_id: GPU_ID.into(),
                    name: "Test RTX".into(),
                    integrated: false,
                    dedicated_bytes: 16 << 30,
                },
            ]
        );
        let json = serde_json::to_value(&gpus[1]).unwrap();
        assert_eq!(json["deviceId"], GPU_ID);
        assert_eq!(json["dedicatedBytes"], 16u64 << 30);
    }

    #[test]
    fn gpu_request_resolves_luid_from_device_id() {
        let rig = rig_on("gpuluid", scripted(Script::default()), gpu_machine());
        let plan = rig.runner.preview(&gpu_request(Some(GPU_ID))).unwrap();
        let target = plan.gpu.unwrap();
        assert_eq!((target.luid, target.integrated), (0xABCD, false));
        let plan = rig.runner.preview(&gpu_request(Some("gpu/0"))).unwrap();
        assert_eq!(plan.gpu.unwrap().luid, 0x77);
    }

    #[test]
    fn unknown_gpu_device_is_no_gpu() {
        let rig = rig_on("nogpu", scripted(Script::default()), gpu_machine());
        for request in [gpu_request(Some("gpu/pci-1002-0000")), gpu_request(None)] {
            let err = rig.runner.preview(&request).unwrap_err();
            assert_eq!(err.wire(), "build:no_gpu");
            let err = rig.runner.start(request).unwrap_err();
            assert_eq!(err.wire(), "build:no_gpu");
        }
        assert!(!rig.runner.is_running());
    }

    #[test]
    fn gpu_start_uses_the_gpu_threshold() {
        let rig = rig_on("gputhr", scripted(running_script()), gpu_machine());
        rig.runner
            .deps
            .settings
            .update_with(|s| s.performance.gpu_stop_c = 83);
        rig.runner.start(gpu_request(Some(GPU_ID))).unwrap();
        assert_eq!(rig.runner.status().stop_c, Some(83.0));
        rig.runner.stop();
        wait_idle(&rig.runner);
    }

    #[test]
    fn gpu_samples_come_from_gpu_sensors() {
        let rig = rig_on("gpusample", scripted(running_script()), gpu_machine());
        rig.runner.start(gpu_request(Some(GPU_ID))).unwrap();
        wait_running(&rig.runner);
        feed_gpu_sample(&rig.runner);
        let st = rig.runner.status();
        assert_eq!(st.temp_c, Some(71.0), "the GPU core, not the CPU");
        assert_eq!(st.power_w, Some(250.0));
        assert_eq!(st.clock_mhz, Some(2600.0));
        rig.runner.stop();
        wait_idle(&rig.runner);
    }

    #[test]
    fn gpu_test_never_warns_no_service() {
        // The rig's service is never connected.
        let rig = rig_on("gpunoservice", scripted(running_script()), gpu_machine());
        rig.runner.start(gpu_request(Some(GPU_ID))).unwrap();
        wait_running(&rig.runner);
        feed_gpu_sample(&rig.runner);
        let warnings = rig.runner.status().warnings;
        assert!(!warnings.iter().any(|w| w == "noService"), "{warnings:?}");
        rig.runner.stop();
        wait_idle(&rig.runner);
    }

    #[test]
    fn pcie_replay_is_polled_every_five_seconds() {
        assert!(pcie_due(None, 0));
        assert!(!pcie_due(Some(0), 4_999));
        assert!(pcie_due(Some(0), 5_000));
        assert!(!pcie_due(Some(5_000), 9_999));
        // About 6 s of test: a poll at the start and one at 5 s, the second higher.
        let mut on_run: Vec<LoadMessage> = (0..120).map(|_| progress(0)).collect();
        on_run.push(finished(FinishReason::Completed, 0));
        let machine = gpu_machine();
        *machine.pcie.lock().unwrap() = vec![10, 12];
        let polls = Arc::clone(&machine.pcie_polls);
        let rig = rig_on(
            "pcie",
            scripted(Script {
                on_run,
                exit_after_run: true,
                spread: Some(Duration::from_millis(50)),
                ..Default::default()
            }),
            machine,
        );
        rig.runner.start(gpu_request(Some(GPU_ID))).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while rig.runner.is_running() {
            assert!(Instant::now() < until, "the runner did not finish");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(*polls.lock().unwrap(), [GPU_ID, GPU_ID]);
        let s = only_session(&rig.runner);
        assert!(
            s.events.iter().any(|e| e.code == "pcie_replay"),
            "{:?}",
            s.events
        );
    }

    #[test]
    fn cpu_tests_never_read_the_pcie_counter() {
        let machine = gpu_machine();
        let polls = Arc::clone(&machine.pcie_polls);
        let rig = rig_on(
            "cpunopcie",
            scripted(Script {
                on_run: vec![progress(0), finished(FinishReason::Completed, 0)],
                exit_after_run: true,
                ..Default::default()
            }),
            machine,
        );
        rig.runner.start(request()).unwrap();
        wait_idle(&rig.runner);
        assert!(polls.lock().unwrap().is_empty());
        assert_eq!(only_session(&rig.runner).gpu_device_id, None);
    }

    #[test]
    fn gpu_session_records_the_device() {
        let rig = rig_on(
            "gpudevice",
            scripted(Script {
                on_run: vec![progress(0), finished(FinishReason::Completed, 0)],
                exit_after_run: true,
                ..Default::default()
            }),
            gpu_machine(),
        );
        rig.runner.start(gpu_request(Some(GPU_ID))).unwrap();
        wait_idle(&rig.runner);
        let s = only_session(&rig.runner);
        assert_eq!(s.component, Component::Gpu);
        assert_eq!(s.device, "Test RTX");
        assert_eq!(s.gpu_device_id.as_deref(), Some(GPU_ID));
    }

    #[test]
    fn busy_share_excludes_our_tree_dwm_and_system() {
        let row = |pid: u32, name: &str, load: Option<f64>| oma_win::gpu::GpuProcess {
            pid,
            name: name.into(),
            load_percent: load,
            engine: None,
            dedicated_bytes: None,
            shared_bytes: None,
        };
        let ours = std::collections::HashSet::from([100, 101]);
        // An empty (or stale) table: nothing to say.
        assert_eq!(other_gpu_share(&[], &ours), None);
        let mine = [
            row(0, "Idle", Some(90.0)),
            row(4, "System", Some(80.0)),
            row(100, "oma-app.exe", Some(70.0)),
            row(101, "msedgewebview2.exe", Some(60.0)),
            row(200, "DWM.EXE", Some(50.0)),
        ];
        assert_eq!(other_gpu_share(&mine, &ours), Some(0.0));
        let mut rows = mine.to_vec();
        rows.extend([row(300, "game.exe", Some(25.0)), row(301, "new.exe", None)]);
        assert_eq!(other_gpu_share(&rows, &ours), Some(0.25));
    }

    // ---- Disk (M8c) ----

    pub(crate) const DISK_ID: &str = "disk/test-0";
    const GIB: u64 = 1 << 30;

    /// The root of the temporary folder, where every disk test of the rig lives.
    pub(crate) fn temp_root() -> String {
        root_of(&std::env::temp_dir().to_string_lossy()).unwrap()
    }

    pub(crate) fn volume(root: &str, free: u64, drive: DriveKind) -> oma_win::volumes::VolumeInfo {
        oma_win::volumes::VolumeInfo {
            root: root.into(),
            label: "Data".into(),
            fs: "NTFS".into(),
            total_bytes: 1000 * GIB,
            free_bytes: free,
            drive,
            disk_index: Some(0),
            system: false,
        }
    }

    /// An NVMe disk holding the temporary folder's volume, 500 GiB free.
    pub(crate) fn disk_machine() -> FakeMachine {
        FakeMachine {
            volumes: vec![
                volume(&temp_root(), 500 * GIB, DriveKind::Fixed),
                oma_win::volumes::VolumeInfo {
                    disk_index: None,
                    ..volume("Z:\\", 0, DriveKind::Remote)
                },
            ],
            drives: vec![DriveEntry::new(
                0,
                DISK_ID.into(),
                Some("Test NVMe".into()),
                Some("SN1".into()),
            )],
            temp: PathBuf::from(r"C:\Users\t\AppData\Local\Temp"),
            ..Default::default()
        }
    }

    pub(crate) fn hdd() -> DiskTraits {
        DiskTraits {
            bus: Some(11),
            class: DiskClass::RotationalOrUnknown,
            kind: DiskKind::Hdd,
        }
    }

    fn disk_request(folder: &Path, wake: bool) -> StartRequest {
        StartRequest {
            component: Component::Disk,
            disk: Some(oma_core::load::DiskStart {
                folder: folder.to_string_lossy().into_owned(),
                wake,
            }),
            ..request()
        }
    }

    fn passed_script() -> Script {
        Script {
            on_run: vec![progress(0), finished(FinishReason::Completed, 0)],
            exit_after_run: true,
            ..Default::default()
        }
    }

    #[test]
    fn system_lists_volumes() {
        let mut machine = disk_machine();
        machine.volumes[0].system = true;
        machine.volumes.push(oma_win::volumes::VolumeInfo {
            drive: DriveKind::Removable,
            ..volume("E:\\", GIB, DriveKind::Removable)
        });
        let probes = Arc::clone(&machine.probes);
        let rig = rig_on("sysvol", scripted(Script::default()), machine);
        let volumes = rig.runner.system().volumes;
        // The network drive is left out.
        assert_eq!(volumes.len(), 2, "{volumes:?}");
        let sys = &volumes[0];
        assert_eq!(sys.folder, r"C:\Users\t\AppData\Local\Temp");
        assert_eq!(sys.device_id.as_deref(), Some(DISK_ID));
        assert_eq!(sys.model.as_deref(), Some("Test NVMe"));
        assert_eq!(
            (sys.kind, sys.removable, sys.standby),
            (DiskKind::Nvme, false, false)
        );
        let usb = &volumes[1];
        assert_eq!(usb.folder, "E:\\");
        assert!(usb.removable);
        let json = serde_json::to_value(sys).unwrap();
        assert_eq!(json["deviceId"], DISK_ID);
        assert_eq!(json["kind"], "nvme");
        assert_eq!(json["totalBytes"], 1000 * GIB);
        assert!(json.get("solid").is_none());
        // Listing never probes (the probe writes a file).
        assert!(probes.lock().unwrap().is_empty());
    }

    #[test]
    fn disk_stress_runs_and_saves_the_disk_session() {
        let rig = rig_on("diskrun", scripted(passed_script()), disk_machine());
        let folder = rig.disk_dir();
        let id = rig.runner.start(disk_request(&folder, false)).unwrap();
        wait_idle(&rig.runner);
        let s = only_session(&rig.runner);
        assert_eq!(s.id, id);
        assert_eq!(s.component, Component::Disk);
        assert_eq!(s.device, "Test NVMe");
        let d = s.disk.unwrap();
        assert_eq!(d.device_id, DISK_ID);
        assert_eq!(d.volume, temp_root().trim_end_matches('\\'));
        assert_eq!(d.kind, "nvme");
        assert_eq!(d.file_bytes, 8 * GIB);
        let target = s.plan.disk.unwrap();
        assert_eq!(target.reserve_bytes, 50 * GIB);
        // The folder is not saved in the session (DC10), neither in the request nor in the plan.
        assert_eq!(s.request.disk.unwrap().folder, "");
        assert_eq!(target.dir, "");
        let dir = rig._dir.0.join("stress");
        let name = std::fs::read_dir(&dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let raw = std::fs::read_to_string(name).unwrap();
        let unique = rig
            ._dir
            .0
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            !raw.contains(&unique),
            "the folder leaked into the file: {raw}"
        );
    }

    #[test]
    fn standby_hdd_needs_consent() {
        let machine = FakeMachine {
            traits: Some(hdd()),
            power: Some(DiskPower::Standby),
            ..disk_machine()
        };
        let probes = Arc::clone(&machine.probes);
        let rig = rig_on("standby", scripted(passed_script()), machine);
        let folder = rig.disk_dir();
        let orphan = plant_orphan(&folder);
        let err = rig.runner.start(disk_request(&folder, false)).unwrap_err();
        assert_eq!(err.wire(), "disk:standby");
        assert!(orphan.exists(), "no sweep before the consent");
        let bench = super::super::bench::DiskBenchRequest {
            folder: folder.to_string_lossy().into_owned(),
            profile: oma_core::scores::DiskProfile::B1,
            compressible: false,
            wake: false,
        };
        assert_eq!(
            rig.runner.start_disk_bench(bench).unwrap_err().wire(),
            "disk:standby"
        );
        // The pick shows the standby without writing the probe file.
        let c = rig.runner.disk_probe(&folder.to_string_lossy()).unwrap();
        assert!(c.standby);
        assert!(
            probes.lock().unwrap().is_empty(),
            "never probed while asleep"
        );
        assert!(rig.runner.store().list().is_empty());
        assert_eq!(
            rig.runner.deps.settings.snapshot().performance.disk_folder,
            None
        );
        // With consent the test starts.
        rig.runner.start(disk_request(&folder, true)).unwrap();
        assert!(!orphan.exists());
        wait_idle(&rig.runner);
        assert_eq!(only_session(&rig.runner).outcome, Some(Outcome::Passed));
        assert_eq!(probes.lock().unwrap().len(), 1);
    }

    #[test]
    fn remote_folder_is_refused() {
        let rig = rig_on("remote", scripted(passed_script()), disk_machine());
        for folder in [r"\\server\share\x", r"\\?\UNC\server\share", r"Z:\x"] {
            let err = rig
                .runner
                .start(disk_request(Path::new(folder), false))
                .unwrap_err();
            assert_eq!(err.wire(), "disk:remote", "{folder}");
            assert_eq!(
                rig.runner.disk_probe(folder).unwrap_err().wire(),
                "disk:remote"
            );
        }
        // Relative, `..` and too long: not a folder a test can use.
        let long = format!("{}{}", temp_root(), "a".repeat(1100));
        for folder in ["relative\\x", r"C:\a\..\b", &long] {
            let err = rig.runner.disk_probe(folder).unwrap_err();
            assert_eq!(err.wire(), "disk:not_found", "{folder}");
        }
        // A folder the probe cannot write into.
        let machine = FakeMachine {
            probe: Some(FolderProbe {
                volume_root: Some(temp_root()),
                remote: false,
                exists: true,
                writable: false,
                sync: false,
                free_bytes: 500 * GIB,
                total_bytes: 1000 * GIB,
            }),
            ..disk_machine()
        };
        let rig2 = rig_on("notwritable", scripted(passed_script()), machine);
        let folder = rig2.disk_dir();
        let err = rig2.runner.start(disk_request(&folder, false)).unwrap_err();
        assert_eq!(err.wire(), "disk:not_writable");
        assert!(!rig.runner.is_running() && !rig2.runner.is_running());
        assert!(rig2.runner.store().list().is_empty());
    }

    #[test]
    fn too_little_space_is_no_space() {
        // 1 TB volume: a 50 GiB reserve, and 50.5 GiB free leaves half a GiB.
        let mut machine = disk_machine();
        machine.volumes[0].free_bytes = 50 * GIB + GIB / 2;
        let rig = rig_on("nospace", scripted(passed_script()), machine);
        let folder = rig.disk_dir();
        let err = rig.runner.start(disk_request(&folder, false)).unwrap_err();
        assert_eq!(err.wire(), "disk:no_space");
        let bench = super::super::bench::DiskBenchRequest {
            folder: folder.to_string_lossy().into_owned(),
            profile: oma_core::scores::DiskProfile::B1,
            compressible: false,
            wake: false,
        };
        assert_eq!(
            rig.runner.start_disk_bench(bench).unwrap_err().wire(),
            "disk:no_space"
        );
        assert!(!rig.runner.is_running());
        // Nothing remembered either.
        assert_eq!(
            rig.runner.deps.settings.snapshot().performance.disk_folder,
            None
        );
    }

    #[test]
    fn disk_start_writes_the_folder_to_settings_and_journal() {
        let rig = rig_on("diskfolder", scripted(running_script()), disk_machine());
        let folder = rig.disk_dir();
        let text = folder.to_string_lossy().into_owned();
        rig.runner.start(disk_request(&folder, false)).unwrap();
        assert_eq!(
            rig.runner
                .deps
                .settings
                .snapshot()
                .performance
                .disk_folder
                .as_deref(),
            Some(text.as_str())
        );
        wait_running(&rig.runner);
        let until = Instant::now() + Duration::from_secs(3);
        let journal = loop {
            if let Some(Ok(j)) = rig.runner.store().read_journal() {
                break j;
            }
            assert!(Instant::now() < until, "no journal");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(journal.disk_folder.as_deref(), Some(text.as_str()));
        rig.runner.stop();
        wait_idle(&rig.runner);
    }

    fn disk_schema(warning: Option<&str>) -> Schema {
        use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
        let sensor = |kind, name: &str, unit| {
            Sensor::new(DISK_ID, kind, name, unit, Label::new(name), Source::Lhm)
        };
        Schema {
            revision: 1,
            devices: vec![Device {
                id: DISK_ID.into(),
                kind: DeviceKind::Storage,
                name: "Test NVMe".into(),
                vendor: None,
                properties: warning
                    .map(|w| [("tempWarningC".to_owned(), w.to_owned())].into())
                    .unwrap_or_default(),
            }],
            sensors: vec![
                sensor(SensorKind::Temperature, "drive", Unit::Celsius),
                sensor(SensorKind::Throughput, "read", Unit::BytesPerSecond),
                sensor(SensorKind::Throughput, "write", Unit::BytesPerSecond),
                sensor(SensorKind::Data, "host-written", Unit::Bytes),
                // A CPU temperature the disk test must not read.
                Sensor::new(
                    "cpu/0",
                    SensorKind::Temperature,
                    "tdie",
                    Unit::Celsius,
                    Label::new("tdie"),
                    Source::Lhm,
                ),
            ],
        }
    }

    pub(crate) fn disk_tick() -> TickOutput {
        use oma_core::provider::Quality;
        TickOutput {
            snapshot: oma_core::model::Snapshot {
                revision: 1,
                seq: 0,
                timestamp_ms: 0,
                values: vec![
                    Some(48.0),
                    Some(3e9),
                    Some(1e9),
                    Some(100.0 * (1u64 << 30) as f64),
                    Some(90.0),
                ],
            },
            schema: None,
            quality: vec![Quality::Fresh; 5],
            health: None,
            entries: vec![],
            monotonic_ms: 0,
        }
    }

    #[test]
    fn disk_samples_come_from_disk_sensors() {
        let rig = rig_on("disksample", scripted(running_script()), disk_machine());
        let folder = rig.disk_dir();
        rig.runner.start(disk_request(&folder, false)).unwrap();
        wait_running(&rig.runner);
        let schema = disk_schema(None);
        let until = Instant::now() + Duration::from_secs(3);
        while rig.runner.status().temp_c.is_none() {
            assert!(Instant::now() < until, "no disk sample arrived");
            rig.runner.on_tick(&disk_tick(), &schema);
            std::thread::sleep(Duration::from_millis(20));
        }
        let st = rig.runner.status();
        assert_eq!(st.temp_c, Some(48.0), "the drive, not the CPU");
        // The service is not connected, and the disk test does not need it.
        assert!(
            !st.warnings.iter().any(|w| w == "noService"),
            "{:?}",
            st.warnings
        );
        assert!(
            !st.warnings.iter().any(|w| w == "smartMissing"),
            "{:?}",
            st.warnings
        );
        rig.runner.stop();
        wait_idle(&rig.runner);
        let d = only_session(&rig.runner).disk.unwrap();
        assert_eq!(d.host_written_before_gib, Some(100.0));
    }

    #[test]
    fn disk_threshold_uses_wctemp() {
        let rig = rig_schema(
            "diskthr",
            scripted(running_script()),
            disk_machine(),
            disk_schema(Some("65")),
        );
        let folder = rig.disk_dir();
        rig.runner.start(disk_request(&folder, false)).unwrap();
        assert_eq!(rig.runner.status().stop_c, Some(65.0));
        rig.runner.stop();
        wait_idle(&rig.runner);
        rig.runner
            .deps
            .settings
            .update_with(|s| s.performance.disk_stop_c = Some(60));
        assert_eq!(rig.runner.disk_threshold(DISK_ID), 60.0);
        // Without the setting and without WCTEMP: 70 °C.
        let rig = rig_on("diskthr70", scripted(Script::default()), disk_machine());
        assert_eq!(rig.runner.disk_threshold(DISK_ID), 70.0);
    }

    /// An orphan of a dead run in `dir`: the data file and its sidecar.
    fn plant_orphan(dir: &Path) -> PathBuf {
        let prefix = "oma-test-00000000000000aa";
        let data = dir.join(format!("{prefix}.bin"));
        std::fs::write(&data, b"x").unwrap();
        std::fs::write(
            dir.join(format!("{prefix}.oma-test.json")),
            format!(r#"{{"format":1,"pid":9,"startedAt":5,"prefix":"{prefix}"}}"#),
        )
        .unwrap();
        data
    }

    #[test]
    fn recover_sweeps_the_journal_folder_and_skips_an_hdd_last_folder() {
        for (traits, last_swept) in [(Some(hdd()), false), (None, true)] {
            let machine = FakeMachine {
                traits,
                ..disk_machine()
            };
            let rig = rig_on("recoversweep", scripted(Script::default()), machine);
            let journal_dir = rig.disk_dir().join("journal");
            let last_dir = rig.disk_dir().join("last");
            std::fs::create_dir_all(&journal_dir).unwrap();
            std::fs::create_dir_all(&last_dir).unwrap();
            let (in_journal, in_last) = (plant_orphan(&journal_dir), plant_orphan(&last_dir));
            let id = "00000000-0000-4000-8000-000000000001";
            rig.runner
                .store()
                .write_journal(&oma_core::load::Journal {
                    format: 1,
                    session_id: id.into(),
                    plan_summary: "disk".into(),
                    phase_index: 0,
                    kernel: None,
                    core: None,
                    updated_at: "2026-10-08T10:00:00Z".into(),
                    clean_end: false,
                    disk_folder: Some(journal_dir.to_string_lossy().into_owned()),
                })
                .unwrap();
            let last = last_dir.to_string_lossy().into_owned();
            rig.runner
                .deps
                .settings
                .update_with(|s| s.performance.disk_folder = Some(last));
            rig.runner.recover_now();
            assert!(!in_journal.exists(), "the journal folder is always swept");
            assert_eq!(!in_last.exists(), last_swept, "{traits:?}");
            // The journal became a disk session.
            let s = rig.runner.store().load(id).unwrap().unwrap();
            assert_eq!(s.component, Component::Disk);
        }
    }

    #[test]
    fn disk_start_sweeps_the_target_folder() {
        let rig = rig_on("disksweep", scripted(passed_script()), disk_machine());
        let folder = rig.disk_dir();
        let orphan = plant_orphan(&folder);
        rig.runner.start(disk_request(&folder, false)).unwrap();
        assert!(!orphan.exists());
        wait_idle(&rig.runner);
    }

    #[test]
    fn preview_of_a_disk_request_never_probes() {
        let machine = disk_machine();
        let probes = Arc::clone(&machine.probes);
        let rig = rig_on("diskpreview", scripted(Script::default()), machine);
        let folder = rig.disk_dir();
        let orphan = plant_orphan(&folder);
        let plan = rig.runner.preview(&disk_request(&folder, false)).unwrap();
        assert_eq!(plan.disk.unwrap().dir, folder.to_string_lossy());
        assert!(probes.lock().unwrap().is_empty(), "the probe writes a file");
        assert!(orphan.exists(), "a preview sweeps nothing");
        assert_eq!(
            rig.runner.deps.settings.snapshot().performance.disk_folder,
            None
        );
    }

    #[test]
    fn a_linked_folder_is_refused() {
        let machine = FakeMachine {
            linked: true,
            ..disk_machine()
        };
        let probes = Arc::clone(&machine.probes);
        let rig = rig_on("disklink", scripted(passed_script()), machine);
        let folder = rig.disk_dir();
        let text = folder.to_string_lossy().into_owned();
        let orphan = plant_orphan(&folder);
        let wire = |r: Result<(), StartError>| r.unwrap_err().wire();
        assert_eq!(wire(rig.runner.disk_probe(&text).map(drop)), "disk:link");
        assert_eq!(
            wire(rig.runner.preview(&disk_request(&folder, false)).map(drop)),
            "disk:link"
        );
        assert_eq!(
            wire(rig.runner.start(disk_request(&folder, false)).map(drop)),
            "disk:link"
        );
        let bench = super::super::bench::DiskBenchRequest {
            folder: text.clone(),
            profile: oma_core::scores::DiskProfile::B1,
            compressible: false,
            wake: false,
        };
        assert_eq!(
            wire(rig.runner.start_disk_bench(bench).map(drop)),
            "disk:link"
        );
        assert!(probes.lock().unwrap().is_empty());
        assert!(orphan.exists(), "nothing behind a link is swept");
        // Nor at startup, from the journal or the last folder.
        rig.runner
            .store()
            .write_journal(&oma_core::load::Journal {
                format: 1,
                session_id: "00000000-0000-4000-8000-000000000002".into(),
                plan_summary: "disk".into(),
                phase_index: 0,
                kernel: None,
                core: None,
                updated_at: "2026-10-08T10:00:00Z".into(),
                clean_end: false,
                disk_folder: Some(text.clone()),
            })
            .unwrap();
        rig.runner.remember_disk_folder(&text);
        rig.runner.recover_now();
        assert!(orphan.exists());
    }
}
