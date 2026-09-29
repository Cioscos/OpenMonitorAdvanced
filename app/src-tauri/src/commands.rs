//! Tauri commands called by the UI (see app/src/lib/backend/tauri.ts).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use oma_core::engine::Engine;
use oma_core::history::{History, HistoryWindow};
use oma_core::model::Schema;
use oma_core::sampler::{unix_ms, IntervalHandle};
use oma_core::settings::VendorLibraries;
use oma_core::stats::SensorStats;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::service::ServiceShell;
use crate::settings::{Effect, EffectStatus, SettingsStore};
use crate::window::NavState;
use crate::AppState;

#[cfg(not(windows))]
pub use no_gpu_processes::{GpuProcess, GpuProcessTable};
#[cfg(not(windows))]
pub use no_vendor_libraries::{Vendor, VendorMask, VendorSwitch};
#[cfg(windows)]
pub use oma_win::gpu::{GpuProcess, GpuProcessTable, Vendor, VendorMask, VendorSwitch};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySeed {
    revision: u64,
    seq: u64,
    #[serde(flatten)]
    history: HistoryWindow,
}

/// Longest history the UI may request: the whole buffer (1 h).
const MAX_HISTORY_SECONDS: u64 = 3_600;

pub(crate) fn history_since(now_ms: u64, seconds: u64) -> u64 {
    now_ms.saturating_sub(seconds.min(MAX_HISTORY_SECONDS) * 1_000)
}

/// Bounds of `maxPoints`: the envelope needs two rows per bucket, and more
/// rows than the 1 h buffer holds at 1 s would be the raw window anyway.
const MIN_POINTS: u32 = 2;
const MAX_POINTS: u32 = 3_600;

/// Raw window without `max_points`, min/max envelope with it (decision D3).
pub(crate) fn history_window(
    history: &History,
    ids: &[String],
    since_ms: u64,
    max_points: Option<u32>,
) -> HistoryWindow {
    match max_points {
        None => history.window(ids, since_ms),
        Some(n) => {
            history.window_decimated(ids, since_ms, n.clamp(MIN_POINTS, MAX_POINTS) as usize)
        }
    }
}

/// Min/max/average since start (or the last reset) for the requested ids, in
/// the same order; `null` for unknown ids and sensors without valid samples.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsReply {
    revision: u64,
    stats: Vec<Option<SensorStats>>,
}

pub(crate) fn stats_reply(engine: &Engine, ids: &[String]) -> StatsReply {
    StatsReply {
        revision: engine.schema().revision,
        stats: engine.stats().get(ids),
    }
}

/// Monitoring session facts the UI cannot know: the WebView is recreated on
/// every window open, the sampler has been running since the app started.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Unix time of the first sample; `null` before the first tick.
    pub started_at_ms: Option<u64>,
    pub interval_ms: u64,
}

/// `interval` is the live handle, so the answer follows a change of the setting.
pub(crate) fn session(engine: &Engine, interval: &IntervalHandle) -> Session {
    Session {
        started_at_ms: engine.started_at_ms(),
        interval_ms: interval.get().as_millis() as u64,
    }
}

// Run off the main thread: the sampler holds the engine lock for up to
// ~200 ms per tick, and a sync command would block window/tray event handling
// for that long.
#[tauri::command(async)]
pub fn get_schema(state: State<'_, AppState>) -> Schema {
    state
        .engine
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .schema()
        .clone()
}

/// `maxPoints` is optional in JS: callers that omit it get the raw window.
#[tauri::command(async)]
pub fn get_history(
    state: State<'_, AppState>,
    ids: Vec<String>,
    seconds: u64,
    max_points: Option<u32>,
) -> HistorySeed {
    let since = history_since(unix_ms(), seconds);
    let engine = state.engine.lock().unwrap_or_else(PoisonError::into_inner);
    HistorySeed {
        revision: engine.schema().revision,
        seq: engine.sequence(),
        history: history_window(engine.history(), &ids, since, max_points),
    }
}

#[tauri::command(async)]
pub fn get_stats(state: State<'_, AppState>, ids: Vec<String>) -> StatsReply {
    let engine = state.engine.lock().unwrap_or_else(PoisonError::into_inner);
    stats_reply(&engine, &ids)
}

/// Restarts min/max/average of the given sensors (the page's reset button).
#[tauri::command(async)]
pub fn reset_stats(state: State<'_, AppState>, ids: Vec<String>) {
    state
        .engine
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .stats_mut()
        .reset(&ids);
}

