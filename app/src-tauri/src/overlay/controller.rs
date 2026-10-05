//! The overlay controller: from the settings, the service's frame data, the
//! foreground window and the overlay process's state it decides what the
//! frame engine does, which game is the target, where the overlay shows (if
//! at all), which profile it draws and which data it gets. Pure: no threads,
//! no Windows calls; the caller (the `oma-overlay-ctl` thread) feeds it the
//! inputs and a monotonic clock in milliseconds, and carries out the
//! [`Outputs`] of each [`Controller::step`].
//!
//! - **Engine:** `FramesConfigure` with `enabled = overlay.enabled ||
//!   OMA_FRAMES_DEBUG`, and tracking flags that OR the overlay's settings
//!   (while it is on) and the variable's (DP13); sent on change and again
//!   once a second, as M7b did, with the target.
//! - **Target:** a [`TargetPicker`] over the presenting processes and the
//!   foreground PID; a target that keeps presenting is replaced only by a
//!   game in the foreground on its own monitor. A new target activates `gameProfiles[exe]`, else
//!   `defaultProfile`, and ends a `next_profile` choice.
//!   Without frame data (service down or incompatible, engine starting or
//!   not running) the target is kept while its window exists, and a
//!   foreground process whose last known name has a `gameProfiles` entry can become one, so
//!   sensor blocks keep working (spec §9).
//! - **Visibility (DP9–DP11):** the overlay shows only while it is on, its
//!   process runs, the target's window is visible and not minimized and
//!   either is the foreground one or the foreground window is on another
//!   monitor, the game is not in `blockedGames` and the user has not
//!   hidden it. `SetPlacement` is sent only when it changes.
//! - **Data:** `SetProfile` on a change of profile, drawing settings or
//!   schema, and after every new `Running`; `FrameMetrics` at `textHz`,
//!   `FrameTimes` at 10 Hz with the new frames only, sensor values through
//!   [`ValuesPlan`]; no data while the overlay is hidden.
//! - **Editor and preview (M7d):** with the editor open the same data goes
//!   to the canvas ([`Outputs::editor_data`]) and, while it runs, to the
//!   preview process, with its own `SetProfile`. Their frames are the
//!   target's, or without a target the editor's [`SyntheticFeed`]; the
//!   in-game overlay never gets synthetic frames. The metrics are computed
//!   once per step; the overlay keeps its own cadence and frame position and
//!   gets only the active profile's lows, so the editor changes nothing in
//!   it.
//! - **Benchmark (M7d, §8):** [`Controller::toggle_benchmark`] starts a
//!   capture of the target (the overlay must be on, DD6) or stops it. The
//!   frames of the target's main swapchain go to a [`Recorder`] too (DD7);
//!   the files are the caller's ([`Outputs::benchmark`]). The overlay gets
//!   `● REC` once a second and the summary for [`SUMMARY_SHOW_MS`], never
//!   for a game in `blockedGames`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use oma_core::csv::LocalTime;
use oma_core::frames::metrics::LowDefinition;
use oma_core::frames::{pick_swapchain, read, FrameReadout, FrameWindow, LOWS_WINDOW_S};
use oma_core::model::{Schema, Snapshot};
use oma_core::overlay::{Foreground, Profile, PxRect, WindowGeometry};
use oma_core::provider::Quality;
use oma_core::settings::{Attach, Settings};
use oma_ipc::overlay::{
    BenchmarkOverlay, FrameMetrics, OverlayMessage, PxArea, SetPlacement, WireBenchmarkSummary,
    WireFrameTime,
};
use oma_ipc::{
    frames_state, FrameBatch, FramesConfigure, FramesStatus, PresentingProcess, PresentingProcesses,
};
use oma_win::svc::LinkCommand;
use serde::Serialize;

use super::benchmark::{file_stem, BenchmarkRecord, EndReason, Recorder, SUMMARY_SHOW_MS};
use super::editor_feed::{EditorData, SyntheticFeed};
use super::forward::{
    frame_times_since, keep_lows, low_windows, metrics_message, set_profile, union_needs,
    values_message,
};
use super::frames::{detail_label, line, sample_of, state_label, LineContext};
use super::host::{HostFailure, HostState};
use super::profiles::{ProfileCatalog, ProfileDiagnostic, ProfileEntry};
use super::target::{ProcessInfo, TargetPicker, MIN_GAME_FPS, OWN_PROCESS_NAMES, SYSTEM_EXCLUDED};
use crate::i18n::Lang;
use crate::log::session::LogError;
use crate::log::writer::WriteFailure;
use crate::log::HotkeyStatus;

/// How often the engine configuration and the target are sent again.
const RESEND_MS: u64 = 1_000;
/// How often the `frames:` line is written.
const LINE_MS: u64 = 1_000;
/// `FrameTimes` period (10 Hz).
const FRAME_TIMES_MS: u64 = 100;
/// Longest lows window a profile may ask for (DP12).
const MAX_LOWS_WINDOW_S: f64 = 300.0;
/// `present_mode` values of a game in exclusive fullscreen (DP16), where the
/// overlay cannot show.
const EXCLUSIVE_PRESENT_MODES: [&str; 2] = [
    "Hardware: Legacy Flip",
    "Hardware: Legacy Copy to front buffer",
];
/// The DPI of a hidden placement, which the overlay ignores.
const DEFAULT_DPI: u32 = 96;
/// The profile id of the preview's `SetProfile`.
const PREVIEW_ID: &str = "preview";
/// `FrameMetrics.state` and `OverlayStatus.frames` without the service.
pub const FRAMES_UNAVAILABLE: &str = "unavailable";

/// A notification the app should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastRequest {
    /// The target runs in exclusive fullscreen, where the overlay cannot
    /// show: once per executable and per app session (DP16).
    ExclusiveFullscreen { exe: String },
    /// A capture was asked for without a game to measure.
    BenchmarkNoTarget,
    /// The capture stopped on a write error.
    BenchmarkError { error: LogError },
}

/// What the caller does with the capture's files, in order.
// One `Finish` per capture: its size does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum BenchmarkCommand {
    /// Create the CSV.
    Begin { stem: String },
    /// Append these rows.
    Rows(Vec<String>),
    /// Close the CSV, with its summary (`None`: no frames, remove it).
    Finish { record: Option<BenchmarkRecord> },
}

/// The capture, for the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkStatus {
    /// `idle`, `recording` or `error`.
    pub state: String,
    pub game: Option<String>,
    pub elapsed_s: Option<u32>,
    pub error: Option<LogError>,
}

/// What the sampler needs to send sensor values, published by the
/// controller's thread (ruling R2): the profile's sensors and whether the
/// overlay shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValuesPlan {
    pub used: Vec<String>,
    pub wanted: bool,
}

/// `Values` for one sampler tick under `plan`: `None` while the overlay is
/// hidden or its profile reads no sensor.
pub fn tick_values(
    plan: &ValuesPlan,
    schema: &Schema,
    snapshot: &Snapshot,
    quality: &[Quality],
    at_ms: u64,
) -> Option<OverlayMessage> {
    (plan.wanted && !plan.used.is_empty())
        .then(|| values_message(schema, snapshot, quality, &plan.used, at_ms))
}

/// The followed game, for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetStatus {
    pub name: String,
    pub pid: u32,
}

/// The overlay's hotkeys, for the UI (filled by the hotkey manager).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayHotkeys {
    pub toggle: HotkeyStatus,
    pub next_profile: HotkeyStatus,
    pub benchmark: HotkeyStatus,
}

/// The overlay's state, for the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayStatus {
    pub enabled: bool,
    /// `off`, `starting`, `running` or `failed`.
    pub process: String,
    /// With `failed`: `crashing` or `incompatible`.
    pub process_reason: Option<String>,
    /// A `frames_state` value, or [`FRAMES_UNAVAILABLE`].
    pub frames: String,
    pub frames_detail: Option<String>,
    pub target: Option<TargetStatus>,
    pub active_profile: Option<String>,
    pub profiles: Vec<ProfileEntry>,
    pub diagnostics: Vec<ProfileDiagnostic>,
    pub hidden_by_user: bool,
    pub hotkeys: OverlayHotkeys,
    /// The preview is open.
    pub preview: bool,
    pub benchmark: BenchmarkStatus,
}

/// What one [`Controller::step`] asks the caller to do.
#[derive(Debug, Default)]
pub struct Outputs {
    /// Commands for the service link, in order.
    pub link: Vec<LinkCommand>,
    /// Messages for the overlay, in order (only while it is `Running`).
    pub overlay: Vec<OverlayMessage>,
    /// Whether the overlay process should run.
    pub want_process: bool,
    /// Whether the preview process should run: there is a preview profile.
    pub want_preview: bool,
    /// Messages for the preview process, in order (only while it is
    /// `Running`): its `SetProfile` and the overlay's data.
    pub preview: Vec<OverlayMessage>,
    /// For the editor's canvas, only while it is open.
    pub editor_data: Option<EditorData>,
    /// Watch this window's geometry (`Some(None)`: stop watching).
    pub track: Option<Option<Foreground>>,
    pub toast: Option<ToastRequest>,
    /// The capture's file work, in order.
    pub benchmark: Vec<BenchmarkCommand>,
    /// The new status, only when it changed.
    pub status: Option<OverlayStatus>,
    /// A `frames:` line to log (`OMA_FRAMES_DEBUG` only).
    pub diagnostics_line: Option<String>,
    /// «Retry» on a failed overlay process: call `OverlayHost::retry`.
    pub retry_host: bool,
    /// The new plan for the sampler, only when it changed.
    pub values_plan: Option<ValuesPlan>,
    /// The target's present mode, when it changed (for the log).
    pub present_mode: Option<String>,
    /// The foreground window as the service sees it, when the window or
    /// its standing (listed, game rate) changed (for the log).
    pub foreground_note: Option<ForegroundNote>,
}

/// What the service says about the foreground process.
#[derive(Debug, Clone, PartialEq)]
pub struct ForegroundNote {
    pub pid: u32,
    /// From the last process list, while known.
    pub name: Option<String>,
    /// Displayed FPS and present mode, `None` when it is not presenting.
    pub presenting: Option<(f64, String)>,
}

pub struct Controller {
    own_pid: u32,
    qpc_frequency: u64,

    // Inputs.
    settings: Settings,
    lang: Lang,
    env: Option<FramesConfigure>,
    connected: bool,
    status: Option<FramesStatus>,
    processes: Vec<PresentingProcess>,
    /// Received since the last step.
    batches: Vec<FrameBatch>,
    foreground: Option<Foreground>,
    /// The monitor of the foreground window, when known.
    foreground_monitor: Option<PxRect>,
    geometry: Option<WindowGeometry>,
    /// A geometry arrived for the tracked window: a `None` after it means
    /// the window is gone.
    geometry_seen: bool,
    /// Names of the processes of the last list received, kept without the
    /// service.
    known_names: BTreeMap<u32, String>,
    /// The link came back since the last step.
    reconnected: bool,
    catalog: ProfileCatalog,
    schema: Arc<Schema>,
    host: HostState,
    hidden_by_user: bool,
    hotkeys: OverlayHotkeys,

    // Target and frames.
    picker: TargetPicker,
    target: Option<ProcessInfo>,
    /// The target's window: the foreground window when it became the target
    /// or last came to the foreground.
    tracked: Option<Foreground>,
    window: FrameWindow,
    window_s: f64,
    /// The main swapchain, from the last readout.
    swapchain: Option<u64>,
    dropped: u64,

    // Profile.
    /// The `next_profile` choice, until the target changes.
    choice: Option<String>,
    /// The active profile must be resolved again.
    choice_dirty: bool,
    /// The resolved id and profile.
    active: Option<(String, Profile)>,
    lows: Vec<(u32, LowDefinition)>,
    used: Vec<String>,
    /// `SetProfile` may differ from the one sent.
    profile_dirty: bool,

    // Editor and preview.
    /// The editor's synthetic frames: `Some` exactly while it is open.
    editor: Option<SyntheticFeed>,
    preview: Option<Profile>,
    preview_host: HostState,
    preview_was_running: bool,
    /// The preview's `SetProfile` may differ from the one sent.
    preview_dirty: bool,
    sent_preview: Option<OverlayMessage>,
    /// `want_preview` of the last step.
    sent_want_preview: bool,
    /// The profile being edited: its lows reach the canvas.
    editor_profile: Option<Profile>,
    /// The metrics last computed while the editor is open.
    editor_metrics: Option<FrameMetrics>,
    /// The lows of the active profile, the only ones the overlay gets.
    overlay_lows: Vec<(u32, LowDefinition)>,

    // Benchmark.
    recorder: Recorder,
    /// The recorded game's PID and start time.
    bench_game: Option<(u32, LocalTime)>,
    bench_error: Option<LogError>,
    /// Queued between steps.
    bench_commands: Vec<BenchmarkCommand>,
    bench_toast: Option<ToastRequest>,
    /// The summary box and when it goes.
    bench_summary: Option<(WireBenchmarkSummary, u64)>,
    sent_bench: BenchmarkOverlay,
    /// The local wall clock, for the file name and the summary.
    clock: fn() -> LocalTime,
    /// The time of the last step.
    now_ms: u64,

    // What was sent.
    sent_config: Option<FramesConfigure>,
    config_sent_ms: u64,
    retry_frames: bool,
    retry_host: bool,
    sent_track: Option<Foreground>,
    was_running: bool,
    /// Since the overlay last became `Running`.
    sent_profile: Option<OverlayMessage>,
    /// `Some(None)`: hidden; `None`: nothing sent since `Running`.
    sent_placement: Option<Option<(PxArea, u32)>>,
    shown: bool,
    /// The in-game overlay's data.
    overlay_cursor: DataCursor,
    /// The editor's and the preview's data.
    side_cursor: DataCursor,
    line_ms: u64,
    /// Lowercase executables already toasted.
    toasted: BTreeSet<String>,
    sent_status: Option<OverlayStatus>,
    sent_plan: Option<ValuesPlan>,
    /// The target's present mode last reported.
    sent_present_mode: Option<String>,
    /// Window, listed, at a game's rate: of the last foreground note (the
    /// target's mode has its own line).
    sent_foreground: Option<(Foreground, bool, bool)>,
}

