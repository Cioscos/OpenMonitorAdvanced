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
//!   foreground PID. A new target activates `gameProfiles[exe]`, else
//!   `defaultProfile`, and ends a `next_profile` choice.
//!   Without frame data (service down or incompatible, engine not running)
//!   the target is kept while its window exists, and a foreground process
//!   whose last known name has a `gameProfiles` entry can become one, so
//!   sensor blocks keep working (spec §9).
//! - **Visibility (DP9–DP11):** the overlay shows only while it is on, its
//!   process runs, the target's window is the foreground one, visible and
//!   not minimized, the game is not in `blockedGames` and the user has not
//!   hidden it. `SetPlacement` is sent only when it changes.
//! - **Data:** `SetProfile` on a change of profile, drawing settings or
//!   schema, and after every new `Running`; `FrameMetrics` at `textHz`,
//!   `FrameTimes` at 10 Hz with the new frames only, sensor values through
//!   [`ValuesPlan`]; no data while the overlay is hidden.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use oma_core::frames::metrics::LowDefinition;
use oma_core::frames::{pick_swapchain, read, FrameReadout, FrameWindow, LOWS_WINDOW_S};
use oma_core::model::{Schema, Snapshot};
use oma_core::overlay::{Foreground, Profile, WindowGeometry};
use oma_core::provider::Quality;
use oma_core::settings::{Attach, Settings};
use oma_ipc::overlay::{OverlayMessage, PxArea, SetPlacement};
use oma_ipc::{
    frames_state, FrameBatch, FramesConfigure, FramesStatus, PresentingProcess, PresentingProcesses,
};
use oma_win::svc::LinkCommand;
use serde::Serialize;

use super::forward::{
    frame_times_since, low_windows, metrics_message, set_profile, used_sensors, values_message,
};
use super::frames::{detail_label, line, sample_of, state_label, LineContext};
use super::host::{HostFailure, HostState};
use super::profiles::{ProfileCatalog, ProfileDiagnostic, ProfileEntry};
use super::target::{ProcessInfo, TargetPicker, OWN_PROCESS_NAMES, SYSTEM_EXCLUDED};
use crate::i18n::Lang;
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
/// `FrameMetrics.state` and `OverlayStatus.frames` without the service.
pub const FRAMES_UNAVAILABLE: &str = "unavailable";