#[tauri::command(async)]
pub fn get_session(state: State<'_, AppState>) -> Session {
    let engine = state.engine.lock().unwrap_or_else(PoisonError::into_inner);
    session(&engine, &state.interval)
}

/// Why vendor libraries were not loaded at startup (spec §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeModeReason {
    /// The user started the app with `--safe`.
    Flag,
    /// The previous run left a crash marker.
    Crash,
}

/// Safe-mode status shown by the UI (`StartupStatus` in app/src/lib/types.ts).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupStatus {
    /// True while the GPU vendor libraries are switched off.
    pub safe_mode: bool,
    /// Why this session started in safe mode; `None` for a normal start.
    pub reason: Option<SafeModeReason>,
    /// File name of the module that crashed the previous run, e.g. "nvml.dll".
    pub crash_module: Option<String>,
}

impl StartupStatus {
    /// Safe mode when the user passed `--safe` or the previous run left a crash
    /// marker. A crash wins over the flag: it is the more useful explanation.
    pub fn at_startup(safe_flag: bool, crash_marker: Option<&str>) -> Self {
        let reason = match (crash_marker, safe_flag) {
            (Some(_), _) => Some(SafeModeReason::Crash),
            (None, true) => Some(SafeModeReason::Flag),
            (None, false) => None,
        };
        Self {
            safe_mode: reason.is_some(),
            reason,
            crash_module: crash_marker.and_then(crash_module),
        }
    }
}

/// File name of the faulting module in a crash marker written by
/// `oma_win::crash` (`code=0xc0000005 module=C:\...\nvml.dll`).
pub(crate) fn crash_module(marker: &str) -> Option<String> {
    let (_, path) = marker.split_once("module=")?;
    let name = path.trim().rsplit(['\\', '/']).next()?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// The vendor-library switch shared with the GPU provider, plus the reason
/// the session started in safe mode.
pub struct StartupState {
    switch: VendorSwitch,
    status: StartupStatus,
}

impl StartupState {
    pub fn new(switch: VendorSwitch, status: StartupStatus) -> Self {
        Self { switch, status }
    }

    /// The startup status with `safe_mode` read from the switch now.
    pub(crate) fn current(&self) -> StartupStatus {
        StartupStatus {
            safe_mode: !self.switch.enabled(),
            ..self.status.clone()
        }
    }

    /// Turns the vendor libraries on for this process (they stay on, D1).
    pub(crate) fn enable_vendor_libraries(&self) -> StartupStatus {
        self.switch.enable();
        self.current()
    }
}

/// The view a tray item asked for before the window existed: `"simple"`,
/// `"advanced"` or `null`. Returned once (the request is consumed).
#[tauri::command(async)]
pub fn take_pending_view(nav: State<'_, NavState>) -> Option<&'static str> {
    nav.take().map(oma_core::settings::ViewKind::as_str)
}

#[tauri::command(async)]
pub fn get_startup_status(state: State<'_, StartupState>) -> StartupStatus {
    state.current()
}

/// Leaves safe mode without a restart: at its next poll the GPU provider sees
/// the switch, asks for a rediscover and loads the vendor libraries on its own
/// worker thread.
#[tauri::command(async)]
pub fn enable_vendor_libraries(state: State<'_, StartupState>) -> StartupStatus {
    tracing::info!("GPU vendor libraries re-enabled from the UI");
    state.enable_vendor_libraries()
}

/// The vendor libraries the settings leave switched on.
pub(crate) fn vendor_mask(libraries: &VendorLibraries) -> VendorMask {
    VendorMask::NONE
        .with(Vendor::Nvml, libraries.nvml)
        .with(Vendor::Nvapi, libraries.nvapi)
        .with(Vendor::Adl, libraries.adl)
        .with(Vendor::Igcl, libraries.igcl)
}

/// Keeps the GPU provider's per-library switches in step with
/// `sources.vendorLibraries`: every change of the set is stored in `switch`
/// (the provider notices it at its next poll and rediscovers) and reported as
/// applied. `applied` is the set `switch` was built with; the store is checked
/// once right after subscribing, so a change made in between is not lost.
///
/// The listener may run on the settings writer thread or on a command thread:
/// it compares masks under a private lock (which also orders it against the
/// catch-up below) and, on a change, does one atomic store and a status update
/// (queued by the store when called from a listener); it never waits for the
/// store or the provider. Libraries already
/// loaded are never unloaded (D1); this only decides which ones the next
/// discovery uses.
pub(crate) fn follow_vendor_libraries(
    store: &Arc<SettingsStore>,
    switch: VendorSwitch,
    applied: VendorMask,
) {
    let last = Mutex::new(applied);
    let apply = {
        let store = Arc::clone(store);
        move |libraries: &VendorLibraries| {
            let mask = vendor_mask(libraries);
            {
                let mut last = last.lock().unwrap_or_else(PoisonError::into_inner);
                if *last == mask {
                    return;
                }
                *last = mask;
                switch.set_libraries(mask);
            }
            // Outside the lock: called from outside a delivery (the catch-up),
            // `set_effect` delivers to the listeners at once, this one included.
            store.set_effect(Effect::VendorLibraries, EffectStatus::Applied);
        }
    };
    let apply = Arc::new(apply);
    let listener = Arc::clone(&apply);
    store.subscribe(Box::new(move |settings, _| {
        listener(&settings.sources.vendor_libraries)
    }));
    apply(&store.settings().sources.vendor_libraries);
}

/// What the About page shows (`AppInfo` in app/src/lib/types.ts).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    /// From the last service `Hello`; `None` until a service has answered.
    pub service_version: Option<String>,
    pub protocol_version: u32,
    /// The folder that holds `settings.json`.
    pub settings_path: Option<String>,
    /// The folder of the diagnostic logs.
    pub logs_path: Option<String>,
}

/// The only places `open_known_path` opens: never a path the UI chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KnownPath {
    SettingsFolder,
    LogsFolder,
    ThirdPartyNotices,
    /// Windows Settings › Apps › Startup, where Windows keeps the real state
    /// of the start-up entry.
    StartupAppsSettings,
}