impl Controller {
    pub fn new(own_pid: u32, qpc_frequency: u64) -> Self {
        Self {
            own_pid,
            qpc_frequency,
            settings: Settings::default(),
            lang: Lang::En,
            env: None,
            connected: false,
            status: None,
            processes: Vec::new(),
            batches: Vec::new(),
            foreground: None,
            foreground_monitor: None,
            geometry: None,
            geometry_seen: false,
            known_names: BTreeMap::new(),
            reconnected: false,
            catalog: ProfileCatalog::default(),
            schema: Arc::new(Schema::default()),
            host: HostState::Off,
            hidden_by_user: false,
            hotkeys: OverlayHotkeys::default(),
            picker: new_picker(own_pid),
            target: None,
            tracked: None,
            window: FrameWindow::new(LOWS_WINDOW_S),
            window_s: LOWS_WINDOW_S,
            swapchain: None,
            dropped: 0,
            choice: None,
            choice_dirty: true,
            active: None,
            lows: Vec::new(),
            used: Vec::new(),
            profile_dirty: true,
            editor: None,
            preview: None,
            preview_host: HostState::Off,
            preview_was_running: false,
            preview_dirty: true,
            sent_preview: None,
            sent_want_preview: false,
            editor_profile: None,
            editor_metrics: None,
            overlay_lows: Vec::new(),
            recorder: Recorder::new(),
            bench_game: None,
            bench_error: None,
            bench_commands: Vec::new(),
            bench_toast: None,
            bench_summary: None,
            sent_bench: NO_BENCHMARK,
            clock: wall_clock,
            now_ms: 0,
            sent_config: None,
            config_sent_ms: 0,
            retry_frames: false,
            retry_host: false,
            sent_track: None,
            was_running: false,
            sent_profile: None,
            sent_placement: None,
            shown: false,
            overlay_cursor: DataCursor::NEW,
            side_cursor: DataCursor::NEW,
            line_ms: 0,
            toasted: BTreeSet::new(),
            sent_status: None,
            sent_plan: None,
            sent_present_mode: None,
            sent_foreground: None,
        }
    }

    /// The settings, the UI language (for `SetProfile`) and the
    /// configuration `OMA_FRAMES_DEBUG` asks for.
    pub fn on_settings(&mut self, settings: &Settings, lang: Lang, env: Option<FramesConfigure>) {
        if *settings != self.settings || lang != self.lang {
            // Turned on: the overlay starts visible (DP11).
            if settings.overlay.enabled && !self.settings.overlay.enabled {
                self.hidden_by_user = false;
            }
            // A new default or game association shows at once: it ends the
            // `next_profile` choice.
            let (old, new) = (&self.settings.overlay, &settings.overlay);
            if old.default_profile != new.default_profile || old.game_profiles != new.game_profiles
            {
                self.choice = None;
            }
            self.settings = settings.clone();
            self.lang = lang;
            self.choice_dirty = true;
            self.profile_dirty = true;
            self.preview_dirty = true;
        }
        self.env = env;
    }

    /// The frame feed's state, as `FramesFeed::drain` gives it: the latest
    /// status and process list (kept until a disconnection) and the batches
    /// received since the previous drain.
    pub fn on_frames(
        &mut self,
        status: Option<&FramesStatus>,
        processes: Option<&PresentingProcesses>,
        batches: &[FrameBatch],
    ) {
        self.status = status.cloned();
        self.processes = processes.map(|p| p.processes.clone()).unwrap_or_default();
        if processes.is_some() {
            self.known_names = self
                .processes
                .iter()
                .map(|p| (p.pid, p.name.clone()))
                .collect();
        }
        self.batches.extend_from_slice(batches);
    }

    /// Whether the service link is up.
    pub fn on_service(&mut self, connected: bool) {
        self.reconnected |= connected && !self.connected;
        self.connected = connected;
        if !connected {
            self.status = None;
            self.processes.clear();
        }
    }

    /// A new foreground window; its monitor follows with
    /// [`Self::on_foreground_monitor`], unknown until then.
    pub fn on_foreground(&mut self, fg: Foreground) {
        self.foreground = Some(fg);
        self.foreground_monitor = None;
    }

    /// The monitor of the foreground window (`None`: unknown).
    pub fn on_foreground_monitor(&mut self, monitor: Option<PxRect>) {
        self.foreground_monitor = monitor;
    }

    /// The tracked window's geometry (`None`: unknown or gone).
    pub fn on_geometry(&mut self, geometry: Option<WindowGeometry>) {
        self.geometry_seen |= geometry.is_some();
        self.geometry = geometry;
    }

    pub fn on_catalog(&mut self, catalog: ProfileCatalog) {
        self.catalog = catalog;
        self.choice_dirty = true;
    }

    /// A new schema: the built-in profiles bind to its sensors again.
    pub fn on_schema(&mut self, schema: Arc<Schema>) {
        self.schema = schema;
        self.choice_dirty = true;
        self.profile_dirty = true;
        self.preview_dirty = true;
    }

    pub fn on_host(&mut self, state: HostState) {
        self.host = state;
    }

    /// The «show/hide» hotkey or tray item; not saved (DP11). Nothing
    /// while the overlay is off.
    pub fn toggle_hidden(&mut self) {
        if !self.settings.overlay.enabled {
            return;
        }
        self.hidden_by_user = !self.hidden_by_user;
    }

    /// The statuses of the overlay's hotkeys, from the hotkey manager.
    pub fn on_hotkeys(&mut self, hotkeys: OverlayHotkeys) {
        self.hotkeys = hotkeys;
    }

    /// The profile after the active one, until the target changes.
    pub fn next_profile(&mut self) {
        let current = self
            .active
            .as_ref()
            .map_or_else(|| self.chosen_profile(), |(id, _)| id.clone());
        let next = self.catalog.next_after(&current);
        self.use_now(next);
    }

    /// «Use now» in the editor: the profile `id` until the target changes
    /// (DD9).
    pub fn use_now(&mut self, id: String) {
        self.choice = Some(id);
        self.choice_dirty = true;
    }

    /// The editor window opened or closed. Closing it ends the preview.
    pub fn on_editor(&mut self, open: bool) {
        if open == self.editor.is_some() {
            return;
        }
        if open {
            self.editor = Some(SyntheticFeed::new());
            // The canvas gets its data at once.
            self.side_cursor = DataCursor::NEW;
        } else {
            self.editor = None;
            self.editor_metrics = None;
            self.editor_profile = None;
            self.set_preview(None);
        }
    }

    /// The profile the preview draws; `None` closes the preview.
    pub fn set_preview(&mut self, profile: Option<Profile>) {
        self.preview = profile;
        self.preview_dirty = true;
        self.refresh_needs();
    }

    /// The profile open in the editor: the canvas gets its lows windows.
    pub fn set_editor_profile(&mut self, profile: Option<Profile>) {
        self.editor_profile = profile;
        self.refresh_needs();
    }

    /// The preview process's state. Going off by itself (it was wanted at
    /// the last step and had started) means the user closed its window: the
    /// preview ends rather than restarting. A stop we asked for keeps a
    /// preview set since.
    pub fn on_preview_host(&mut self, state: HostState) {
        let started = matches!(self.preview_host, HostState::Starting | HostState::Running);
        if state == HostState::Off && started && self.sent_want_preview && self.preview.is_some() {
            self.set_preview(None);
        }
        self.preview_host = state;
    }

    /// The benchmark hotkey or button: starts a capture of the target, which
    /// needs the overlay on (DD6), or stops the one running.
    pub fn toggle_benchmark(&mut self, now_ms: u64) {
        if self.recorder.is_recording() {
            self.end_benchmark(EndReason::User, now_ms);
            return;
        }
        let target = self
            .target
            .clone()
            .filter(|_| self.settings.overlay.enabled);
        let Some(target) = target else {
            self.bench_toast = Some(ToastRequest::BenchmarkNoTarget);
            return;
        };
        self.recorder.on_target(true, now_ms);
        if self.recorder.start(&target.name, now_ms).is_err() {
            return;
        }
        let start = (self.clock)();
        self.bench_game = Some((target.pid, start));
        self.bench_error = None;
        self.bench_summary = None;
        self.bench_commands.push(BenchmarkCommand::Begin {
            stem: file_stem(&target.name, start),
        });
    }

    /// A capture is running.
    pub fn recording(&self) -> bool {
        self.recorder.is_recording()
    }

    /// The capture's files failed: it stops with `Error` and the reason
    /// stays in the status. A failure after the end (the summary) is
    /// reported only if the capture had none.
    pub fn on_benchmark_error(&mut self, failure: WriteFailure) {
        let recording = self.recorder.is_recording();
        if !recording && self.bench_error.is_some() {
            return;
        }
        if recording {
            self.end_benchmark(EndReason::Error, self.now_ms);
        }
        let error = LogError::from(&failure);
        self.bench_error = Some(error.clone());
        self.bench_toast = Some(ToastRequest::BenchmarkError { error });
    }

    /// The app exits: a running capture ends with `Shutdown`; returns the
    /// file work still due, to be done before the app goes.
    pub fn shutdown_benchmark(&mut self, now_ms: u64) -> Vec<BenchmarkCommand> {
        self.end_benchmark(EndReason::Shutdown, now_ms);
        std::mem::take(&mut self.bench_commands)
    }

    fn end_benchmark(&mut self, reason: EndReason, now_ms: u64) {
        let Some(game) = self.recorder.game().map(str::to_owned) else {
            return;
        };
        let summary = self.recorder.stop(reason);
        let start = self.bench_game.take().map_or_else(self.clock, |(_, t)| t);
        let record = summary.map(|s| BenchmarkRecord::new(&game, start, reason, s));
        if let Some(r) = record.as_ref().filter(|_| !self.blocked_exe(&game)) {
            let s = &r.summary;
            let summary = WireBenchmarkSummary {
                fps_displayed: s.fps_displayed,
                low_one_percent: s.lows_integral.one_percent,
                low_point_one_percent: s.lows_integral.point_one_percent,
                stutter_count: s.stutter_count,
                stutter_percent: s.stutter_percent,
            };
            self.bench_summary = Some((summary, now_ms.saturating_add(SUMMARY_SHOW_MS)));
        }
        self.bench_commands
            .push(BenchmarkCommand::Finish { record });
    }

    /// Whether the recorded game is the target; a capture that ran too long
    /// or without its game ends.
    fn step_benchmark(&mut self, now_ms: u64, out: &mut Outputs) {
        if let Some((pid, _)) = self.bench_game {
            let present = self.target.as_ref().is_some_and(|t| t.pid == pid);
            self.recorder.on_target(present, now_ms);
            if let Some(reason) = self.recorder.tick(now_ms) {
                self.end_benchmark(reason, now_ms);
            }
        }
        out.benchmark.append(&mut self.bench_commands);
    }

    /// The badge or the summary box, while the overlay runs.
    fn step_bench_overlay(&mut self, now_ms: u64, out: &mut Outputs) {
        if self
            .bench_summary
            .as_ref()
            .is_some_and(|&(_, until)| now_ms >= until)
        {
            self.bench_summary = None;
        }
        if self.host != HostState::Running {
            return;
        }
        let blocked = self.recorder.game().is_some_and(|g| self.blocked_exe(g));
        let wanted = if blocked {
            NO_BENCHMARK
        } else {
            BenchmarkOverlay {
                recording_s: self.recorder.elapsed_s(now_ms),
                summary: self.bench_summary.as_ref().map(|(s, _)| s.clone()),
            }
        };
        if wanted != self.sent_bench {
            self.sent_bench = wanted.clone();
            out.overlay.push(OverlayMessage::Benchmark(wanted));
        }
    }

    /// «Retry»: restarts a frame engine that failed or was denied, and an
    /// overlay process that failed.
    pub fn retry(&mut self) {
        let state = state_label(self.status.as_ref());
        if self.wanted_config().enabled
            && (state == frames_state::FAILED || state == frames_state::DENIED)
        {
            self.retry_frames = true;
        }
        if matches!(self.host, HostState::Failed { .. }) {
            self.retry_host = true;
        }
    }

    pub fn step(&mut self, now_ms: u64) -> Outputs {
        let mut out = Outputs {
            want_process: self.settings.overlay.enabled,
            want_preview: self.preview.is_some(),
            retry_host: std::mem::take(&mut self.retry_host),
            ..Outputs::default()
        };
        self.now_ms = now_ms;
        // Queued since the last step (a `Begin`), before this step's rows.
        out.benchmark.append(&mut self.bench_commands);
        let config = self.wanted_config();
        self.step_config(&config, now_ms, &mut out);
        self.step_target(config.enabled, now_ms, &mut out);
        self.step_benchmark(now_ms, &mut out);
        self.step_resend(&config, now_ms, &mut out);
        self.step_track(&mut out);
        self.refresh_profile();
        if self.target.is_none() {
            if let Some(feed) = &mut self.editor {
                feed.advance(now_ms as f64 / 1_000.0);
            }
        }
        self.step_overlay();
        self.step_overlay_messages(&mut out);
        self.step_bench_overlay(now_ms, &mut out);
        self.step_preview(&mut out);
        self.step_data(&config, now_ms, &mut out);
        out.toast = self.bench_toast.take().or_else(|| self.toast());
        let mode = self.target.as_ref().and_then(|t| {
            self.processes
                .iter()
                .find(|p| p.pid == t.pid)
                .map(|p| p.present_mode.clone())
        });
        if mode.is_some() && mode != self.sent_present_mode {
            self.sent_present_mode.clone_from(&mode);
            out.present_mode = mode;
        }
        out.foreground_note = self.foreground_note();

        let status = self.current_status();
        if self.sent_status.as_ref() != Some(&status) {
            self.sent_status = Some(status.clone());
            out.status = Some(status);
        }
        if self.env.is_some() && now_ms.saturating_sub(self.line_ms) >= LINE_MS {
            self.line_ms = now_ms;
            let readout = self.readout(config.track_gpu);
            out.diagnostics_line = Some(self.line(&readout));
        }
        self.sent_want_preview = out.want_preview;
        let plan = self.values_plan();
        if self.sent_plan.as_ref() != Some(&plan) {
            self.sent_plan = Some(plan.clone());
            out.values_plan = Some(plan);
        }
        out
    }

    /// The sampler's plan now (see [`tick_values`]).
    pub fn values_plan(&self) -> ValuesPlan {
        ValuesPlan {
            used: self.used.clone(),
            wanted: self.shown || self.preview_running(),
        }
    }

    fn preview_running(&self) -> bool {
        self.preview.is_some() && self.preview_host == HostState::Running
    }

    /// The editor's synthetic frames, used while there is no target.
    fn synthetic(&self) -> Option<&SyntheticFeed> {
        self.editor.as_ref().filter(|_| self.target.is_none())
    }

    /// The frames the overlay, the preview and the canvas show.
    fn frames_source(&self) -> &FrameWindow {
        self.synthetic().map_or(&self.window, SyntheticFeed::window)
    }

    fn reset_times(&mut self) {
        self.swapchain = None;
        self.overlay_cursor.after_s = f64::NEG_INFINITY;
        self.side_cursor.after_s = f64::NEG_INFINITY;
    }