/// A notification the app should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastRequest {
    /// The target runs in exclusive fullscreen, where the overlay cannot
    /// show: once per executable and per app session (DP16).
    ExclusiveFullscreen { exe: String },
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
    /// Watch this window's geometry (`Some(None)`: stop watching).
    pub track: Option<Option<Foreground>>,
    pub toast: Option<ToastRequest>,
    /// The new status, only when it changed.
    pub status: Option<OverlayStatus>,
    /// A `frames:` line to log (`OMA_FRAMES_DEBUG` only).
    pub diagnostics_line: Option<String>,
    /// «Retry» on a failed overlay process: call `OverlayHost::retry`.
    pub retry_host: bool,
    /// The new plan for the sampler, only when it changed.
    pub values_plan: Option<ValuesPlan>,
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
    metrics_sent_ms: Option<u64>,
    times_sent_ms: Option<u64>,
    /// The newest frame time sent in `FrameTimes`.
    times_after_s: f64,
    line_ms: u64,
    /// Lowercase executables already toasted.
    toasted: BTreeSet<String>,
    sent_status: Option<OverlayStatus>,
    sent_plan: Option<ValuesPlan>,
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
            sent_config: None,
            config_sent_ms: 0,
            retry_frames: false,
            retry_host: false,
            sent_track: None,
            was_running: false,
            sent_profile: None,
            sent_placement: None,
            shown: false,
            metrics_sent_ms: None,
            times_sent_ms: None,
            times_after_s: f64::NEG_INFINITY,
            line_ms: 0,
            toasted: BTreeSet::new(),
            sent_status: None,
            sent_plan: None,
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
            self.settings = settings.clone();
            self.lang = lang;
            self.choice_dirty = true;
            self.profile_dirty = true;
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

    pub fn on_foreground(&mut self, fg: Foreground) {
        self.foreground = Some(fg);
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
        self.choice = Some(self.catalog.next_after(&current));
        self.choice_dirty = true;
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
            retry_host: std::mem::take(&mut self.retry_host),
            ..Outputs::default()
        };
        let config = self.wanted_config();
        self.step_config(&config, now_ms, &mut out);
        self.step_target(config.enabled, now_ms, &mut out);
        self.step_resend(&config, now_ms, &mut out);
        self.step_track(&mut out);
        self.refresh_profile();
        self.step_overlay(&config, now_ms, &mut out);
        out.toast = self.toast();

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
        let plan_changed = self
            .sent_plan
            .as_ref()
            .is_none_or(|p| p.wanted != self.shown || p.used != self.used);
        if plan_changed {
            let plan = self.values_plan();
            self.sent_plan = Some(plan.clone());
            out.values_plan = Some(plan);
        }
        out
    }

    /// The sampler's plan now (see [`tick_values`]).
    pub fn values_plan(&self) -> ValuesPlan {
        ValuesPlan {
            used: self.used.clone(),
            wanted: self.shown,
        }
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
            frames_detail: self.status.as_ref().and_then(|s| s.detail.clone()),
            target: self.target.as_ref().map(|t| TargetStatus {
                name: t.name.clone(),
                pid: t.pid,
            }),
            active_profile: self.active.as_ref().map(|(id, _)| id.clone()),
            profiles: self.catalog.entries.clone(),
            diagnostics: self.catalog.diagnostics.clone(),
            hidden_by_user: self.hidden_by_user,
            hotkeys: self.hotkeys.clone(),
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
            self.hold_target(out);
        }
        for batch in std::mem::take(&mut self.batches) {
            self.dropped = self.dropped.saturating_add(u64::from(batch.dropped));
            // A batch of the previous target may still arrive after a change.
            if self.qpc_frequency == 0 || Some(batch.pid) != self.target.as_ref().map(|t| t.pid) {
                continue;
            }
            for frame in &batch.frames {
                self.window.push(sample_of(frame, self.qpc_frequency));
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
            self.picker.on_foreground(fg.pid, now_ms);
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

    /// Without frame data (service down or incompatible, engine not
    /// running) sensor blocks must keep working (spec §9): the target is
    /// kept, without the grace limit, while its window exists; without one,
    /// the foreground process becomes the target if its name, known from the
    /// last process list, has a profile in `gameProfiles`. Without any list
    /// (service down since the start) no game is recognised. Any other
    /// window (a browser) is never a target.
    fn hold_target(&mut self, out: &mut Outputs) {
        if self.target.is_some() {
            if !self.target_window_alive() {
                self.set_target(None);
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
        self.set_target(Some(ProcessInfo {
            pid: fg.pid,
            name: name.clone(),
            displayed_fps: 0.0,
        }));
        out.link.push(LinkCommand::SetFramesTarget(Some(fg.pid)));
    }

    /// The target's window is watched and has not been reported gone.
    fn target_window_alive(&self) -> bool {
        self.sent_track.is_some() && !(self.geometry_seen && self.geometry.is_none())
    }

    /// The frame engine gives data (or is about to).
    fn frames_available(&self) -> bool {
        matches!(
            self.frames_state(),
            frames_state::RUNNING | frames_state::STARTING
        )
    }

    fn set_target(&mut self, target: Option<ProcessInfo>) {
        self.tracked = target
            .as_ref()
            .and_then(|t| self.foreground.filter(|fg| fg.pid == t.pid));
        self.target = target;
        self.window.clear();
        self.swapchain = None;
        self.times_after_s = f64::NEG_INFINITY;
        self.choice = None;
        self.choice_dirty = true;
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
        self.lows = low_windows(&profile);
        self.used = used_sensors(&profile);
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
        self.active = Some((id, profile));
        self.profile_dirty = true;
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
        if fg.pid != target.pid || self.tracked != Some(fg) {
            return None;
        }
        let g = self.geometry.filter(|g| g.visible && !g.minimized)?;
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
        let exe = target.name.to_lowercase();
        self.settings.overlay.blocked_games.contains(&exe)
    }

    /// `SetProfile`, `SetPlacement` and the data, while the overlay runs.
    fn step_overlay(&mut self, config: &FramesConfigure, now_ms: u64, out: &mut Outputs) {
        let running = self.host == HostState::Running;
        if running && !self.was_running {
            // A new overlay process knows nothing yet.
            self.sent_profile = None;
            self.sent_placement = None;
            self.profile_dirty = true;
            self.times_after_s = f64::NEG_INFINITY;
            self.metrics_sent_ms = None;
            self.times_sent_ms = None;
        }
        self.was_running = running;
        let placement = self.placement();
        self.shown = placement.is_some();
        if !running {
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
        if self.sent_placement != Some(placement) {
            self.sent_placement = Some(placement);
            out.overlay.push(OverlayMessage::SetPlacement(SetPlacement {
                area: placement.map(|(area, _)| area),
                dpi: placement.map_or(DEFAULT_DPI, |(_, dpi)| dpi),
            }));
        }
        if !self.shown {
            self.metrics_sent_ms = None;
            self.times_sent_ms = None;
            return;
        }
        let text_ms = 1_000 / u64::from(self.settings.overlay.text_hz.max(1));
        if self
            .metrics_sent_ms
            .is_none_or(|t| now_ms.saturating_sub(t) >= text_ms)
        {
            self.metrics_sent_ms = Some(now_ms);
            let msg = if self.connected {
                let readout = self.readout(config.track_gpu);
                metrics_message(Some(&readout), self.frames_state())
            } else {
                metrics_message(None, FRAMES_UNAVAILABLE)
            };
            out.overlay.push(msg);
        }
        if self
            .times_sent_ms
            .is_none_or(|t| now_ms.saturating_sub(t) >= FRAME_TIMES_MS)
        {
            self.times_sent_ms = Some(now_ms);
            if self.swapchain.is_none() {
                self.swapchain = pick_swapchain(&self.window.last(LOWS_WINDOW_S));
            }
            let (msg, newest) = frame_times_since(&self.window, self.swapchain, self.times_after_s);
            self.times_after_s = newest;
            if matches!(&msg, OverlayMessage::FrameTimes(t) if !t.frames.is_empty()) {
                out.overlay.push(msg);
            }
        }
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

    /// The frame engine's state for the overlay and the UI.
    fn frames_state(&self) -> &str {
        if !self.connected {
            return FRAMES_UNAVAILABLE;
        }
        match &self.status {
            Some(status) => state_label(Some(status)),
            None if self.wanted_config().enabled => frames_state::STARTING,
            None => frames_state::OFF,
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
    use oma_core::overlay::PxRect;
    use oma_ipc::overlay::{FrameMetrics, FrameTimes, SetProfile, Values};
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
        // Another game without a profile of its own: back to the default.
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
        // Another game: the choice ends.
        c.on_foreground(Foreground {
            pid: OTHER,
            hwnd: 0x2000,
        });
        let out = c.step(600);
        assert_eq!(targets(&out), vec![Some(OTHER)]);
        assert_eq!(profiles_sent(&out), vec!["builtin-gaming"]);
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
        // Elsewhere until the target drops, then back: no second toast.
        c.on_foreground(BROWSER_FG);
        c.step(200);
        c.step(3_300);
        assert_eq!(c.current_status().target, None);
        c.on_foreground(GAME_FG);
        assert_eq!(c.step(3_400).toast, None);
        // Another game in exclusive fullscreen gets its own.
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
}