/// `THIRD_PARTY_NOTICES.md` as the bundle ships it (`bundle.resources` in
/// tauri.conf.json): renamed to `.txt`, which Windows always knows how to open.
const THIRD_PARTY_NOTICES: &str = "THIRD_PARTY_NOTICES.txt";
const STARTUP_APPS_SETTINGS: &str = "ms-settings:startupapps";

/// Where the known paths are on this machine.
pub(crate) struct KnownDirs {
    pub settings_file: Option<PathBuf>,
    pub logs: Option<PathBuf>,
    /// The folder of the bundled resources.
    pub resources: Option<PathBuf>,
}

impl KnownDirs {
    pub(crate) fn current(app: &AppHandle) -> Self {
        Self {
            settings_file: crate::settings::settings_path(),
            logs: crate::logs_dir(),
            resources: app.path().resource_dir().ok(),
        }
    }

    fn settings_folder(&self) -> Option<PathBuf> {
        self.settings_file
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    /// What `target` opens; `None` when this machine has no such place.
    pub(crate) fn target(&self, target: KnownPath) -> Option<PathBuf> {
        match target {
            KnownPath::SettingsFolder => self.settings_folder(),
            KnownPath::LogsFolder => self.logs.clone(),
            KnownPath::ThirdPartyNotices => self
                .resources
                .as_ref()
                .map(|dir| dir.join(THIRD_PARTY_NOTICES)),
            KnownPath::StartupAppsSettings => Some(PathBuf::from(STARTUP_APPS_SETTINGS)),
        }
    }
}

pub(crate) fn app_info(
    version: String,
    service_version: Option<String>,
    dirs: &KnownDirs,
) -> AppInfo {
    let text = |path: Option<PathBuf>| path.map(|p| p.display().to_string());
    AppInfo {
        version,
        service_version,
        protocol_version: oma_ipc::PROTOCOL_VERSION,
        settings_path: text(dirs.settings_folder()),
        logs_path: text(dirs.logs.clone()),
    }
}

#[tauri::command(async)]
pub fn get_app_info(app: AppHandle, service: State<'_, ServiceShell>) -> AppInfo {
    app_info(
        app.package_info().version.to_string(),
        service.service_version(),
        &KnownDirs::current(&app),
    )
}

/// Opens one of the fixed [`KnownPath`] targets with the shell. The error is
/// the system's text, shown next to the button.
#[tauri::command(async)]
pub fn open_known_path(app: AppHandle, target: KnownPath) -> Result<(), String> {
    let path = KnownDirs::current(&app)
        .target(target)
        .ok_or_else(|| "not available on this system".to_owned())?;
    shell_open(&path).map_err(|err| {
        tracing::warn!(?target, %err, "cannot open a known path");
        err.to_string()
    })
}

#[cfg(windows)]
fn shell_open(path: &Path) -> std::io::Result<()> {
    oma_win::shell_open::open(path)
}

#[cfg(not(windows))]
fn shell_open(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("not supported on this system"))
}