    /// `Values` for one sampler tick, under the plan of the last step. The
    /// sampler itself uses [`tick_values`] over the published plan (R2).
    #[cfg(test)]
    pub fn on_tick_values(
        &self,
        schema: &Schema,
        snapshot: &Snapshot,
        quality: &[Quality],
        at_ms: u64,
    ) -> Option<OverlayMessage> {
        tick_values(&self.values_plan(), schema, snapshot, quality, at_ms)
    }

    pub fn current_status(&self) -> OverlayStatus {
        let (process, process_reason) = match self.host {
            HostState::Off => ("off", None),
            HostState::Starting => ("starting", None),
            HostState::Running => ("running", None),
            HostState::Failed { reason } => (
                "failed",
                Some(match reason {
                    HostFailure::Crashing => "crashing",
                    HostFailure::Incompatible => "incompatible",
                }),
            ),
        };
        OverlayStatus {
            enabled: self.settings.overlay.enabled,
            process: process.to_owned(),
            process_reason: process_reason.map(str::to_owned),
            frames: self.frames_state().to_owned(),
            frames_detail: self
                .status
                .as_ref()
                .filter(|_| self.wanted_config().enabled)
                .and_then(|s| s.detail.clone()),
            target: self.target.as_ref().map(|t| TargetStatus {
                name: t.name.clone(),
                pid: t.pid,
            }),
            active_profile: self.active.as_ref().map(|(id, _)| id.clone()),
            profiles: self.catalog.entries.clone(),
            diagnostics: self.catalog.diagnostics.clone(),
            hidden_by_user: self.hidden_by_user,
            hotkeys: self.hotkeys.clone(),
            preview: self.preview.is_some(),
            benchmark: BenchmarkStatus {
                state: if self.recorder.is_recording() {
                    "recording"
                } else if self.bench_error.is_some() {
                    "error"
                } else {
                    "idle"
                }
                .to_owned(),
                game: self.recorder.game().map(str::to_owned),
                elapsed_s: self.recorder.elapsed_s(self.now_ms),
                error: self.bench_error.clone(),
            },
        }
    }

    /// The engine configuration: the overlay's (while it is on) OR the
    /// variable's (DP13).
    fn wanted_config(&self) -> FramesConfigure {
        let overlay = &self.settings.overlay;
        let on = overlay.enabled;
        let env = self.env.as_ref();
        FramesConfigure {
            enabled: on || env.is_some(),
            track_pc_latency: (on && overlay.track_pc_latency)
                || env.is_some_and(|e| e.track_pc_latency),
            track_gpu: (on && overlay.track_gpu) || env.is_some_and(|e| e.track_gpu),
        }
    }

    /// `ConfigureFrames` on a change (never before the engine was first
    /// wanted) and the two steps of a «Retry».
    fn step_config(&mut self, config: &FramesConfigure, now_ms: u64, out: &mut Outputs) {
        if std::mem::take(&mut self.retry_frames) {
            let off = FramesConfigure {
                enabled: false,
                track_pc_latency: false,
                track_gpu: false,
            };
            out.link.push(LinkCommand::ConfigureFrames(off.clone()));
            self.sent_config = Some(off);
            self.config_sent_ms = now_ms;
            return;
        }
        if self.sent_config.as_ref() != Some(config)
            && (config.enabled || self.sent_config.is_some())
        {
            out.link.push(LinkCommand::ConfigureFrames(config.clone()));
            self.sent_config = Some(config.clone());
            self.config_sent_ms = now_ms;
        }
    }

    /// A command that met a full link queue is lost: while the engine is
    /// on, the configuration and the target go again once a second (the
    /// link drops unchanged values); an engine turned off is told again only
    /// after a reconnection.
    fn step_resend(&mut self, config: &FramesConfigure, now_ms: u64, out: &mut Outputs) {
        let reconnected = std::mem::take(&mut self.reconnected);
        if self.sent_config.as_ref() != Some(config) {
            return;
        }
        let config_sent = out
            .link
            .iter()
            .any(|c| matches!(c, LinkCommand::ConfigureFrames(_)));
        let due = if config.enabled {
            now_ms.saturating_sub(self.config_sent_ms) >= RESEND_MS
        } else {
            reconnected && !config_sent
        };
        if !due {
            return;
        }
        self.config_sent_ms = now_ms;
        out.link.push(LinkCommand::ConfigureFrames(config.clone()));
        let target_sent = out
            .link
            .iter()
            .any(|c| matches!(c, LinkCommand::SetFramesTarget(_)));
        if config.enabled && !target_sent {
            out.link.push(LinkCommand::SetFramesTarget(
                self.target.as_ref().map(|t| t.pid),
            ));
        }
    }

    /// The target, and the frames of the received batches.
    fn step_target(&mut self, engine_on: bool, now_ms: u64, out: &mut Outputs) {
        if !engine_on {
            self.batches.clear();
            if self.target.is_some() {
                self.picker = new_picker(self.own_pid);
                self.set_target(None);
                out.link.push(LinkCommand::SetFramesTarget(None));
            }
            return;
        }
        if self.frames_available() {
            self.pick_target(now_ms, out);
        } else {
            self.hold_target(now_ms, out);
        }
        let recorded = self.bench_game.map(|(pid, _)| pid);
        let mut fresh = Vec::new();
        for batch in std::mem::take(&mut self.batches) {
            self.dropped = self.dropped.saturating_add(u64::from(batch.dropped));
            // A batch of the previous target may still arrive after a change.
            if self.qpc_frequency == 0 || Some(batch.pid) != self.target.as_ref().map(|t| t.pid) {
                continue;
            }
            let record = Some(batch.pid) == recorded;
            for frame in &batch.frames {
                let sample = sample_of(frame, self.qpc_frequency);
                self.window.push(sample);
                if record {
                    fresh.push(sample);
                }
            }
        }
        if !fresh.is_empty() {
            // Only the main swapchain is measured (DD7).
            let swapchain = self
                .swapchain
                .or_else(|| pick_swapchain(&self.window.last(LOWS_WINDOW_S)));
            self.swapchain = swapchain;
            fresh.retain(|f| Some(f.swapchain) == swapchain);
            let rows = self.recorder.on_frames(&fresh, now_ms);
            if !rows.is_empty() {
                out.benchmark.push(BenchmarkCommand::Rows(rows));
            }
        }
    }

    /// The normal choice: the [`TargetPicker`] over the service's list.
    fn pick_target(&mut self, now_ms: u64, out: &mut Outputs) {
        let list: Vec<ProcessInfo> = self
            .processes
            .iter()
            .map(|p| ProcessInfo {
                pid: p.pid,
                name: p.name.clone(),
                displayed_fps: p.displayed_fps,
            })
            .collect();
        self.picker.on_processes(&list, now_ms);
        if let Some(fg) = self.foreground {
            let beside = self
                .foreground_monitor
                .zip(self.geometry)
                .is_some_and(|(m, g)| m != g.monitor);
            self.picker.on_foreground(fg.pid, beside, now_ms);
        }
        self.picker.tick(now_ms);
        // Compared with our own target, which `hold_target` may have kept
        // or chosen while the picker was not fed.
        let current = self.picker.current().cloned();
        let pid = current.as_ref().map(|p| p.pid);
        if pid != self.target.as_ref().map(|t| t.pid) {
            self.set_target(current);
            out.link.push(LinkCommand::SetFramesTarget(pid));
        }
    }

    /// Without frame data (service down or incompatible, engine starting
    /// or not running) sensor blocks must keep working (spec §9): the target is
    /// kept, without the grace limit, while its window exists; without one,
    /// the foreground process becomes the target if its name, known from the
    /// last process list, has a profile in `gameProfiles`. Without any list
    /// (service down since the start) no game is recognised. Any other
    /// window (a browser) is never a target.
    /// The picker adopts the held target, to keep it once frames return.
    fn hold_target(&mut self, now_ms: u64, out: &mut Outputs) {
        if self.target.is_some() {
            if !self.target_window_alive() {
                self.set_target(None);
                self.picker.adopt(None, now_ms);
                out.link.push(LinkCommand::SetFramesTarget(None));
            }
            return;
        }
        let Some(fg) = self.foreground.filter(|fg| fg.pid != self.own_pid) else {
            return;
        };
        let Some(name) = self.known_names.get(&fg.pid) else {
            return;
        };
        let exe = name.to_lowercase();
        let excluded =
            OWN_PROCESS_NAMES.contains(&exe.as_str()) || SYSTEM_EXCLUDED.contains(&exe.as_str());
        if excluded || !self.settings.overlay.game_profiles.contains_key(&exe) {
            return;
        }
        let held = ProcessInfo {
            pid: fg.pid,
            name: name.clone(),
            displayed_fps: 0.0,
        };
        self.picker.adopt(Some(held.clone()), now_ms);
        self.set_target(Some(held));
        out.link.push(LinkCommand::SetFramesTarget(Some(fg.pid)));
    }

    /// The target's window is watched and has not been reported gone.
    fn target_window_alive(&self) -> bool {
        self.sent_track.is_some() && !(self.geometry_seen && self.geometry.is_none())
    }

    /// The frame engine gives data. While it starts (after a reconnection
    /// it may take long) its lists are empty, so the target is held rather
    /// than dropped after the picker's grace.
    fn frames_available(&self) -> bool {
        self.frames_state() == frames_state::RUNNING
    }

    fn set_target(&mut self, target: Option<ProcessInfo>) {
        self.tracked = target
            .as_ref()
            .and_then(|t| self.foreground.filter(|fg| fg.pid == t.pid));
        // A synthetic run starts afresh, without a gap in its window.
        if target.is_none() && self.editor.is_some() {
            self.editor = Some(SyntheticFeed::new());
        }
        self.target = target;
        self.window.clear();
        self.reset_times();
        self.choice = None;
        self.choice_dirty = true;
        self.sent_present_mode = None;
    }

    /// Follows the target's window (a game may replace it) and asks for its
    /// geometry while the overlay is on.
    fn step_track(&mut self, out: &mut Outputs) {
        if let (Some(target), Some(fg)) = (&self.target, self.foreground) {
            if fg.pid == target.pid {
                self.tracked = Some(fg);
            }
        }
        let wanted = self.tracked.filter(|_| self.settings.overlay.enabled);
        if wanted != self.sent_track {
            self.sent_track = wanted;
            // The previous window's geometry does not apply.
            self.geometry = None;
            self.geometry_seen = false;
            out.track = Some(wanted);
        }
    }

    /// The profile id the settings and the choice ask for.
    fn chosen_profile(&self) -> String {
        if let Some(choice) = &self.choice {
            return choice.clone();
        }
        let overlay = &self.settings.overlay;
        self.target
            .as_ref()
            .and_then(|t| overlay.game_profiles.get(&t.name.to_lowercase()))
            .unwrap_or(&overlay.default_profile)
            .clone()
    }

    fn refresh_profile(&mut self) {
        if !std::mem::take(&mut self.choice_dirty) {
            return;
        }
        let (id, profile) = self.catalog.resolve(&self.chosen_profile(), &self.schema);
        if self
            .active
            .as_ref()
            .is_some_and(|(i, p)| *i == id && *p == profile)
        {
            return;
        }
        self.active = Some((id, profile));
        self.profile_dirty = true;
        self.refresh_needs();
    }

    /// The sensors of the active and the preview profiles, the lows windows
    /// of those and of the edited one, and the frame window long enough for
    /// them.
    fn refresh_needs(&mut self) {
        let active = self.active.as_ref().map(|(_, p)| p);
        (self.used, _) = union_needs(active.into_iter().chain(&self.preview));
        let all = active
            .into_iter()
            .chain(&self.preview)
            .chain(&self.editor_profile);
        (_, self.lows) = union_needs(all);
        self.overlay_lows = active.map(low_windows).unwrap_or_default();
        let window_s = self
            .lows
            .iter()
            .map(|&(s, _)| f64::from(s))
            .fold(LOWS_WINDOW_S, f64::max)
            .min(MAX_LOWS_WINDOW_S);
        if window_s != self.window_s {
            // Keep the frames: the new window trims them to its own age.
            let mut window = FrameWindow::new(window_s);
            for frame in self.window.last(f64::INFINITY) {
                window.push(frame);
            }
            self.window = window;
            self.window_s = window_s;
        }
    }

    /// Where the overlay shows, if it does (DP9–DP11).
    fn placement(&self) -> Option<(PxArea, u32)> {
        let overlay = &self.settings.overlay;
        if !overlay.enabled || self.host != HostState::Running || self.hidden_by_user {
            return None;
        }
        let target = self.target.as_ref()?;
        if self.blocked(target) {
            return None;
        }
        let fg = self.foreground?;
        let g = self.geometry.filter(|g| g.visible && !g.minimized)?;
        // Another window in front of the game hides it; one on another
        // monitor (a second screen) leaves the game in view.
        let in_game = fg.pid == target.pid && self.tracked == Some(fg);
        let elsewhere = self.foreground_monitor.is_some_and(|m| m != g.monitor);
        if !in_game && !elsewhere {
            return None;
        }
        let r = match overlay.attach {
            Attach::Window => g.client,
            Attach::Monitor => g.monitor,
        };
        Some((
            PxArea {
                x: r.x,
                y: r.y,
                width: r.w,
                height: r.h,
            },
            g.dpi,
        ))
    }

    fn blocked(&self, target: &ProcessInfo) -> bool {
        self.blocked_exe(&target.name)
    }

    fn blocked_exe(&self, exe: &str) -> bool {
        self.settings
            .overlay
            .blocked_games
            .contains(&exe.to_lowercase())
    }

    /// Whether the overlay shows, and a fresh start for a new process.
    fn step_overlay(&mut self) {
        let running = self.host == HostState::Running;
        if running && !self.was_running {
            // A new overlay process knows nothing yet.
            self.sent_profile = None;
            self.sent_placement = None;
            self.profile_dirty = true;
            self.overlay_cursor = DataCursor::NEW;
            self.sent_bench = NO_BENCHMARK;
        }
        self.was_running = running;
        let was_shown = self.shown;
        self.shown = self.placement().is_some();
        if self.shown && !was_shown {
            // Shown again: its data at once, the frames missed included.
            self.overlay_cursor.pause();
        }
    }

    /// `SetProfile` and `SetPlacement`, while the overlay runs.
    fn step_overlay_messages(&mut self, out: &mut Outputs) {
        if self.host != HostState::Running {
            return;
        }
        if std::mem::take(&mut self.profile_dirty) {
            if let Some((id, profile)) = &self.active {
                let msg = set_profile(id, profile, &self.schema, &self.settings, self.lang);
                if self.sent_profile.as_ref() != Some(&msg) {
                    self.sent_profile = Some(msg.clone());
                    out.overlay.push(msg);
                }
            }
        }
        let placement = self.placement();
        if self.sent_placement != Some(placement) {
            self.sent_placement = Some(placement);
            out.overlay.push(OverlayMessage::SetPlacement(SetPlacement {
                area: placement.map(|(area, _)| area),
                dpi: placement.map_or(DEFAULT_DPI, |(_, dpi)| dpi),
            }));
        }
    }

