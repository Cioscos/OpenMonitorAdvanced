//! Tauri commands called by the UI (see app/src/lib/backend/tauri.ts).

use std::sync::PoisonError;

use oma_core::engine::Engine;
use oma_core::history::{History, HistoryWindow};
use oma_core::model::Schema;
use oma_core::sampler::unix_ms;
use oma_core::stats::SensorStats;
use serde::Serialize;
use tauri::State;

use crate::AppState;

#[cfg(not(windows))]
pub use no_vendor_libraries::VendorSwitch;
#[cfg(windows)]
pub use oma_win::gpu::VendorSwitch;

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

pub(crate) fn session(engine: &Engine, interval_ms: u64) -> Session {
    Session {
        started_at_ms: engine.started_at_ms(),
        interval_ms,
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
    session(&engine, state.interval_ms)
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

/// Off Windows there are no GPU vendor libraries: the switch only keeps state.
#[cfg(not(windows))]
mod no_vendor_libraries {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[derive(Debug, Clone, Default)]
    pub struct VendorSwitch(Arc<AtomicBool>);

    impl VendorSwitch {
        pub fn new(enabled: bool) -> Self {
            Self(Arc::new(AtomicBool::new(enabled)))
        }

        pub fn enabled(&self) -> bool {
            self.0.load(Ordering::Relaxed)
        }

        pub fn enable(&self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
    use oma_core::provider::{Inventory, Provider, ProviderError};

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
        let engine = Engine::new(Vec::new(), 10);
        assert_eq!(
            serde_json::to_value(session(&engine, 1_000)).expect("serialize"),
            serde_json::json!({ "startedAtMs": null, "intervalMs": 1000 })
        );
        let engine = engine_after(&[1.0, 2.0]);
        assert_eq!(
            serde_json::to_value(session(&engine, 1_000)).expect("serialize"),
            serde_json::json!({ "startedAtMs": 1000, "intervalMs": 1000 })
        );
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
        let switch = VendorSwitch::new(false);
        let state = StartupState::new(switch.clone(), StartupStatus::at_startup(true, None));
        assert!(state.current().safe_mode);

        let status = state.enable_vendor_libraries();
        assert!(switch.enabled());
        assert!(!status.safe_mode);
        assert_eq!(status.reason, Some(SafeModeReason::Flag));
        assert!(!state.current().safe_mode);
    }
}