/// Per-process GPU usage, published by the GPU provider every tick (decision D5).
pub struct GpuProcessState(pub GpuProcessTable);

/// Processes using GPU `device_id` (JS argument `deviceId`): busiest first, at
/// most 20 rows; an unknown device gives an empty list.
#[tauri::command(async)]
pub fn get_gpu_processes(state: State<'_, GpuProcessState>, device_id: String) -> Vec<GpuProcess> {
    state.0.processes(&device_id)
}

/// Off Windows no provider publishes GPU processes: the table is always empty.
#[cfg(not(windows))]
mod no_gpu_processes {
    /// Never constructed off Windows; any serializable type fits the empty reply.
    pub type GpuProcess = serde_json::Value;

    #[derive(Clone, Default)]
    pub struct GpuProcessTable;

    impl GpuProcessTable {
        pub fn new() -> Self {
            Self
        }

        pub fn processes(&self, _device_id: &str) -> Vec<GpuProcess> {
            Vec::new()
        }
    }
}

/// Off Windows there are no GPU vendor libraries: the switch only keeps state.
#[cfg(not(windows))]
mod no_vendor_libraries {
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::Arc;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Vendor {
        Nvml,
        Nvapi,
        Adl,
        Igcl,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct VendorMask(u8);

    impl VendorMask {
        pub const NONE: VendorMask = VendorMask(0);
        pub const ALL: VendorMask = VendorMask(0b1111);

        pub fn contains(self, vendor: Vendor) -> bool {
            self.0 & (1 << vendor as u8) != 0
        }

        #[must_use]
        pub fn with(self, vendor: Vendor, on: bool) -> Self {
            let bit = 1 << vendor as u8;
            Self(if on { self.0 | bit } else { self.0 & !bit })
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct VendorSwitch(Arc<Switches>);

    #[derive(Debug, Default)]
    struct Switches {
        master: AtomicBool,
        libraries: AtomicU8,
    }

    impl VendorSwitch {
        pub fn new(master: bool, libraries: VendorMask) -> Self {
            Self(Arc::new(Switches {
                master: AtomicBool::new(master),
                libraries: AtomicU8::new(libraries.0),
            }))
        }

        pub fn enabled(&self) -> bool {
            self.0.master.load(Ordering::Relaxed)
        }

        pub fn enable(&self) {
            self.0.master.store(true, Ordering::Relaxed);
        }

        pub fn set_libraries(&self, libraries: VendorMask) {
            self.0.libraries.store(libraries.0, Ordering::Relaxed);
        }

        pub fn effective(&self) -> VendorMask {
            if self.enabled() {
                VendorMask(self.0.libraries.load(Ordering::Relaxed))
            } else {
                VendorMask::NONE
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
    use oma_core::provider::{Inventory, Provider, ProviderError};
    use oma_core::settings::VendorLibraries;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// One CPU load sensor whose readings come from a fixed list.
    struct Scripted(std::collections::VecDeque<f64>);

    impl Provider for Scripted {
        fn name(&self) -> &'static str {
            "scripted"
        }

        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            Ok(Inventory {
                devices: vec![Device {
                    id: "cpu/0".into(),
                    kind: DeviceKind::Cpu,
                    name: "cpu".into(),
                    vendor: None,
                    properties: Default::default(),
                }],
                sensors: vec![Sensor::new(
                    "cpu/0",
                    SensorKind::Load,
                    "total",
                    Unit::Percent,
                    Label::new("cpu.load.total"),
                    Source::Mock,
                )],
            })
        }

        fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
            Ok(vec![self.0.pop_front()])
        }
    }

    fn engine_after(values: &[f64]) -> Engine {
        let provider = Scripted(values.iter().copied().collect());
        let mut engine = Engine::new(vec![Box::new(provider)], 3_600);
        for (i, _) in values.iter().enumerate() {
            let t = 1_000 * (i as u64 + 1);
            engine.tick(t, t);
        }
        engine
    }

    const LOAD: &str = "cpu/0/load/total";

    #[test]
    fn history_without_max_points_is_the_raw_window() {
        let engine = engine_after(&[10.0, 20.0, 30.0, 40.0, 50.0]);
        let w = history_window(engine.history(), &ids(&[LOAD]), 0, None);
        assert_eq!(w.timestamps_ms.len(), 5);
    }

    #[test]
    fn history_with_max_points_is_decimated_and_clamped() {
        let engine = engine_after(&[10.0, 20.0, 30.0, 40.0, 50.0, 60.0]);
        let history = engine.history();
        let load = ids(&[LOAD]);
        let w = history_window(history, &load, 0, Some(4));
        assert_eq!(w.timestamps_ms, vec![1_000, 3_000, 4_000, 6_000]);
        assert_eq!(
            w.series[0],
            vec![Some(10.0), Some(30.0), Some(40.0), Some(60.0)]
        );
        // 0 and 1 are raised to 2 points: one bucket, minimum then maximum.
        for tiny in [0, 1] {
            let w = history_window(history, &load, 0, Some(tiny));
            assert_eq!(w.series[0], vec![Some(10.0), Some(60.0)]);
        }
        // Huge values are capped at 3600, which still fits all six samples.
        let w = history_window(history, &load, 0, Some(u32::MAX));
        assert_eq!(w, history.window(&load, 0));
    }

    #[test]
    fn stats_reply_reports_min_max_avg_in_request_order() {
        let engine = engine_after(&[10.0, 30.0, 20.0]);
        let reply = stats_reply(&engine, &ids(&["unknown", LOAD]));
        assert_eq!(
            serde_json::to_value(&reply).expect("serialize"),
            serde_json::json!({
                "revision": 1,
                "stats": [null, { "min": 10.0, "max": 30.0, "avg": 20.0, "count": 3 }]
            })
        );
    }

    #[test]
    fn reset_restarts_the_statistics() {
        let mut engine = engine_after(&[10.0, 30.0]);
        engine.stats_mut().reset(&ids(&[LOAD]));
        assert_eq!(stats_reply(&engine, &ids(&[LOAD])).stats, vec![None]);
    }

    #[test]
    fn session_reports_the_first_tick_and_the_interval() {
        let interval = IntervalHandle::new(Duration::from_millis(1_000));
        let engine = Engine::new(Vec::new(), 10);
        assert_eq!(
            serde_json::to_value(session(&engine, &interval)).expect("serialize"),
            serde_json::json!({ "startedAtMs": null, "intervalMs": 1000 })
        );
        let engine = engine_after(&[1.0, 2.0]);
        assert_eq!(
            serde_json::to_value(session(&engine, &interval)).expect("serialize"),
            serde_json::json!({ "startedAtMs": 1000, "intervalMs": 1000 })
        );
    }

    #[test]
    fn session_reports_the_current_interval() {
        let interval = IntervalHandle::new(Duration::from_millis(1_000));
        let engine = Engine::new(Vec::new(), 10);
        interval.set(Duration::from_millis(2_500));
        assert_eq!(session(&engine, &interval).interval_ms, 2_500);
    }

    #[test]
    fn history_window_is_capped_at_one_hour() {
        assert_eq!(history_since(10_000_000, 300), 10_000_000 - 300_000);
        assert_eq!(history_since(10_000_000, 999_999), 10_000_000 - 3_600_000);
    }

    #[test]
    fn history_window_never_underflows() {
        assert_eq!(history_since(1_000, 300), 0);
    }

    #[test]
    fn history_seed_serializes_with_the_ts_contract_keys() {
        let seed = HistorySeed {
            revision: 1,
            seq: 2,
            history: HistoryWindow {
                timestamps_ms: vec![1_000],
                series: vec![vec![Some(3.0)]],
            },
        };
        let value = serde_json::to_value(&seed).expect("serialize");
        let object = value.as_object().expect("object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["revision", "seq", "series", "timestampsMs"]);
    }

    const NVML_CRASH: &str = r"code=0xc0000005 module=C:\Windows\System32\DriverStore\FileRepository\nv_dispi.inf_amd64_1234\nvml.dll";

    #[test]
    fn startup_status_reports_crash_module() {
        let status = StartupStatus::at_startup(false, Some(NVML_CRASH));
        assert_eq!(
            status,
            StartupStatus {
                safe_mode: true,
                reason: Some(SafeModeReason::Crash),
                crash_module: Some("nvml.dll".to_owned()),
            }
        );
        assert_eq!(
            serde_json::to_value(&status).expect("serialize"),
            serde_json::json!({ "safeMode": true, "reason": "crash", "crashModule": "nvml.dll" })
        );
    }

    #[test]
    fn crash_wins_over_the_safe_flag() {
        let status = StartupStatus::at_startup(true, Some(NVML_CRASH));
        assert_eq!(status.reason, Some(SafeModeReason::Crash));
    }

    #[test]
    fn safe_flag_alone_reports_the_flag() {
        let status = StartupStatus::at_startup(true, None);
        assert_eq!(
            serde_json::to_value(&status).expect("serialize"),
            serde_json::json!({ "safeMode": true, "reason": "flag", "crashModule": null })
        );
    }

    #[test]
    fn normal_start_is_not_safe_mode() {
        let status = StartupStatus::at_startup(false, None);
        assert_eq!(
            serde_json::to_value(&status).expect("serialize"),
            serde_json::json!({ "safeMode": false, "reason": null, "crashModule": null })
        );
    }

    #[test]
    fn crash_module_is_the_file_name_of_the_module_path() {
        assert_eq!(
            crash_module(r"code=0xc0000005 module=C:\Program Files\Vendor\x.dll").as_deref(),
            Some("x.dll")
        );
        assert_eq!(crash_module("code=0xc0000005 module=").as_deref(), None);
        assert_eq!(crash_module("code=0xc0000005").as_deref(), None);
        assert_eq!(crash_module("").as_deref(), None);
    }

    #[test]
    fn enabling_vendor_libraries_leaves_safe_mode() {
        let switch = VendorSwitch::new(false, VendorMask::ALL);
        let state = StartupState::new(switch.clone(), StartupStatus::at_startup(true, None));
        assert!(state.current().safe_mode);

        let status = state.enable_vendor_libraries();
        assert!(switch.enabled());
        assert!(!status.safe_mode);
        assert_eq!(status.reason, Some(SafeModeReason::Flag));
        assert!(!state.current().safe_mode);
    }

    fn vendor_store() -> Arc<SettingsStore> {
        Arc::new(crate::settings::fake_fs::open_fast(
            &crate::settings::fake_fs::FakeFs::new(),
        ))
    }

    fn only(vendors: &[Vendor]) -> VendorMask {
        vendors
            .iter()
            .fold(VendorMask::NONE, |mask, v| mask.with(*v, true))
    }

    #[test]
    fn the_mask_follows_the_library_switches() {
        let mut libraries = VendorLibraries::default();
        assert_eq!(vendor_mask(&libraries), VendorMask::ALL);
        libraries.nvapi = false;
        libraries.igcl = false;
        assert_eq!(vendor_mask(&libraries), only(&[Vendor::Nvml, Vendor::Adl]));
    }

    #[test]
    fn a_library_switch_reaches_the_gpu_switch_and_reports_applied() {
        let store = vendor_store();
        let switch = VendorSwitch::new(true, VendorMask::ALL);
        follow_vendor_libraries(&store, switch.clone(), VendorMask::ALL);
        assert_eq!(
            store.state().apply_status.vendor_libraries,
            EffectStatus::Idle,
            "nothing changed yet"
        );

        store.update_with(|s| s.sources.vendor_libraries.nvml = false);
        assert_eq!(
            switch.effective(),
            only(&[Vendor::Nvapi, Vendor::Adl, Vendor::Igcl])
        );
        assert_eq!(
            store.state().apply_status.vendor_libraries,
            EffectStatus::Applied
        );
    }

    #[test]
    fn other_settings_leave_the_gpu_switch_alone() {
        let store = vendor_store();
        let switch = VendorSwitch::new(true, VendorMask::ALL);
        follow_vendor_libraries(&store, switch.clone(), VendorMask::ALL);
        store.update_with(|s| s.general.interval_ms = 2_000);
        store.update_with(|s| s.sources.anti_cheat = true);
        assert_eq!(switch.effective(), VendorMask::ALL);
        assert_eq!(
            store.state().apply_status.vendor_libraries,
            EffectStatus::Idle
        );
    }

    #[test]
    fn a_change_made_before_following_is_not_lost() {
        let store = vendor_store();
        store.update_with(|s| s.sources.vendor_libraries.adl = false);
        let switch = VendorSwitch::new(true, VendorMask::ALL);
        follow_vendor_libraries(&store, switch.clone(), VendorMask::ALL);
        assert_eq!(
            switch.effective(),
            only(&[Vendor::Nvml, Vendor::Nvapi, Vendor::Igcl])
        );
    }

    #[test]
    fn safe_mode_keeps_the_master_off_while_switches_change() {
        let store = vendor_store();
        let switch = VendorSwitch::new(false, VendorMask::ALL);
        follow_vendor_libraries(&store, switch.clone(), VendorMask::ALL);
        store.update_with(|s| s.sources.vendor_libraries.nvml = false);
        assert!(!switch.enabled());
        assert_eq!(switch.effective(), VendorMask::NONE);
        switch.enable();
        assert_eq!(
            switch.effective(),
            only(&[Vendor::Nvapi, Vendor::Adl, Vendor::Igcl])
        );
    }

    #[test]
    fn open_known_path_rejects_anything_but_the_four_targets() {
        for (json, target) in [
            ("\"settingsFolder\"", KnownPath::SettingsFolder),
            ("\"logsFolder\"", KnownPath::LogsFolder),
            ("\"thirdPartyNotices\"", KnownPath::ThirdPartyNotices),
            ("\"startupAppsSettings\"", KnownPath::StartupAppsSettings),
        ] {
            assert_eq!(serde_json::from_str::<KnownPath>(json).unwrap(), target);
        }
        for json in [
            r#""C:\\Windows\\System32\\cmd.exe""#,
            r#""ms-settings:startupapps""#,
            r#""SettingsFolder""#,
            r#""""#,
            r#"{"path":"C:\\"}"#,
            r#"{"settingsFolder":"C:\\"}"#,
            "0",
            "null",
        ] {
            assert!(
                serde_json::from_str::<KnownPath>(json).is_err(),
                "{json} must be rejected"
            );
        }
    }

    fn dirs() -> KnownDirs {
        KnownDirs {
            settings_file: Some(PathBuf::from(r"C:\Roaming\OMA\settings.json")),
            logs: Some(PathBuf::from(r"C:\Local\OMA\logs")),
            resources: Some(PathBuf::from(r"C:\Program Files\OMA")),
        }
    }

    #[test]
    fn known_paths_resolve_to_fixed_places() {
        let dirs = dirs();
        assert_eq!(
            dirs.target(KnownPath::SettingsFolder),
            Some(PathBuf::from(r"C:\Roaming\OMA"))
        );
        assert_eq!(
            dirs.target(KnownPath::LogsFolder),
            Some(PathBuf::from(r"C:\Local\OMA\logs"))
        );
        assert_eq!(
            dirs.target(KnownPath::ThirdPartyNotices),
            Some(PathBuf::from(
                r"C:\Program Files\OMA\THIRD_PARTY_NOTICES.txt"
            ))
        );
        assert_eq!(
            dirs.target(KnownPath::StartupAppsSettings),
            Some(PathBuf::from("ms-settings:startupapps"))
        );
        let none = KnownDirs {
            settings_file: None,
            logs: None,
            resources: None,
        };
        assert_eq!(none.target(KnownPath::SettingsFolder), None);
        assert_eq!(none.target(KnownPath::LogsFolder), None);
        assert_eq!(none.target(KnownPath::ThirdPartyNotices), None);
        assert!(none.target(KnownPath::StartupAppsSettings).is_some());
    }

    #[test]
    fn app_info_serializes_versions_and_folders() {
        let info = app_info("0.1.0".into(), Some("0.1.0-svc".into()), &dirs());
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::json!({
                "version": "0.1.0",
                "serviceVersion": "0.1.0-svc",
                "protocolVersion": oma_ipc::PROTOCOL_VERSION,
                "settingsPath": r"C:\Roaming\OMA",
                "logsPath": r"C:\Local\OMA\logs",
            })
        );
        let bare = app_info(
            "0.1.0".into(),
            None,
            &KnownDirs {
                settings_file: None,
                logs: None,
                resources: None,
            },
        );
        let value = serde_json::to_value(&bare).unwrap();
        assert_eq!(value["serviceVersion"], serde_json::Value::Null);
        assert_eq!(value["settingsPath"], serde_json::Value::Null);
        assert_eq!(value["logsPath"], serde_json::Value::Null);
    }
}