    /// The preview's `SetProfile`, while it runs; it ignores placements.
    fn step_preview(&mut self, out: &mut Outputs) {
        let running = self.preview_running();
        if running && !self.preview_was_running {
            self.sent_preview = None;
            self.preview_dirty = true;
        }
        self.preview_was_running = running;
        if !running || !std::mem::take(&mut self.preview_dirty) {
            return;
        }
        if let Some(profile) = &self.preview {
            let msg = set_profile(PREVIEW_ID, profile, &self.schema, &self.settings, self.lang);
            if self.sent_preview.as_ref() != Some(&msg) {
                self.sent_preview = Some(msg.clone());
                out.preview.push(msg);
            }
        }
    }

    /// `FrameMetrics` at `textHz` and `FrameTimes` at 10 Hz for the overlay
    /// (while shown) and, on a cursor of their own, for the preview (while
    /// it runs) and the editor (while open). The metrics are computed once;
    /// the overlay's carry only the active profile's lows.
    fn step_data(&mut self, config: &FramesConfigure, now_ms: u64, out: &mut Outputs) {
        let (shown, preview, editor) = (self.shown, self.preview_running(), self.editor.is_some());
        let side = preview || editor;
        if !shown {
            self.overlay_cursor.pause();
        }
        if !side {
            self.side_cursor.pause();
        }
        let text_ms = 1_000 / u64::from(self.settings.overlay.text_hz.max(1));
        let overlay_metrics = shown && due(&mut self.overlay_cursor.metrics_ms, now_ms, text_ms);
        let side_metrics = side && due(&mut self.side_cursor.metrics_ms, now_ms, text_ms);
        let mut editor_due = false;
        if overlay_metrics || side_metrics {
            if let OverlayMessage::FrameMetrics(m) = self.metrics_now(config.track_gpu) {
                if overlay_metrics {
                    let mut own = m.clone();
                    keep_lows(&mut own, &self.overlay_lows);
                    out.overlay.push(OverlayMessage::FrameMetrics(own));
                }
                if side_metrics && preview {
                    out.preview.push(OverlayMessage::FrameMetrics(m.clone()));
                }
                if side_metrics && editor {
                    self.editor_metrics = Some(m);
                    editor_due = true;
                }
            }
        }
        let overlay_times = shown && due(&mut self.overlay_cursor.times_ms, now_ms, FRAME_TIMES_MS);
        let side_times = side && due(&mut self.side_cursor.times_ms, now_ms, FRAME_TIMES_MS);
        let mut frame_times = Vec::new();
        if overlay_times || side_times {
            let swapchain = self
                .swapchain
                .or_else(|| pick_swapchain(&self.frames_source().last(LOWS_WINDOW_S)));
            self.swapchain = swapchain;
            if overlay_times {
                let (msg, newest) =
                    frame_times_since(self.frames_source(), swapchain, self.overlay_cursor.after_s);
                self.overlay_cursor.after_s = newest;
                if newest_frames(&msg).is_some() {
                    out.overlay.push(msg);
                }
            }
            if side_times {
                let (msg, newest) =
                    frame_times_since(self.frames_source(), swapchain, self.side_cursor.after_s);
                self.side_cursor.after_s = newest;
                if let Some(frames) = newest_frames(&msg) {
                    if editor {
                        frame_times = frames.to_vec();
                    }
                    if preview {
                        out.preview.push(msg);
                    }
                }
            }
        }
        if editor_due || !frame_times.is_empty() {
            out.editor_data = self.editor_metrics.clone().map(|metrics| EditorData {
                metrics,
                frame_times,
            });
        }
    }

    /// `FrameMetrics` now: of the synthetic frames, the target's, or empty
    /// without the service.
    fn metrics_now(&mut self, track_gpu: bool) -> OverlayMessage {
        if let Some(feed) = self.synthetic() {
            // Every metric of the made-up game shows, the bottleneck too.
            let readout = read(feed.window(), &self.lows, true);
            return metrics_message(Some(&readout), frames_state::RUNNING);
        }
        if self.connected {
            let readout = self.readout(track_gpu);
            metrics_message(Some(&readout), self.frames_state())
        } else {
            metrics_message(None, FRAMES_UNAVAILABLE)
        }
    }
    /// A note when the foreground window or its standing in the service's
    /// list changed; the FPS value alone does not count.
    fn foreground_note(&mut self) -> Option<ForegroundNote> {
        let fg = self.foreground?;
        let listed = self.processes.iter().find(|p| p.pid == fg.pid);
        let key = (
            fg,
            listed.is_some(),
            listed.is_some_and(|p| p.displayed_fps >= MIN_GAME_FPS),
        );
        if self.sent_foreground.as_ref() == Some(&key) {
            return None;
        }
        self.sent_foreground = Some(key);
        Some(ForegroundNote {
            pid: fg.pid,
            name: self.known_names.get(&fg.pid).cloned(),
            presenting: listed.map(|p| (p.displayed_fps, p.present_mode.clone())),
        })
    }

    /// The exclusive-fullscreen notice, once per executable (DP16).
    fn toast(&mut self) -> Option<ToastRequest> {
        if !self.settings.overlay.enabled {
            return None;
        }
        let target = self.target.as_ref().filter(|t| !self.blocked(t))?;
        let process = self.processes.iter().find(|p| p.pid == target.pid)?;
        if !EXCLUSIVE_PRESENT_MODES.contains(&process.present_mode.as_str()) {
            return None;
        }
        let exe = target.name.to_lowercase();
        self.toasted
            .insert(exe.clone())
            .then_some(ToastRequest::ExclusiveFullscreen { exe })
    }

    /// The frame engine's state for the overlay and the UI: `off` while the
    /// engine is not wanted, whatever the service link (with the overlay off
    /// nothing steps on a reconnection, so its state would be stale).
    fn frames_state(&self) -> &str {
        if !self.wanted_config().enabled {
            return frames_state::OFF;
        }
        if !self.connected {
            return FRAMES_UNAVAILABLE;
        }
        match &self.status {
            Some(status) => state_label(Some(status)),
            None => frames_state::STARTING,
        }
    }

    fn readout(&mut self, track_gpu: bool) -> FrameReadout {
        let readout = read(&self.window, &self.lows, track_gpu);
        self.swapchain = readout.swapchain;
        readout
    }

    fn line(&self, readout: &FrameReadout) -> String {
        let detail = detail_label(self.status.as_ref());
        line(
            readout,
            &LineContext {
                target: self.target.as_ref().map(|t| (t.name.as_str(), t.pid)),
                state: state_label(self.status.as_ref()),
                detail: &detail,
                dropped: self.dropped,
            },
        )
    }
}

/// When a consumer of the frame data last got metrics and frame times, and
/// the newest frame time it got.
#[derive(Debug, Clone, Copy)]
struct DataCursor {
    metrics_ms: Option<u64>,
    times_ms: Option<u64>,
    after_s: f64,
}

impl DataCursor {
    const NEW: Self = Self {
        metrics_ms: None,
        times_ms: None,
        after_s: f64::NEG_INFINITY,
    };

    /// Due at the next step; the frames not yet sent stay due too.
    fn pause(&mut self) {
        self.metrics_ms = None;
        self.times_ms = None;
    }
}

/// Whether `period_ms` passed since `*last`; if so `*last` becomes now.
fn due(last: &mut Option<u64>, now_ms: u64, period_ms: u64) -> bool {
    let due = last.is_none_or(|t| now_ms.saturating_sub(t) >= period_ms);
    if due {
        *last = Some(now_ms);
    }
    due
}

/// The frames of a `FrameTimes` with at least one.
fn newest_frames(msg: &OverlayMessage) -> Option<&[WireFrameTime]> {
    match msg {
        OverlayMessage::FrameTimes(t) if !t.frames.is_empty() => Some(&t.frames),
        _ => None,
    }
}

/// The overlay's benchmark box when there is nothing to show.
const NO_BENCHMARK: BenchmarkOverlay = BenchmarkOverlay {
    recording_s: None,
    summary: None,
};

/// The local time now.
fn wall_clock() -> LocalTime {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    crate::report::local_now(now_ms)
}

fn new_picker(own_pid: u32) -> TargetPicker {
    TargetPicker::new(
        vec![own_pid],
        SYSTEM_EXCLUDED.iter().map(|s| (*s).to_owned()).collect(),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::model::{Label, Sensor, SensorKind, Source as SensorSource, Unit};
    use oma_ipc::overlay::{FrameTimes, SetProfile, Values};
    use oma_ipc::{PresentingProcess, WireFrame};

    use super::super::profiles::load_catalog;

    const FREQ: u64 = 10_000_000;
    const OWN: u32 = 7;
    const GAME: u32 = 100;
    const OTHER: u32 = 200;
    const GAME_FG: Foreground = Foreground {
        pid: GAME,
        hwnd: 0x1000,
    };
    const BROWSER_FG: Foreground = Foreground {
        pid: 300,
        hwnd: 0x3000,
    };
    const CLIENT: PxRect = PxRect {
        x: 10,
        y: 20,
        w: 1280,
        h: 720,
    };
    const MONITOR: PxRect = PxRect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    };

    fn config(enabled: bool, track_pc_latency: bool, track_gpu: bool) -> FramesConfigure {
        FramesConfigure {
            enabled,
            track_pc_latency,
            track_gpu,
        }
    }

    fn settings(enabled: bool) -> Settings {
        let mut s = Settings::default();
        s.overlay.enabled = enabled;
        s
    }

    fn status(state: &str) -> FramesStatus {
        FramesStatus {
            state: state.to_owned(),
            detail: None,
            presentmon_version: None,
        }
    }

    fn process(pid: u32, name: &str, present_mode: &str) -> PresentingProcess {
        PresentingProcess {
            pid,
            name: name.to_owned(),
            displayed_fps: 100.0,
            present_mode: present_mode.to_owned(),
            swapchains: 1,
        }
    }

    fn processes() -> PresentingProcesses {
        PresentingProcesses {
            at_qpc: 0,
            processes: vec![
                process(GAME, "my game.exe", "Hardware: Independent Flip"),
                process(OTHER, "other.exe", "Hardware: Independent Flip"),
            ],
        }
    }

    /// The game closed: only the other one presents.
    fn other_only() -> PresentingProcesses {
        PresentingProcesses {
            at_qpc: 0,
            processes: vec![process(OTHER, "other.exe", "Hardware: Independent Flip")],
        }
    }

    /// `count` displayed app frames 10 ms apart, from `first_s`.
    fn batch(pid: u32, first_s: f64, count: u32, dropped: u32) -> FrameBatch {
        let frames = (0..count)
            .map(|i| WireFrame {
                qpc: ((first_s + f64::from(i) * 0.01) * FREQ as f64) as u64,
                swapchain: 0xabc,
                frame_type: "app".to_owned(),
                displayed: true,
                ms_between_presents: 10.0,
                ms_between_display_change: Some(10.0),
                ms_until_displayed: Some(5.0),
                ms_app_frametime: Some(10.0),
                ms_pc_latency: None,
                ms_gpu_busy: None,
                pcl_frame_id: None,
            })
            .collect();
        FrameBatch {
            pid,
            frames,
            dropped,
        }
    }

    fn geometry(dpi: u32) -> WindowGeometry {
        WindowGeometry {
            client: CLIENT,
            monitor: MONITOR,
            dpi,
            minimized: false,
            visible: true,
        }
    }

    fn area(r: PxRect) -> PxArea {
        PxArea {
            x: r.x,
            y: r.y,
            width: r.w,
            height: r.h,
        }
    }

    /// The built-in profiles only.
    fn builtins() -> ProfileCatalog {
        load_catalog(&std::env::temp_dir().join("oma-controller-tests-no-such-dir"))
    }

    fn placements(out: &Outputs) -> Vec<SetPlacement> {
        out.overlay
            .iter()
            .filter_map(|m| match m {
                OverlayMessage::SetPlacement(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    fn profiles_sent(out: &Outputs) -> Vec<String> {
        out.overlay
            .iter()
            .filter_map(|m| match m {
                OverlayMessage::SetProfile(SetProfile { profile_id, .. }) => {
                    Some(profile_id.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn metrics(out: &Outputs) -> Vec<FrameMetrics> {
        out.overlay
            .iter()
            .filter_map(|m| match m {
                OverlayMessage::FrameMetrics(f) => Some(f.clone()),
                _ => None,
            })
            .collect()
    }

    fn times(out: &Outputs) -> Vec<FrameTimes> {
        out.overlay
            .iter()
            .filter_map(|m| match m {
                OverlayMessage::FrameTimes(f) => Some(f.clone()),
                _ => None,
            })
            .collect()
    }

    fn configs(out: &Outputs) -> Vec<FramesConfigure> {
        out.link
            .iter()
            .filter_map(|c| match c {
                LinkCommand::ConfigureFrames(f) => Some(f.clone()),
                _ => None,
            })
            .collect()
    }

    fn targets(out: &Outputs) -> Vec<Option<u32>> {
        out.link
            .iter()
            .filter_map(|c| match c {
                LinkCommand::SetFramesTarget(t) => Some(*t),
                _ => None,
            })
            .collect()
    }

    fn shown(area: PxArea, dpi: u32) -> SetPlacement {
        SetPlacement {
            area: Some(area),
            dpi,
        }
    }

    fn is_hidden(p: &SetPlacement) -> bool {
        p.area.is_none()
    }

    /// An overlay that is on, with a running process and the service up.
    fn enabled_with(settings: &Settings) -> Controller {
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(settings, Lang::En, None);
        c.on_catalog(builtins());
        c.on_service(true);
        c.on_host(HostState::Running);
        c
    }

    /// Following the game, its window in the foreground: shown at 100 ms.
    fn showing_with(settings: &Settings) -> Controller {
        let mut c = enabled_with(settings);
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        let out = c.step(0);
        assert_eq!(out.track, Some(Some(GAME_FG)));
        c.on_geometry(Some(geometry(96)));
        let out = c.step(100);
        assert_eq!(placements(&out), vec![shown(area(CLIENT), 96)]);
        c
    }

    fn showing() -> Controller {
        showing_with(&settings(true))
    }

    #[test]
    fn overlay_off_requests_nothing() {
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&settings(false), Lang::En, None);
        c.on_service(true);
        c.on_frames(Some(&status("off")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        for now in [0, 250, 1_000, 2_000, 5_000] {
            let out = c.step(now);
            assert!(out.link.is_empty(), "{now}: {:?}", out.link);
            assert!(out.overlay.is_empty());
            assert!(!out.want_process);
            assert_eq!(out.track, None);
            assert_eq!(out.toast, None);
            assert_eq!(out.diagnostics_line, None);
            assert!(!out.retry_host);
        }
        let snapshot = Snapshot {
            revision: 0,
            seq: 0,
            timestamp_ms: 0,
            values: vec![],
        };
        assert_eq!(
            c.on_tick_values(&Schema::default(), &snapshot, &[], 0),
            None
        );
    }

    #[test]
    fn enabling_starts_the_engine_and_the_overlay_hidden() {
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&settings(false), Lang::En, None);
        c.on_catalog(builtins());
        c.on_service(true);
        assert!(c.step(0).link.is_empty());
        c.on_settings(&settings(true), Lang::En, None);
        let out = c.step(250);
        assert!(out.want_process);
        assert_eq!(configs(&out), vec![config(true, false, false)]);
        assert!(out.overlay.is_empty(), "nothing before the process runs");
        let status = out.status.expect("the status changed");
        assert!(status.enabled);
        assert_eq!(status.process, "off", "until the host reports");
        assert_eq!(status.active_profile.as_deref(), Some("builtin-gaming"));
        c.on_host(HostState::Starting);
        let out = c.step(300);
        assert!(out.overlay.is_empty(), "nothing before the process runs");
        assert_eq!(out.status.unwrap().process, "starting");
        c.on_host(HostState::Running);
        let out = c.step(500);
        assert_eq!(profiles_sent(&out), vec!["builtin-gaming"]);
        let p = placements(&out);
        assert_eq!(p.len(), 1);
        assert!(is_hidden(&p[0]));
        assert_eq!(out.status.unwrap().process, "running");
        // Nothing changes: nothing is sent again.
        let out = c.step(750);
        assert!(out.overlay.is_empty());
    }

    #[test]
    fn game_in_foreground_shows_the_overlay_on_its_client_area() {
        let mut c = enabled_with(&settings(true));
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        let out = c.step(0);
        assert_eq!(targets(&out), vec![Some(GAME)]);
        assert_eq!(out.track, Some(Some(GAME_FG)));
        // Without the window's geometry, nothing to place yet.
        assert!(placements(&out).iter().all(is_hidden));
        c.on_geometry(Some(geometry(120)));
        let out = c.step(100);
        assert_eq!(placements(&out), vec![shown(area(CLIENT), 120)]);
        assert_eq!(
            out.values_plan,
            Some(ValuesPlan {
                used: vec![],
                wanted: true
            })
        );
        let status = c.step(200).status;
        assert_eq!(status, None, "unchanged");
        let s = c.current_status();
        assert_eq!(
            s.target,
            Some(TargetStatus {
                name: "my game.exe".into(),
                pid: GAME
            })
        );
    }

    #[test]
    fn attach_monitor_uses_the_monitor_rect() {
        let mut s = settings(true);
        s.overlay.attach = Attach::Monitor;
        let mut c = enabled_with(&s);
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        c.step(0);
        c.on_geometry(Some(geometry(144)));
        let out = c.step(100);
        assert_eq!(placements(&out), vec![shown(area(MONITOR), 144)]);
    }

    #[test]
    fn leaving_the_game_hides_at_once_but_keeps_the_target() {
        let mut c = showing();
        c.on_foreground(BROWSER_FG);
        let out = c.step(200);
        let p = placements(&out);
        assert_eq!(p.len(), 1);
        assert!(is_hidden(&p[0]));
        assert!(targets(&out).is_empty());
        assert_eq!(out.track, None, "still watching the game's window");
        assert_eq!(c.current_status().target.map(|t| t.pid), Some(GAME));
        assert_eq!(
            out.values_plan,
            Some(ValuesPlan {
                used: vec![],
                wanted: false
            })
        );
        // Back in the game before the grace ran out: shown again.
        c.on_foreground(GAME_FG);
        let out = c.step(900);
        assert_eq!(placements(&out), vec![shown(area(CLIENT), 96)]);
        assert!(targets(&out).is_empty());
    }

    /// A monitor to the right of [`MONITOR`].
    const SECOND: PxRect = PxRect {
        x: 1920,
        y: 0,
        w: 2560,
        h: 1440,
    };

    #[test]
    fn window_on_another_monitor_keeps_the_overlay_on_the_game() {
        let mut c = showing();
        c.on_foreground(BROWSER_FG);
        c.on_foreground_monitor(Some(SECOND));
        let out = c.step(200);
        assert!(placements(&out).is_empty(), "still shown on the game");
        assert_eq!(out.values_plan, None, "data keeps flowing");
        assert_eq!(c.current_status().target.map(|t| t.pid), Some(GAME));
        // Long after the grace: the game still presents, still the target.
        assert!(placements(&c.step(10_000)).is_empty());
        assert_eq!(c.current_status().target.map(|t| t.pid), Some(GAME));
    }

    #[test]
    fn window_on_the_game_monitor_hides_the_overlay() {
        let mut c = showing();
        c.on_foreground(BROWSER_FG);
        c.on_foreground_monitor(Some(MONITOR));
        let p = placements(&c.step(200));
        assert_eq!(p.len(), 1);
        assert!(is_hidden(&p[0]));
    }

    #[test]
    fn another_game_does_not_take_a_presenting_target() {
        let mut c = showing();
        c.on_foreground(Foreground {
            pid: OTHER,
            hwnd: 0x2000,
        });
        c.on_foreground_monitor(Some(SECOND));
        let out = c.step(200);
        assert!(targets(&out).is_empty());
        assert!(placements(&out).is_empty(), "still shown on the game");
        assert_eq!(c.current_status().target.map(|t| t.pid), Some(GAME));
    }

    #[test]
    fn game_on_the_same_monitor_takes_the_target() {
        let mut c = showing();
        c.on_foreground(Foreground {
            pid: OTHER,
            hwnd: 0x2000,
        });
        c.on_foreground_monitor(Some(MONITOR));
        let out = c.step(200);
        assert_eq!(targets(&out), vec![Some(OTHER)]);
    }

    #[test]
    fn minimized_game_hides_the_overlay() {
        let mut c = showing();
        let mut g = geometry(96);
        g.minimized = true;
        c.on_geometry(Some(g));
        let p = placements(&c.step(200));
        assert_eq!(p.len(), 1);
        assert!(is_hidden(&p[0]));
        let mut g = geometry(96);
        g.visible = false;
        c.on_geometry(Some(g));
        assert!(placements(&c.step(300)).is_empty(), "still hidden");
        c.on_geometry(Some(geometry(96)));
        assert_eq!(placements(&c.step(400)), vec![shown(area(CLIENT), 96)]);
    }

    #[test]
    fn moving_to_another_monitor_sends_the_new_dpi() {
        let mut c = showing();
        let mut g = geometry(144);
        g.client.x = 2_000;
        c.on_geometry(Some(g));
        let mut moved = area(CLIENT);
        moved.x = 2_000;
        assert_eq!(placements(&c.step(200)), vec![shown(moved, 144)]);
        // The same geometry again: nothing.
        c.on_geometry(Some(g));
        assert!(placements(&c.step(300)).is_empty());
    }

    #[test]
    fn blocked_game_stays_target_but_overlay_hidden() {
        let mut s = settings(true);
        s.overlay.blocked_games = vec!["my game.exe".into()];
        let mut c = enabled_with(&s);
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        let out = c.step(0);
        assert_eq!(targets(&out), vec![Some(GAME)]);
        c.on_geometry(Some(geometry(96)));
        let out = c.step(100);
        assert!(placements(&out).iter().all(is_hidden));
        assert_eq!(c.current_status().target.map(|t| t.pid), Some(GAME));
        // Unblocked: shown.
        c.on_settings(&settings(true), Lang::En, None);
        assert_eq!(placements(&c.step(200)), vec![shown(area(CLIENT), 96)]);
    }

    #[test]
    fn game_profile_is_activated_on_target_change() {
        let mut s = settings(true);
        s.overlay
            .game_profiles
            .insert("my game.exe".into(), "builtin-full".into());
        let mut c = enabled_with(&s);
        assert_eq!(profiles_sent(&c.step(0)), vec!["builtin-gaming"]);
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        let out = c.step(100);
        assert_eq!(profiles_sent(&out), vec!["builtin-full"]);
        assert_eq!(
            out.status.unwrap().active_profile.as_deref(),
            Some("builtin-full")
        );
        // The game closes and another one without a profile of its own
        // comes to the foreground: back to the default.
        c.on_frames(Some(&status("running")), Some(&other_only()), &[]);
        c.on_foreground(Foreground {
            pid: OTHER,
            hwnd: 0x2000,
        });
        let out = c.step(200);
        assert_eq!(targets(&out), vec![Some(OTHER)]);
        assert_eq!(profiles_sent(&out), vec!["builtin-gaming"]);
    }

    #[test]
    fn unknown_game_uses_default_profile() {
        let mut s = settings(true);
        s.overlay.default_profile = "builtin-bar".into();
        s.overlay
            .game_profiles
            .insert("someone else.exe".into(), "builtin-full".into());
        let mut c = enabled_with(&s);
        assert_eq!(profiles_sent(&c.step(0)), vec!["builtin-bar"]);
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        let out = c.step(100);
        assert_eq!(targets(&out), vec![Some(GAME)]);
        assert!(profiles_sent(&out).is_empty(), "still builtin-bar");
        assert_eq!(
            c.current_status().active_profile.as_deref(),
            Some("builtin-bar")
        );
    }

    #[test]
    fn next_profile_lasts_until_target_change() {
        let mut c = showing();
        c.next_profile();
        assert_eq!(profiles_sent(&c.step(200)), vec!["builtin-full"]);
        c.next_profile();
        assert_eq!(profiles_sent(&c.step(300)), vec!["builtin-bar"]);
        // A short alt-tab keeps the target, and the choice.
        c.on_foreground(BROWSER_FG);
        assert!(profiles_sent(&c.step(400)).is_empty());
        c.on_foreground(GAME_FG);
        assert!(profiles_sent(&c.step(500)).is_empty());
        // The game closes, another one comes: the choice ends.
        c.on_frames(Some(&status("running")), Some(&other_only()), &[]);
        c.on_foreground(Foreground {
            pid: OTHER,
            hwnd: 0x2000,
        });
        let out = c.step(600);
        assert_eq!(targets(&out), vec![Some(OTHER)]);
        assert_eq!(profiles_sent(&out), vec!["builtin-gaming"]);
    }

    #[test]
    fn changing_the_profile_settings_ends_the_choice() {
        let mut c = showing();
        c.next_profile();
        assert_eq!(profiles_sent(&c.step(200)), vec!["builtin-full"]);
        // The default profile changes in the settings: it shows at once.
        let mut s = settings(true);
        s.overlay.default_profile = "builtin-bar".into();
        c.on_settings(&s, Lang::En, None);
        assert_eq!(profiles_sent(&c.step(300)), vec!["builtin-bar"]);
        assert_eq!(
            c.current_status().active_profile.as_deref(),
            Some("builtin-bar")
        );
        // A choice, then a game association for another profile.
        c.next_profile();
        c.step(400);
        s.overlay
            .game_profiles
            .insert("my game.exe".into(), "builtin-gaming".into());
        c.on_settings(&s, Lang::En, None);
        assert_eq!(profiles_sent(&c.step(500)), vec!["builtin-gaming"]);
    }

    #[test]
    fn other_settings_keep_the_choice() {
        let mut c = showing();
        c.next_profile();
        assert_eq!(profiles_sent(&c.step(200)), vec!["builtin-full"]);
        let mut s = settings(true);
        s.overlay.hide_from_capture = !s.overlay.hide_from_capture;
        c.on_settings(&s, Lang::En, None);
        c.step(300);
        assert_eq!(
            c.current_status().active_profile.as_deref(),
            Some("builtin-full")
        );
    }

    #[test]
    fn user_hide_hides_and_toggle_shows_again() {
        let mut c = showing();
        c.toggle_hidden();
        let out = c.step(200);
        let p = placements(&out);
        assert_eq!(p.len(), 1);
        assert!(is_hidden(&p[0]));
        assert!(out.status.unwrap().hidden_by_user);
        // No data while hidden.
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 5.0, 50, 0)],
        );
        for now in [300, 700, 1_200] {
            let out = c.step(now);
            assert!(metrics(&out).is_empty() && times(&out).is_empty());
        }
        c.toggle_hidden();
        let out = c.step(1_300);
        assert_eq!(placements(&out), vec![shown(area(CLIENT), 96)]);
        assert_eq!(metrics(&out).len(), 1);
        assert_eq!(times(&out).len(), 1);
        assert!(!out.status.unwrap().hidden_by_user);
    }

    #[test]
    fn toggle_with_the_overlay_off_does_nothing() {
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&settings(false), Lang::En, None);
        c.on_catalog(builtins());
        c.on_service(true);
        c.step(0);
        c.toggle_hidden();
        let out = c.step(100);
        assert!(!c.current_status().hidden_by_user);
        assert!(out.status.is_none(), "nothing changed");
        // Turned on later: the overlay shows over the game.
        c.on_settings(&settings(true), Lang::En, None);
        c.on_host(HostState::Running);
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        let out = c.step(200);
        assert!(!out.status.unwrap().hidden_by_user);
        c.on_geometry(Some(geometry(96)));
        assert_eq!(placements(&c.step(300)), vec![shown(area(CLIENT), 96)]);
    }

    #[test]
    fn engine_not_wanted_reports_frames_off() {
        // The first step may come before the service link is up.
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&settings(false), Lang::En, None);
        let status = c.step(0).status.unwrap();
        assert_eq!(status.frames, frames_state::OFF);
        assert_eq!(status.frames_detail, None);
        // Wanted (here by OMA_FRAMES_DEBUG) without the service: unavailable.
        c.on_settings(&settings(false), Lang::En, Some(config(true, false, false)));
        assert_eq!(c.step(100).status.unwrap().frames, FRAMES_UNAVAILABLE);
    }

    #[test]
    fn turning_the_overlay_on_clears_a_user_hide() {
        let mut c = showing();
        c.toggle_hidden();
        assert!(c.step(200).status.unwrap().hidden_by_user);
        c.on_settings(&settings(false), Lang::En, None);
        c.step(300);
        c.on_settings(&settings(true), Lang::En, None);
        let out = c.step(400);
        assert!(!out.status.unwrap().hidden_by_user);
        // The game's window is watched again, then the overlay shows on it.
        assert_eq!(out.track, Some(Some(GAME_FG)));
        c.on_geometry(Some(geometry(96)));
        assert_eq!(placements(&c.step(500)), vec![shown(area(CLIENT), 96)]);
    }

    #[test]
    fn hotkey_statuses_reach_the_overlay_status() {
        use crate::log::HotkeyState;
        let mut c = enabled_with(&settings(true));
        c.step(0);
        let hotkeys = OverlayHotkeys {
            toggle: HotkeyStatus {
                requested: Some("Ctrl+Alt+F1".to_owned()),
                effective: Some("Ctrl+Alt+F1".to_owned()),
                state: HotkeyState::Active,
                reason: None,
            },
            next_profile: HotkeyStatus {
                requested: Some("Ctrl+Alt+F2".to_owned()),
                effective: None,
                state: HotkeyState::Failed,
                reason: Some("log.hotkey.inUse".to_owned()),
            },
            benchmark: HotkeyStatus::default(),
        };
        c.on_hotkeys(hotkeys.clone());
        let out = c.step(100);
        assert_eq!(out.status.unwrap().hotkeys, hotkeys);
        assert_eq!(c.current_status().hotkeys, hotkeys);
        // The same statuses again: no new status.
        c.on_hotkeys(hotkeys);
        assert!(c.step(200).status.is_none());
    }

    #[test]
    fn retry_after_failed_sends_disabled_then_enabled() {
        let mut c = enabled_with(&settings(true));
        c.on_frames(Some(&status("running")), None, &[]);
        c.step(0);
        // Running: «Retry» does nothing.
        c.retry();
        assert!(configs(&c.step(100)).is_empty());
        for state in ["failed", "denied"] {
            c.on_frames(Some(&status(state)), None, &[]);
            c.step(200);
            c.retry();
            let out = c.step(300);
            assert_eq!(configs(&out), vec![config(false, false, false)], "{state}");
            assert!(!out.retry_host);
            let out = c.step(400);
            assert_eq!(configs(&out), vec![config(true, false, false)], "{state}");
        }
        // A failed overlay process: the host retries.
        c.on_host(HostState::Failed {
            reason: HostFailure::Crashing,
        });
        let out = c.step(500);
        assert_eq!(
            out.status.unwrap().process_reason.as_deref(),
            Some("crashing")
        );
        c.retry();
        assert!(c.step(600).retry_host);
        assert!(!c.step(700).retry_host);
    }

    #[test]
    fn exclusive_fullscreen_toasts_once_per_game() {
        let legacy = PresentingProcesses {
            at_qpc: 0,
            processes: vec![
                process(GAME, "My Game.exe", "Hardware: Legacy Flip"),
                process(OTHER, "other.exe", "Hardware: Legacy Copy to front buffer"),
            ],
        };
        let mut c = enabled_with(&settings(true));
        c.on_frames(Some(&status("running")), Some(&legacy), &[]);
        c.on_foreground(GAME_FG);
        assert_eq!(
            c.step(0).toast,
            Some(ToastRequest::ExclusiveFullscreen {
                exe: "my game.exe".into()
            })
        );
        assert_eq!(c.step(100).toast, None);
        // Elsewhere while it stops presenting until the target drops, then
        // back: no second toast.
        let other_only = PresentingProcesses {
            at_qpc: 0,
            processes: vec![process(
                OTHER,
                "other.exe",
                "Hardware: Legacy Copy to front buffer",
            )],
        };
        c.on_frames(Some(&status("running")), Some(&other_only), &[]);
        c.on_foreground(BROWSER_FG);
        c.step(200);
        c.step(3_300);
        assert_eq!(c.current_status().target, None);
        c.on_frames(Some(&status("running")), Some(&legacy), &[]);
        c.on_foreground(GAME_FG);
        assert_eq!(c.step(3_400).toast, None);
        // The game closes; another game in exclusive fullscreen gets its own.
        c.on_frames(Some(&status("running")), Some(&other_only), &[]);
        c.on_foreground(Foreground {
            pid: OTHER,
            hwnd: 0x2000,
        });
        assert_eq!(
            c.step(3_500).toast,
            Some(ToastRequest::ExclusiveFullscreen {
                exe: "other.exe".into()
            })
        );
    }

    #[test]
    fn target_present_mode_reported_on_change() {
        let mut c = enabled_with(&settings(true));
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        assert_eq!(
            c.step(0).present_mode.as_deref(),
            Some("Hardware: Independent Flip")
        );
        assert_eq!(c.step(100).present_mode, None, "unchanged");
        let legacy = PresentingProcesses {
            at_qpc: 0,
            processes: vec![process(GAME, "my game.exe", "Hardware: Legacy Flip")],
        };
        c.on_frames(Some(&status("running")), Some(&legacy), &[]);
        assert_eq!(
            c.step(200).present_mode.as_deref(),
            Some("Hardware: Legacy Flip")
        );
    }

    #[test]
    fn foreground_note_on_change_of_window_or_standing() {
        let mut c = enabled_with(&settings(true));
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(BROWSER_FG);
        let note = c.step(0).foreground_note.unwrap();
        assert_eq!(note.pid, 300);
        assert_eq!(note.presenting, None, "not in the list");
        assert_eq!(c.step(100).foreground_note, None, "unchanged");
        c.on_foreground(GAME_FG);
        let note = c.step(200).foreground_note.unwrap();
        assert_eq!(note.name.as_deref(), Some("my game.exe"));
        assert_eq!(
            note.presenting,
            Some((100.0, "Hardware: Independent Flip".to_owned()))
        );
        // Still presenting at another rate: no new note.
        let mut slower = processes();
        slower.processes[0].displayed_fps = 90.0;
        c.on_frames(Some(&status("running")), Some(&slower), &[]);
        assert_eq!(c.step(300).foreground_note, None);
        // Below a game's rate: a note.
        slower.processes[0].displayed_fps = 3.0;
        c.on_frames(Some(&status("running")), Some(&slower), &[]);
        let note = c.step(400).foreground_note.unwrap();
        assert_eq!(note.presenting.map(|(fps, _)| fps), Some(3.0));
    }

    #[test]
    fn frame_metrics_sent_at_text_hz() {
        let mut c = showing();
        // Shown at 100 with metrics; then at 2 Hz.
        let sent: Vec<u64> = (2..=12)
            .map(|i| i * 100)
            .filter(|&now| !metrics(&c.step(now)).is_empty())
            .collect();
        assert_eq!(sent, vec![600, 1_100]);
        let mut s = settings(true);
        s.overlay.text_hz = 4;
        c.on_settings(&s, Lang::En, None);
        let sent: Vec<u64> = (13..=22)
            .map(|i| i * 100)
            .filter(|&now| !metrics(&c.step(now)).is_empty())
            .collect();
        assert_eq!(sent, vec![1_400, 1_700, 2_000]);
    }

    #[test]
    fn frame_times_sent_at_10_hz_only_new_frames() {
        let mut c = showing();
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 5.0, 20, 0)],
        );
        assert!(times(&c.step(150)).is_empty(), "not 100 ms yet");
        let sent = times(&c.step(200));
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].frames.len(), 20);
        assert!(times(&c.step(300)).is_empty(), "no new frames");
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 5.2, 5, 0), batch(OTHER, 5.2, 5, 0)],
        );
        let sent = times(&c.step(400));
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].frames.len(), 5);
        assert!((sent[0].frames[0].t_s - 5.2).abs() < 1e-6);
    }

    #[test]
    fn service_down_sends_unavailable_metrics() {
        let mut c = showing();
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 5.0, 101, 0)],
        );
        let m = metrics(&c.step(600));
        assert_eq!(m[0].state, "running");
        assert!(m[0].fps_displayed.is_some());
        c.on_service(false);
        let out = c.step(1_100);
        let m = metrics(&out);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].state, FRAMES_UNAVAILABLE);
        assert_eq!(m[0].fps_displayed, None);
        assert!(m[0].lows.is_empty());
        assert_eq!(out.status.unwrap().frames, FRAMES_UNAVAILABLE);
    }

    #[test]
    fn set_profile_resent_after_overlay_restart() {
        let mut c = showing();
        c.on_host(HostState::Starting);
        let out = c.step(200);
        assert!(out.overlay.is_empty());
        assert_eq!(
            out.values_plan.map(|p| p.wanted),
            Some(false),
            "no values for a restarting overlay"
        );
        c.on_host(HostState::Running);
        let out = c.step(300);
        assert_eq!(profiles_sent(&out), vec!["builtin-gaming"]);
        assert_eq!(placements(&out), vec![shown(area(CLIENT), 96)]);
        // SetProfile comes before SetPlacement.
        assert!(matches!(out.overlay[0], OverlayMessage::SetProfile(_)));
    }

    #[test]
    fn env_debug_config_merges_with_settings() {
        // Overlay off: the variable alone, as in M7b.
        let mut off = settings(false);
        off.overlay.track_gpu = true;
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&off, Lang::En, Some(config(true, false, false)));
        let out = c.step(0);
        assert_eq!(configs(&out), vec![config(true, false, false)]);
        assert!(!out.want_process);
        // Overlay on: the OR of both.
        let mut on = settings(true);
        on.overlay.track_gpu = true;
        c.on_settings(&on, Lang::En, Some(config(true, true, false)));
        assert_eq!(configs(&c.step(100)), vec![config(true, true, true)]);
        // Re-sent once a second.
        assert!(configs(&c.step(600)).is_empty());
        assert_eq!(configs(&c.step(1_100)), vec![config(true, true, true)]);
        // Both off: the engine is turned off.
        c.on_settings(&settings(false), Lang::En, None);
        assert_eq!(configs(&c.step(1_200)), vec![config(false, false, false)]);
    }

    #[test]
    fn service_down_from_startup_keeps_the_overlay_hidden() {
        // Documented limitation: without the service there is no process
        // name, so no game is ever recognised (never the browser either).
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&settings(true), Lang::En, None);
        c.on_catalog(builtins());
        c.on_host(HostState::Running);
        c.on_foreground(GAME_FG);
        let out = c.step(0);
        assert!(placements(&out).iter().all(is_hidden));
        assert_eq!(out.track, None);
        c.on_geometry(Some(geometry(96)));
        assert!(placements(&c.step(5_000)).is_empty());
        let status = c.current_status();
        assert_eq!(status.target, None);
        assert_eq!(status.frames, FRAMES_UNAVAILABLE);
        assert!(!c.values_plan().wanted);
    }

    #[test]
    fn service_drop_during_a_game_keeps_the_overlay_on_the_game() {
        let mut c = showing();
        c.on_service(false);
        c.on_frames(None, None, &[]);
        for now in [1_100, 3_500, 10_500, 20_000] {
            let out = c.step(now);
            assert!(placements(&out).is_empty(), "{now}: still shown");
            // Only the once-a-second re-send of the same target.
            assert!(targets(&out).iter().all(|t| *t == Some(GAME)), "{now}");
            for m in metrics(&out) {
                assert_eq!(m.state, FRAMES_UNAVAILABLE);
            }
        }
        assert_eq!(c.current_status().target.map(|t| t.pid), Some(GAME));
        assert!(c.values_plan().wanted);
        // The frame engine down with the service up behaves the same.
        c.on_service(true);
        c.on_frames(Some(&status("missing")), None, &[]);
        let out = c.step(30_000);
        assert!(placements(&out).is_empty());
        assert!(targets(&out).iter().all(|t| *t == Some(GAME)));
        // The service back with the game in front: the same target, kept.
        c.next_profile();
        c.step(30_100);
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        let out = c.step(30_200);
        assert!(targets(&out).iter().all(|t| *t == Some(GAME)));
        assert!(profiles_sent(&out).is_empty(), "the choice survives");
        assert_eq!(
            c.current_status().active_profile.as_deref(),
            Some("builtin-full")
        );
    }

    #[test]
    fn reconnection_keeps_the_target_while_the_engine_starts() {
        let mut c = showing();
        c.next_profile();
        c.step(200);
        c.on_service(false);
        c.on_frames(None, None, &[]);
        c.step(1_000);
        // Back, the engine `starting` (it may take long) with empty lists:
        // nothing changes, past the picker's grace too.
        c.on_service(true);
        let empty = PresentingProcesses {
            at_qpc: 0,
            processes: vec![],
        };
        for now in [5_000, 6_000, 9_000, 15_000] {
            c.on_frames(Some(&status("starting")), Some(&empty), &[]);
            let out = c.step(now);
            assert!(targets(&out).iter().all(|t| *t == Some(GAME)), "{now}");
            assert!(placements(&out).is_empty(), "{now}: still shown");
        }
        assert_eq!(c.current_status().target.map(|t| t.pid), Some(GAME));
        // Running: the same game, the choice kept.
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        let out = c.step(15_100);
        assert!(targets(&out).iter().all(|t| *t == Some(GAME)), "{out:?}");
        assert!(placements(&out).is_empty());
        assert_eq!(
            c.current_status().active_profile.as_deref(),
            Some("builtin-full")
        );
    }

    #[test]
    fn game_window_closing_while_service_down_hides() {
        let mut c = showing();
        c.on_service(false);
        c.on_frames(None, None, &[]);
        c.step(200);
        c.on_geometry(None);
        let out = c.step(300);
        let p = placements(&out);
        assert_eq!(p.len(), 1);
        assert!(is_hidden(&p[0]));
        assert_eq!(out.track, Some(None));
        assert_eq!(c.current_status().target, None);
        // The game's window again, unknown to `gameProfiles`: not picked.
        c.on_foreground(GAME_FG);
        assert_eq!(c.step(400).track, None);
        assert_eq!(c.current_status().target, None);
    }

    #[test]
    fn service_down_picks_a_known_game_with_a_profile() {
        let mut s = settings(true);
        s.overlay
            .game_profiles
            .insert("my game.exe".into(), "builtin-full".into());
        let mut c = showing_with(&s);
        c.on_service(false);
        c.on_frames(None, None, &[]);
        c.on_geometry(None);
        c.step(200);
        assert_eq!(c.current_status().target, None);
        // Its name is known from the last process list.
        c.on_foreground(GAME_FG);
        let out = c.step(300);
        assert_eq!(out.track, Some(Some(GAME_FG)));
        assert_eq!(c.current_status().target.map(|t| t.pid), Some(GAME));
        c.on_geometry(Some(geometry(96)));
        assert_eq!(placements(&c.step(400)), vec![shown(area(CLIENT), 96)]);
        // A known process without a profile of its own is never picked.
        c.on_geometry(None);
        c.step(500);
        c.on_foreground(Foreground {
            pid: OTHER,
            hwnd: 0x2000,
        });
        c.step(600);
        assert_eq!(c.current_status().target, None);
    }

    #[test]
    fn engine_off_is_not_resent_every_second() {
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&settings(true), Lang::En, None);
        c.on_service(true);
        c.step(0);
        c.on_settings(&settings(false), Lang::En, None);
        assert_eq!(configs(&c.step(100)), vec![config(false, false, false)]);
        for now in [1_100, 2_100, 5_000] {
            assert!(c.step(now).link.is_empty(), "{now}");
        }
        // After a reconnection, once.
        c.on_service(false);
        c.step(5_100);
        c.on_service(true);
        assert_eq!(configs(&c.step(5_200)), vec![config(false, false, false)]);
        assert!(c.step(6_300).link.is_empty());
    }

    fn field<'a>(line: &'a str, key: &str) -> &'a str {
        line.split(' ')
            .find_map(|kv| kv.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
            .unwrap_or_else(|| panic!("no {key} in {line}"))
    }

    /// The M7b diagnostics: the variable alone, the overlay off.
    fn diagnostics(qpc_frequency: u64) -> Controller {
        let mut c = Controller::new(OWN, qpc_frequency);
        c.on_settings(&settings(false), Lang::En, Some(config(true, false, false)));
        c.on_service(true);
        c
    }

    #[test]
    fn diagnostics_line_unchanged_format() {
        // Without data: every key, with dashes.
        let mut d = diagnostics(FREQ);
        let line = d
            .step(1_000)
            .diagnostics_line
            .expect("a line after one second");
        assert_eq!(
            line,
            "frames: target=- pid=- state=- detail=- fps=- rendered=- source=- mult=- low1=- low01=- \
             ft_ms=- stutter=- stutter_pct=- pc_lat_ms=- disp_lat_ms=- bottleneck=- dropped=0"
        );

        // The target's frames, once a second.
        let mut d = diagnostics(FREQ);
        d.on_frames(Some(&status("running")), Some(&processes()), &[]);
        d.on_foreground(GAME_FG);
        let first = d.step(0);
        assert_eq!(targets(&first), vec![Some(GAME)]);
        assert_eq!(first.diagnostics_line, None);
        // Another process's frames, distinguishable: they would pull the
        // displayed FPS far from 100 if they reached the window.
        let mut foreign = batch(999, 5.0, 50, 2);
        for frame in &mut foreign.frames {
            frame.ms_between_display_change = Some(50.0);
        }
        d.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 5.0, 101, 3), foreign],
        );
        let step = d.step(1_000);
        let line = step.diagnostics_line.expect("a line");
        assert_eq!(field(&line, "target"), "my_game.exe");
        assert_eq!(field(&line, "pid"), "100");
        assert_eq!(field(&line, "state"), "running");
        assert_eq!(field(&line, "fps"), "100.0");
        assert_eq!(field(&line, "ft_ms"), "10.00");
        assert_eq!(field(&line, "disp_lat_ms"), "5.0");
        assert_eq!(field(&line, "stutter"), "0");
        assert_eq!(field(&line, "dropped"), "5");
        d.on_frames(None, Some(&processes()), &[]);
        assert_eq!(d.step(1_250).diagnostics_line, None);
    }

    #[test]
    fn diagnostics_unknown_state_is_logged_as_failed() {
        let mut d = diagnostics(FREQ);
        d.on_frames(Some(&status("exploded")), None, &[]);
        let line = d.step(1_000).diagnostics_line.unwrap();
        assert_eq!(field(&line, "state"), "failed");
        assert_eq!(field(&line, "detail"), "-");
        let mut with_detail = status("failed");
        with_detail.detail = Some("bad columns".to_owned());
        d.on_frames(Some(&with_detail), None, &[]);
        let line = d.step(2_000).diagnostics_line.unwrap();
        assert_eq!(field(&line, "detail"), "bad_columns");
    }

    #[test]
    fn diagnostics_target_change_clears_the_window() {
        let mut d = diagnostics(FREQ);
        d.on_frames(None, Some(&processes()), &[]);
        d.on_foreground(GAME_FG);
        d.step(0);
        d.on_frames(None, Some(&processes()), &[batch(GAME, 5.0, 50, 0)]);
        d.step(250);
        // Elsewhere from 500 ms for 3 s: the target drops and its frames go
        // with it.
        d.on_foreground(Foreground {
            pid: 300,
            hwnd: 0x3000,
        });
        assert!(targets(&d.step(500)).is_empty());
        let step = d.step(3_500);
        assert_eq!(targets(&step), vec![None]);
        let line = step.diagnostics_line.unwrap();
        assert_eq!(field(&line, "fps"), "-");
        assert_eq!(field(&line, "target"), "-");
    }

    #[test]
    fn diagnostics_zero_qpc_frequency_ignores_frames() {
        let mut d = diagnostics(0);
        d.on_frames(None, Some(&processes()), &[]);
        d.on_foreground(GAME_FG);
        d.step(0);
        d.on_frames(None, Some(&processes()), &[batch(GAME, 5.0, 50, 1)]);
        let line = d.step(1_000).diagnostics_line.unwrap();
        assert_eq!(field(&line, "fps"), "-");
        assert_eq!(field(&line, "dropped"), "1");
    }

    /// A profile with a text block on `sensor`.
    fn sensor_profile(sensor: &str) -> Profile {
        oma_core::overlay::parse_profile(
            &serde_json::json!({ "format": 1, "name": "p", "blocks": [{
                "id": "a", "rect": { "x": 0, "y": 0, "w": 4, "h": 2 },
                "source": { "sensor": sensor }, "kind": "text"
            }] })
            .to_string(),
        )
        .unwrap()
    }

    fn preview_profiles(out: &Outputs) -> Vec<SetProfile> {
        out.preview
            .iter()
            .filter_map(|m| match m {
                OverlayMessage::SetProfile(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    fn data_of(messages: &[OverlayMessage]) -> Vec<OverlayMessage> {
        messages
            .iter()
            .filter(|m| {
                matches!(
                    m,
                    OverlayMessage::FrameMetrics(_) | OverlayMessage::FrameTimes(_)
                )
            })
            .cloned()
            .collect()
    }

    #[test]
    fn editor_without_target_gets_synthetic_running_metrics() {
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&settings(false), Lang::En, None);
        c.on_catalog(builtins());
        c.on_service(true);
        c.on_editor(true);
        let out = c.step(60_000);
        assert!(out.link.is_empty(), "the frame engine stays off");
        assert!(out.overlay.is_empty());
        assert_eq!(out.status.unwrap().frames, frames_state::OFF);
        let data = out.editor_data.expect("editor data at once");
        assert_eq!(data.metrics.state, frames_state::RUNNING);
        let fps = data.metrics.fps_displayed.expect("displayed FPS");
        assert!((fps - 144.0).abs() < 5.0, "{fps}");
        assert_eq!(data.metrics.fg_multiplier.map(f64::round), Some(2.0));
        assert!(!data.frame_times.is_empty());
        // Frame times at 10 Hz (new ones only), metrics at textHz (2 Hz).
        let (mut with_metrics, mut with_times) = (0, 0);
        for i in 1..=20 {
            if let Some(d) = c.step(60_000 + i * 100).editor_data {
                with_metrics += 1;
                with_times += usize::from(!d.frame_times.is_empty());
            }
        }
        assert!(
            with_metrics >= 4 && with_times >= 2,
            "{with_metrics} {with_times}"
        );
        // The in-game overlay never gets synthetic frames.
        c.on_settings(&settings(true), Lang::En, None);
        c.on_host(HostState::Running);
        for now in [63_000, 63_500, 64_000] {
            let out = c.step(now);
            assert!(data_of(&out.overlay).is_empty(), "{now}");
            assert!(out.editor_data.is_some(), "{now}");
        }
    }

    #[test]
    fn editor_with_target_gets_the_target_metrics() {
        let mut c = showing();
        c.on_editor(true);
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 5.0, 101, 0)],
        );
        let out = c.step(600);
        let data = out.editor_data.clone().expect("editor data");
        assert_eq!(data.metrics.state, "running");
        assert_eq!(data.metrics.fps_displayed, Some(100.0));
        assert_eq!(data.frame_times.len(), 101);
        assert!((data.frame_times[0].t_s - 5.0).abs() < 1e-6);
        // The overlay gets the same data.
        assert_eq!(metrics(&out), vec![data.metrics]);
    }

    #[test]
    fn editor_closed_sends_no_editor_data() {
        let mut c = showing();
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 5.0, 50, 0)],
        );
        for now in [200, 600, 1_100] {
            assert!(c.step(now).editor_data.is_none());
        }
        c.on_editor(true);
        assert!(c.step(1_600).editor_data.is_some());
        c.on_editor(false);
        for now in [1_700, 2_200, 3_000] {
            assert!(c.step(now).editor_data.is_none());
        }
        // Without a target either.
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&settings(false), Lang::En, None);
        for now in [0, 5_000, 10_000] {
            assert!(c.step(now).editor_data.is_none());
        }
    }

    /// A profile with a `low-1` block over 30 s, percentile.
    fn lows_profile() -> Profile {
        oma_core::overlay::parse_profile(
            &serde_json::json!({ "format": 1, "name": "l", "blocks": [{
                "id": "a", "rect": { "x": 0, "y": 0, "w": 4, "h": 2 },
                "source": { "frames": "low-1" }, "kind": "text",
                "stat": { "window": 30, "definition": "percentile" }
            }] })
            .to_string(),
        )
        .unwrap()
    }

    fn low_keys(m: &FrameMetrics) -> Vec<(u32, String)> {
        m.lows
            .iter()
            .map(|l| (l.window_s, l.definition.clone()))
            .collect()
    }

    #[test]
    fn preview_does_not_change_the_in_game_overlay() {
        let (mut plain, mut with_preview) = (showing(), showing());
        with_preview.on_editor(true);
        with_preview.set_editor_profile(Some(lows_profile()));
        with_preview.set_preview(Some(lows_profile()));
        with_preview.on_preview_host(HostState::Running);
        let mut preview_lows = false;
        let mut overlay_frames = BTreeSet::new();
        for (i, now) in (2..60u64).map(|i| i * 100).enumerate() {
            match now {
                // Hidden for a while, then shown: the backlog reaches it.
                1_000 | 2_500 => {
                    plain.toggle_hidden();
                    with_preview.toggle_hidden();
                }
                // A new overlay process.
                3_500 => {
                    plain.on_host(HostState::Starting);
                    with_preview.on_host(HostState::Starting);
                }
                3_700 => {
                    plain.on_host(HostState::Running);
                    with_preview.on_host(HostState::Running);
                }
                _ => {}
            }
            let batches = [batch(GAME, 5.0 + i as f64 * 0.1, 10, 0)];
            for c in [&mut plain, &mut with_preview] {
                c.on_frames(Some(&status("running")), Some(&processes()), &batches);
            }
            let (a, b) = (plain.step(now), with_preview.step(now));
            assert_eq!(a.overlay, b.overlay, "{now}");
            for t in times(&a) {
                overlay_frames.extend(t.frames.iter().map(|f| f.t_s.to_bits()));
            }
            for m in metrics(&a) {
                assert!(
                    low_keys(&m).iter().all(|k| k.0 == 10),
                    "{now}: {:?}",
                    m.lows
                );
            }
            for m in b.preview.iter().filter_map(|m| match m {
                OverlayMessage::FrameMetrics(f) => Some(f),
                _ => None,
            }) {
                preview_lows |= low_keys(m).contains(&(30, "percentile".into()));
            }
        }
        assert!(preview_lows, "the preview gets its own lows");
        // A new process gets the window again; no frame is ever missed.
        assert_eq!(overlay_frames.len(), 58 * 10, "the overlay misses no frame");
    }

    #[test]
    fn opening_the_editor_sends_metrics_at_once() {
        let mut c = showing();
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 5.0, 50, 0)],
        );
        assert_eq!(metrics(&c.step(150)).len(), 0, "sent at 100");
        c.on_editor(true);
        let out = c.step(200);
        assert!(out.editor_data.is_some());
        assert!(metrics(&out).is_empty(), "the overlay keeps its cadence");
    }

    #[test]
    fn editor_profile_lows_reach_the_canvas() {
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&settings(false), Lang::En, None);
        c.on_catalog(builtins());
        c.on_editor(true);
        c.set_editor_profile(Some(lows_profile()));
        let out = c.step(60_000);
        let data = out.editor_data.unwrap();
        assert!(low_keys(&data.metrics).contains(&(30, "percentile".into())));
        assert_eq!(out.values_plan.map(|p| p.wanted), Some(false));
        c.set_editor_profile(None);
        let data = c.step(60_500).editor_data.unwrap();
        assert!(low_keys(&data.metrics).iter().all(|k| k.0 == 10));
    }

    #[test]
    fn a_stop_we_asked_for_keeps_a_new_preview() {
        let mut c = showing();
        c.on_editor(true);
        c.set_preview(Some(sensor_profile("cpu/0/load/total")));
        c.on_preview_host(HostState::Starting);
        c.step(200);
        c.on_preview_host(HostState::Running);
        c.step(300);
        c.set_preview(None);
        assert!(!c.step(400).want_preview);
        c.set_preview(Some(sensor_profile("gpu0/temperature/core")));
        // The stop of the first preview arrives late.
        c.on_preview_host(HostState::Off);
        let out = c.step(500);
        assert!(out.want_preview, "the new preview is kept");
        assert!(out.status.is_none_or(|s| s.preview));
        assert!(c.current_status().preview);
    }

    #[test]
    fn preview_gets_its_own_profile_and_shared_data() {
        let mut c = showing();
        c.on_editor(true);
        c.set_preview(Some(sensor_profile("cpu/0/load/total")));
        let out = c.step(200);
        assert!(out.want_preview);
        assert!(out.preview.is_empty(), "nothing before the preview runs");
        assert!(out.status.unwrap().preview);
        c.on_preview_host(HostState::Running);
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 5.0, 101, 0)],
        );
        let out = c.step(600);
        let profiles = preview_profiles(&out);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].profile_id, "preview");
        assert_eq!(profiles[0].strings["previewTitle"], "Overlay preview");
        assert!(matches!(out.preview[0], OverlayMessage::SetProfile(_)));
        assert!(
            !out.preview
                .iter()
                .any(|m| matches!(m, OverlayMessage::SetPlacement(_))),
            "the preview ignores placements"
        );
        // The same frames, on the preview's own cadence.
        assert_eq!(times(&out).len(), 1);
        let preview_times: Vec<_> = out
            .preview
            .iter()
            .filter(|m| matches!(m, OverlayMessage::FrameTimes(_)))
            .cloned()
            .collect();
        assert_eq!(
            preview_times,
            vec![OverlayMessage::FrameTimes(times(&out)[0].clone())]
        );
        let out = c.step(700);
        let preview_metrics: Vec<_> = out
            .preview
            .iter()
            .filter_map(|m| match m {
                OverlayMessage::FrameMetrics(f) => Some(f.fps_displayed),
                _ => None,
            })
            .collect();
        assert_eq!(preview_metrics, vec![Some(100.0)]);
        // Not sent again while nothing changes; again after a restart.
        assert!(preview_profiles(&c.step(700)).is_empty());
        c.on_preview_host(HostState::Starting);
        assert!(c.step(800).preview.is_empty());
        c.on_preview_host(HostState::Running);
        assert_eq!(preview_profiles(&c.step(900)).len(), 1);
        // Closed by the editor.
        c.set_preview(None);
        let out = c.step(1_000);
        assert!(!out.want_preview && out.preview.is_empty());
    }

    /// A catalog with one user profile reading `sensor`, and its id.
    fn catalog_with(sensor: &str) -> (ProfileCatalog, String) {
        let dir =
            std::env::temp_dir().join(format!("oma-controller-tests-union-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let id = "00000000-0000-4000-8000-0000000000c1";
        let json = serde_json::to_string(&sensor_profile(sensor)).unwrap();
        std::fs::write(dir.join(format!("{id}.json")), json).unwrap();
        let catalog = load_catalog(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        (catalog, id.to_owned())
    }

    #[test]
    fn values_plan_is_the_union_of_active_and_preview() {
        let (catalog, id) = catalog_with("gpu0/temperature/core");
        let mut s = settings(false);
        s.overlay.default_profile = id;
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&s, Lang::En, None);
        c.on_catalog(catalog);
        let plan = c.step(0).values_plan.unwrap();
        assert_eq!(plan.used, vec!["gpu0/temperature/core"]);
        assert!(!plan.wanted);
        // The editor alone does not need values (its canvas reads the
        // LiveStore), nor do the sensors of the profile being edited.
        c.on_editor(true);
        c.set_editor_profile(Some(sensor_profile("cpu/1/load/total")));
        assert_eq!(c.step(100).values_plan, None);
        c.set_preview(Some(sensor_profile("cpu/0/load/total")));
        let plan = c.step(200).values_plan.unwrap();
        assert_eq!(plan.used, vec!["cpu/0/load/total", "gpu0/temperature/core"]);
        assert!(!plan.wanted, "the preview is not running yet");
        c.on_preview_host(HostState::Running);
        assert_eq!(
            c.step(300).values_plan,
            Some(ValuesPlan {
                used: vec!["cpu/0/load/total".into(), "gpu0/temperature/core".into()],
                wanted: true
            })
        );
        c.set_preview(None);
        assert_eq!(
            c.step(400).values_plan,
            Some(ValuesPlan {
                used: vec!["gpu0/temperature/core".into()],
                wanted: false
            })
        );
    }

    #[test]
    fn preview_closed_by_the_user_is_dropped() {
        let mut c = showing();
        c.on_editor(true);
        c.set_preview(Some(sensor_profile("cpu/0/load/total")));
        // Not started yet: an `Off` is not a close.
        c.on_preview_host(HostState::Off);
        assert!(c.step(200).want_preview);
        c.on_preview_host(HostState::Starting);
        c.step(300);
        c.on_preview_host(HostState::Running);
        assert!(c.step(400).want_preview);
        // The user closed the window: the host is off.
        c.on_preview_host(HostState::Off);
        let out = c.step(500);
        assert!(!out.want_preview);
        assert!(out.preview.is_empty());
        assert!(!out.status.unwrap().preview);
        assert!(!c.step(600).want_preview, "not restarted");
    }

    #[test]
    fn use_now_lasts_until_target_change() {
        let mut c = showing();
        c.use_now("builtin-bar".into());
        assert_eq!(profiles_sent(&c.step(200)), vec!["builtin-bar"]);
        assert_eq!(
            c.current_status().active_profile.as_deref(),
            Some("builtin-bar")
        );
        // A short alt-tab keeps it.
        c.on_foreground(BROWSER_FG);
        assert!(profiles_sent(&c.step(300)).is_empty());
        c.on_foreground(GAME_FG);
        assert!(profiles_sent(&c.step(400)).is_empty());
        // Another game: the settings' profile again.
        c.on_frames(Some(&status("running")), Some(&other_only()), &[]);
        c.on_foreground(Foreground {
            pid: OTHER,
            hwnd: 0x2000,
        });
        let out = c.step(500);
        assert_eq!(targets(&out), vec![Some(OTHER)]);
        assert_eq!(profiles_sent(&out), vec!["builtin-gaming"]);
    }

    #[test]
    fn values_follow_the_plan() {
        let schema = Schema {
            revision: 1,
            devices: vec![],
            sensors: vec![Sensor::new(
                "cpu/0",
                SensorKind::Load,
                "total",
                Unit::Percent,
                Label::new("cpu.load.total"),
                SensorSource::Mock,
            )],
        };
        let snapshot = Snapshot {
            revision: 1,
            seq: 1,
            timestamp_ms: 0,
            values: vec![Some(42.0)],
        };
        let used = vec!["cpu/0/load/total".to_owned()];
        let plan = ValuesPlan {
            used: used.clone(),
            wanted: true,
        };
        let msg = tick_values(&plan, &schema, &snapshot, &[Quality::Fresh], 5);
        let Some(OverlayMessage::Values(Values { at_ms, values })) = msg else {
            panic!("{msg:?}");
        };
        assert_eq!((at_ms, values.len(), values[0].value), (5, 1, Some(42.0)));
        let hidden = ValuesPlan {
            used,
            wanted: false,
        };
        assert_eq!(tick_values(&hidden, &schema, &snapshot, &[], 5), None);
        assert_eq!(
            tick_values(&ValuesPlan::default(), &schema, &snapshot, &[], 5),
            None
        );
    }
    // --- Benchmark (M7d, D12) ---------------------------------------------

    fn fixed_clock() -> oma_core::csv::LocalTime {
        // 2026-10-05 21:30:00 UTC
        oma_core::csv::local_time(1_791_235_800_000, 0)
    }

    /// Like [`batch`], on another swapchain.
    fn batch_on(pid: u32, swapchain: u64, first_s: f64, count: u32) -> FrameBatch {
        let mut b = batch(pid, first_s, count, 0);
        for f in &mut b.frames {
            f.swapchain = swapchain;
        }
        b
    }

    fn bench_messages(out: &Outputs) -> Vec<BenchmarkOverlay> {
        out.overlay
            .iter()
            .filter_map(|m| match m {
                OverlayMessage::Benchmark(b) => Some(b.clone()),
                _ => None,
            })
            .collect()
    }

    fn rows(out: &Outputs) -> usize {
        out.benchmark
            .iter()
            .map(|c| match c {
                BenchmarkCommand::Rows(r) => r.len(),
                _ => 0,
            })
            .sum()
    }

    fn finished(out: &Outputs) -> Vec<Option<BenchmarkRecord>> {
        out.benchmark
            .iter()
            .filter_map(|c| match c {
                BenchmarkCommand::Finish { record } => Some(record.clone()),
                _ => None,
            })
            .collect()
    }

    /// `c` with a capture started at `now_ms`, its `Begin` step done.
    fn recording(mut c: Controller, now_ms: u64) -> Controller {
        c.clock = fixed_clock;
        c.toggle_benchmark(now_ms);
        let out = c.step(now_ms);
        assert_eq!(
            out.benchmark,
            vec![BenchmarkCommand::Begin {
                stem: "my_game-20261005-213000".into()
            }]
        );
        c
    }

    #[test]
    fn benchmark_without_target_toasts() {
        let mut c = enabled_with(&settings(true));
        c.step(0);
        c.toggle_benchmark(10);
        let out = c.step(100);
        assert_eq!(out.toast, Some(ToastRequest::BenchmarkNoTarget));
        assert!(out.benchmark.is_empty());
        assert_eq!(c.current_status().benchmark.state, "idle");
        // A target with the overlay off (`OMA_FRAMES_DEBUG`): still no
        // capture (DD6).
        let mut c = Controller::new(OWN, FREQ);
        c.on_settings(&settings(false), Lang::En, Some(config(true, false, false)));
        c.on_catalog(builtins());
        c.on_service(true);
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        c.step(0);
        assert_eq!(c.current_status().target.map(|t| t.pid), Some(GAME));
        c.toggle_benchmark(50);
        let out = c.step(100);
        assert_eq!(out.toast, Some(ToastRequest::BenchmarkNoTarget));
        assert!(out.benchmark.is_empty());
    }

    #[test]
    fn benchmark_records_only_the_target_main_swapchain() {
        let mut c = recording(showing(), 100);
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[
                batch(GAME, 1.0, 20, 0),
                batch_on(GAME, 0xdef, 1.0, 2),
                batch(OTHER, 1.0, 5, 0),
            ],
        );
        let out = c.step(200);
        assert_eq!(rows(&out), 20);
        let bench = c.current_status().benchmark;
        assert_eq!(bench.state, "recording");
        assert_eq!(bench.game.as_deref(), Some("my game.exe"));
        // Another target: its frames are not the recorded game's, and ten
        // seconds without the game end the capture.
        c.on_frames(Some(&status("running")), Some(&other_only()), &[]);
        c.on_foreground(Foreground {
            pid: OTHER,
            hwnd: 0x2000,
        });
        let out = c.step(300);
        assert_eq!(targets(&out), vec![Some(OTHER)]);
        c.on_frames(
            Some(&status("running")),
            Some(&other_only()),
            &[batch(OTHER, 2.0, 5, 0)],
        );
        assert_eq!(rows(&c.step(400)), 0);
        assert!(finished(&c.step(10_299)).is_empty());
        let ended = finished(&c.step(10_300));
        assert_eq!(ended.len(), 1);
        let record = ended[0].clone().expect("frames were recorded");
        assert_eq!(record.end_reason, EndReason::NoTarget);
        assert_eq!(record.summary.frames_displayed, 20);
    }

    #[test]
    fn benchmark_rec_seconds_go_to_the_overlay() {
        let mut c = showing();
        c.clock = fixed_clock;
        c.toggle_benchmark(100);
        assert_eq!(
            bench_messages(&c.step(100)),
            vec![BenchmarkOverlay {
                recording_s: Some(0),
                summary: None
            }]
        );
        assert!(bench_messages(&c.step(200)).is_empty());
        assert!(bench_messages(&c.step(1_000)).is_empty());
        assert_eq!(
            bench_messages(&c.step(1_100)),
            vec![BenchmarkOverlay {
                recording_s: Some(1),
                summary: None
            }]
        );
        assert_eq!(
            c.current_status().benchmark,
            BenchmarkStatus {
                state: "recording".into(),
                game: Some("my game.exe".into()),
                elapsed_s: Some(1),
                error: None,
            }
        );
        // A new overlay process gets the badge again.
        c.on_host(HostState::Starting);
        c.step(1_200);
        c.on_host(HostState::Running);
        assert_eq!(
            bench_messages(&c.step(1_300)),
            vec![BenchmarkOverlay {
                recording_s: Some(1),
                summary: None
            }]
        );
    }

    #[test]
    fn summary_shows_for_ten_seconds_then_clears() {
        let mut c = recording(showing(), 100);
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 1.0, 20, 0)],
        );
        c.step(200);
        c.toggle_benchmark(1_000);
        let out = c.step(1_000);
        let ended = finished(&out);
        let record = ended[0].clone().expect("a summary");
        assert_eq!(record.end_reason, EndReason::User);
        assert_eq!(record.game, "my game.exe");
        assert_eq!(record.started_at, "2026-10-05T21:30:00");
        let shown = bench_messages(&out);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].recording_s, None);
        let summary = shown[0].summary.clone().expect("the summary box");
        assert!((summary.fps_displayed - 100.0).abs() < 1e-6, "{summary:?}");
        assert_eq!(c.current_status().benchmark.state, "idle");
        assert!(bench_messages(&c.step(1_000 + SUMMARY_SHOW_MS - 1)).is_empty());
        assert_eq!(
            bench_messages(&c.step(1_000 + SUMMARY_SHOW_MS)),
            vec![BenchmarkOverlay {
                recording_s: None,
                summary: None
            }]
        );
    }

    #[test]
    fn blocked_game_is_recorded_without_badge() {
        let mut s = settings(true);
        s.overlay.blocked_games = vec!["my game.exe".into()];
        let mut c = enabled_with(&s);
        c.on_frames(Some(&status("running")), Some(&processes()), &[]);
        c.on_foreground(GAME_FG);
        c.step(0);
        c.on_geometry(Some(geometry(96)));
        c.step(100);
        let mut c = recording(c, 100);
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 1.0, 20, 0)],
        );
        let out = c.step(200);
        assert_eq!(rows(&out), 20);
        assert!(bench_messages(&out).is_empty());
        assert!(bench_messages(&c.step(1_500)).is_empty());
        assert_eq!(c.current_status().benchmark.state, "recording");
        c.toggle_benchmark(2_000);
        let out = c.step(2_000);
        assert!(finished(&out)[0].is_some());
        assert!(bench_messages(&out).is_empty(), "no summary box either");
    }

    #[test]
    fn write_error_stops_with_the_reason() {
        let mut c = recording(showing(), 100);
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 1.0, 20, 0)],
        );
        c.step(200);
        c.on_benchmark_error(WriteFailure::DiskFull);
        let out = c.step(300);
        let ended = finished(&out);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].as_ref().unwrap().end_reason, EndReason::Error);
        let error = LogError {
            key: "log.error.diskFull".into(),
            detail: None,
        };
        assert_eq!(
            out.toast,
            Some(ToastRequest::BenchmarkError {
                error: error.clone()
            })
        );
        let bench = c.current_status().benchmark;
        assert_eq!(bench.state, "error");
        assert_eq!(bench.error, Some(error));
        assert_eq!(bench.game, None);
        // The summary that then cannot be written: no second toast.
        c.on_benchmark_error(WriteFailure::DiskFull);
        assert_eq!(c.step(400).toast, None);
        // A new capture clears the error.
        c.toggle_benchmark(500);
        c.step(500);
        let bench = c.current_status().benchmark;
        assert_eq!((bench.state.as_str(), bench.error), ("recording", None));
    }

    #[test]
    fn shutdown_ends_the_capture_with_its_reason() {
        let mut c = recording(showing(), 100);
        c.on_frames(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 1.0, 20, 0)],
        );
        c.step(200);
        let commands = c.shutdown_benchmark(300);
        assert!(!c.recording());
        let [BenchmarkCommand::Finish { record: Some(r) }] = commands.as_slice() else {
            panic!("{commands:?}");
        };
        assert_eq!(r.end_reason, EndReason::Shutdown);
        // Nothing running: nothing to do.
        assert!(c.shutdown_benchmark(400).is_empty());
    }
}
