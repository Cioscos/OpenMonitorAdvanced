//! The `oma-overlay-ctl` thread: it owns the overlay [`Controller`], the
//! [`OverlayHost`] that runs `oma-overlay.exe` and the
//! [`ForegroundWatcher`], and carries out the controller's [`Outputs`].
//!
//! - **Inputs** arrive on an unbounded channel and none is dropped: settings,
//!   foreground changes, window moves, the host's state, the schema, the UI
//!   commands and the hotkeys. Moves are coalesced: the watcher's callback
//!   raises a flag and sends one wake until the geometry has been read again.
//! - **Steps:** while the frame engine is wanted (the overlay is on, or
//!   `OMA_FRAMES_DEBUG` is set) the thread drains the [`FramesFeed`] and calls
//!   [`Controller::step`] every 100 ms, and after every input. Otherwise no
//!   watcher and no process run, and the thread waits on the channel without
//!   a timeout, stepping once per input (§11).
//! - **Sampler:** [`OverlayHandle::on_tick`] builds `Values` from the plan the
//!   controller last published (ruling R2) and queues them for the overlay
//!   without ever blocking the tick.
//! - **Shutdown** ([`OverlayRunner::stop`]): the engine is turned off if the
//!   controller had turned it on, then the host closes the overlay, then the
//!   watcher goes.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oma_core::engine::TickOutput;
use oma_core::model::Schema;
use oma_core::overlay::{Foreground, PxRect};
use oma_core::settings::Settings;
use oma_ipc::overlay::OverlayMessage;
use oma_ipc::FramesConfigure;
use oma_win::foreground::{
    current_foreground, window_geometry, window_monitor, ForegroundEvent, ForegroundWatcher,
};
use oma_win::svc::{FramesFeed, LinkCommand};
use tauri::State;

use super::controller::{
    tick_values, Controller, Outputs, OverlayHotkeys, OverlayStatus, ToastRequest, ValuesPlan,
};
use super::frames::{options_from_env, ENV_VAR};
use super::host::{overlay_exe, HostFailure, HostState, OverlayHost, OverlaySender};
use super::profiles::{load_catalog, profiles_dir, ProfileCatalog};
use crate::hotkeys::OverlayActions;
use crate::i18n::{t, Lang};
use crate::log::HotkeyStatus;
use crate::notifier::{launch_for_main, ToastSink};
use crate::settings::SettingsStore;
use crate::tray::language_for;

/// The UI event carrying every new [`OverlayStatus`].
pub const EVENT_OVERLAY_STATUS: &str = "overlay-status";
/// The controller's period while the frame engine is wanted.
const STEP: Duration = Duration::from_millis(100);
/// How often the tracked window's geometry is read without a move, so a
/// closed window is noticed.
const GEOMETRY_REFRESH_MS: u64 = 1_000;
/// How often the foreground window is checked against Windows.
const FOREGROUND_CHECK_MS: u64 = 1_000;

/// Sends a command to the service link without blocking
/// ([`crate::service::ServiceShell::link_commands`]).
pub type LinkSink = Box<dyn Fn(LinkCommand) + Send + Sync>;
/// Receives every new status, on the controller's thread; must not block.
pub type StatusSink = Box<dyn Fn(&OverlayStatus) + Send>;

/// What the controller's thread reacts to.
pub(crate) enum Input {
    Settings(Arc<Settings>),
    Foreground(Foreground),
    /// The tracked window moved (coalesced, see [`MoveCoalescer`]).
    Moved,
    Host(HostState),
    Schema(Arc<Schema>),
    Retry,
    ReloadProfiles,
    SetHidden(bool),
    ToggleHidden,
    NextProfile,
    Hotkeys(OverlayHotkeys),
    Shutdown,
}

/// Coalesces the tracked window's moves: one wake until the geometry has
/// been read again.
#[derive(Default)]
pub(crate) struct MoveCoalescer(AtomicBool);

impl MoveCoalescer {
    /// A move: `true` when the thread must be woken.
    fn mark(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }

    /// Whether a move is pending; the caller then reads the geometry.
    pub(crate) fn take(&self) -> bool {
        self.0.swap(false, Ordering::AcqRel)
    }
}

/// The watcher's sink: it runs on the `oma-foreground` thread, so it only
/// sends and never blocks.
pub(crate) fn watcher_sink(
    tx: Sender<Input>,
    moves: Arc<MoveCoalescer>,
) -> Box<dyn Fn(ForegroundEvent) + Send> {
    Box::new(move |event| match event {
        ForegroundEvent::Foreground(fg) => {
            let _ = tx.send(Input::Foreground(fg));
        }
        ForegroundEvent::Moved { .. } => {
            if moves.mark() {
                let _ = tx.send(Input::Moved);
            }
        }
    })
}

/// What the sampler needs (ruling R2): the controller's plan and the way to
/// the overlay, while there is one.
#[derive(Default)]
struct Tap {
    plan: ValuesPlan,
    sender: Option<OverlaySender>,
}

/// The side of the controller the rest of the app talks to: the sampler,
/// the Tauri commands, the hotkeys and the tray.
#[derive(Clone)]
pub struct OverlayHandle {
    tx: Sender<Input>,
    tap: Arc<Mutex<Tap>>,
    status: Arc<Mutex<OverlayStatus>>,
}

impl OverlayHandle {
    /// Called on every sampler tick with the latest schema: a new schema goes
    /// to the controller, and `Values` go to the overlay while it shows
    /// sensors. Never blocks on the controller or the overlay.
    pub fn on_tick(&self, out: &TickOutput, schema: Option<&Schema>) {
        if let Some(new) = &out.schema {
            self.send(Input::Schema(Arc::new(new.clone())));
        }
        let Some(schema) = schema else { return };
        let tap = self.tap.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(sender) = &tap.sender else { return };
        if let Some(msg) = tick_values(
            &tap.plan,
            schema,
            &out.snapshot,
            &out.quality,
            out.snapshot.timestamp_ms,
        ) {
            sender.send(msg);
        }
    }

    pub fn status(&self) -> OverlayStatus {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn send(&self, input: Input) {
        let _ = self.tx.send(input);
    }
}

impl OverlayActions for OverlayHandle {
    /// The «show/hide» hotkey and tray item.
    fn toggle_hidden(&self) {
        self.send(Input::ToggleHidden);
    }

    /// The «next profile» hotkey.
    fn next_profile(&self) {
        self.send(Input::NextProfile);
    }

    fn set_hotkeys(&self, toggle: HotkeyStatus, next_profile: HotkeyStatus) {
        self.send(Input::Hotkeys(OverlayHotkeys {
            toggle,
            next_profile,
        }));
    }
}

/// What the controller's thread is given.
pub struct OverlayDeps {
    pub store: Arc<SettingsStore>,
    pub link: LinkSink,
    pub feed: FramesFeed,
    pub toaster: Box<dyn ToastSink>,
    pub on_status: StatusSink,
}

/// The running controller thread; [`Self::stop`] (or dropping it) ends it in
/// order (module docs).
pub struct OverlayRunner {
    handle: OverlayHandle,
    thread: Option<JoinHandle<()>>,
}

impl OverlayRunner {
    /// Starts the `oma-overlay-ctl` thread; call it once the service link is
    /// spawned. With the overlay off and without `OMA_FRAMES_DEBUG` nothing
    /// else starts.
    pub fn start(deps: OverlayDeps) -> io::Result<Self> {
        let env_value = std::env::var(ENV_VAR).ok();
        let env = options_from_env(env_value.as_deref());
        if env.is_some() {
            tracing::info!(
                value = env_value.as_deref().unwrap_or(""),
                "frame diagnostics on"
            );
        }
        let qpc_frequency = oma_win::qpc_frequency();
        if qpc_frequency == 0 {
            tracing::warn!("no QPC frequency: frame times cannot be converted, frames are ignored");
        }

        let (tx, rx) = mpsc::channel();
        // Subscribed before the snapshot, so no change falls in between (one
        // may arrive twice: the controller ignores equal settings).
        {
            let tx = tx.clone();
            deps.store.subscribe(Box::new(move |settings, _| {
                let _ = tx.send(Input::Settings(Arc::new(settings.clone())));
            }));
        }
        let settings = deps.store.snapshot();
        let lang = language_for(settings.general.language);
        let mut controller = Controller::new(std::process::id(), qpc_frequency);
        // The profile folder is read at start (on the controller's thread, so
        // a large or slow folder never delays the app), when the overlay is
        // turned on and on «Reload» (DP17). The built-ins until then.
        controller.on_catalog(ProfileCatalog::builtins());
        controller.on_settings(&settings, lang, env.clone());

        let handle = OverlayHandle {
            tx: tx.clone(),
            tap: Arc::default(),
            status: Arc::new(Mutex::new(controller.current_status())),
        };
        let ctl = Ctl {
            controller,
            rx,
            tx,
            link: deps.link,
            feed: deps.feed,
            toaster: deps.toaster,
            on_status: deps.on_status,
            tap: Arc::clone(&handle.tap),
            status: Arc::clone(&handle.status),
            env,
            settings,
            lang,
            moves: Arc::default(),
            watcher: None,
            watcher_failed: false,
            host: None,
            host_wanted: false,
            host_start_failed: false,
            tracked: None,
            geometry_ms: 0,
            logged_target: None,
            last_foreground: None,
            foreground_ms: 0,
            foreground_now: current_foreground,
            monitor_of: window_monitor,
            logged_shown: false,
            engine_on: false,
            epoch: Instant::now(),
        };
        let thread = std::thread::Builder::new()
            .name("oma-overlay-ctl".into())
            .spawn(move || ctl.run())?;
        Ok(Self {
            handle,
            thread: Some(thread),
        })
    }

    pub fn handle(&self) -> OverlayHandle {
        self.handle.clone()
    }

    /// Turns the engine off if the controller had turned it on, stops the
    /// overlay, then the watcher, and joins the thread.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.handle.send(Input::Shutdown);
            let _ = thread.join();
        }
    }
}

impl Drop for OverlayRunner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The profile catalog from `%APPDATA%`; the built-ins only without it.
fn read_catalog() -> ProfileCatalog {
    match std::env::var_os("APPDATA") {
        Some(app_data) => load_catalog(&profiles_dir(&PathBuf::from(app_data))),
        None => {
            tracing::warn!("no APPDATA: only the built-in overlay profiles");
            ProfileCatalog::builtins()
        }
    }
}

/// The controller's thread.
struct Ctl {
    controller: Controller,
    rx: Receiver<Input>,
    /// For the watcher's and the host's callbacks.
    tx: Sender<Input>,
    link: LinkSink,
    feed: FramesFeed,
    toaster: Box<dyn ToastSink>,
    on_status: StatusSink,
    tap: Arc<Mutex<Tap>>,
    status: Arc<Mutex<OverlayStatus>>,
    env: Option<FramesConfigure>,
    settings: Arc<Settings>,
    lang: Lang,
    moves: Arc<MoveCoalescer>,
    watcher: Option<ForegroundWatcher>,
    /// The watcher could not start: not tried again until the engine is
    /// wanted anew.
    watcher_failed: bool,
    host: Option<OverlayHost>,
    host_wanted: bool,
    /// The host could not start: not tried again (nor logged) until the
    /// overlay's settings change or «Retry»; the status shows the failure meanwhile.
    host_start_failed: bool,
    /// The window whose geometry is read.
    tracked: Option<Foreground>,
    geometry_ms: u64,
    /// The target last written to the log.
    logged_target: Option<(String, u32)>,
    /// The foreground window as last handed to the controller.
    last_foreground: Option<Foreground>,
    /// When the foreground was last checked against Windows.
    foreground_ms: u64,
    /// The foreground window now (`current_foreground`; a fake in tests).
    foreground_now: fn() -> Option<Foreground>,
    /// The monitor of a window (`window_monitor`; a fake in tests).
    monitor_of: fn(isize) -> Option<PxRect>,
    /// The overlay was last logged as shown.
    logged_shown: bool,
    /// The last `ConfigureFrames` sent turned the engine on.
    engine_on: bool,
    epoch: Instant,
}

impl Ctl {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// The frame engine is wanted: the thread steps every 100 ms.
    fn active(&self) -> bool {
        self.settings.overlay.enabled || self.env.is_some()
    }

    fn run(mut self) {
        self.load_catalog(read_catalog());
        // The first step runs at once, so the engine starts without waiting
        // (as in M7b, DP13).
        let mut next_step_ms: u64 = 0;
        loop {
            let first = if self.active() {
                let wait = next_step_ms.saturating_sub(self.now_ms());
                match self.rx.recv_timeout(Duration::from_millis(wait)) {
                    Ok(input) => Some(input),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                // Off: no timeout, nothing runs until an input arrives.
                match self.rx.recv() {
                    Ok(input) => Some(input),
                    Err(_) => break,
                }
            };
            let pending: Vec<Input> = first.into_iter().chain(self.rx.try_iter()).collect();
            if pending.iter().any(|i| matches!(i, Input::Shutdown)) {
                break;
            }
            for input in pending {
                // Each host state gets a step of its own: a quick restart
                // (`Running`, `Starting`, `Running`) must still be seen, so the
                // new process gets its profile.
                let host = matches!(input, Input::Host(_));
                self.handle(input);
                if host {
                    let now = self.now_ms();
                    self.step(now);
                }
            }
            self.sync_watcher();
            let now = self.now_ms();
            self.step(now);
            next_step_ms = now.saturating_add(STEP.as_millis() as u64);
        }
        self.shutdown();
    }

    fn handle(&mut self, input: Input) {
        match input {
            Input::Settings(settings) => {
                let turned_on = settings.overlay.enabled && !self.settings.overlay.enabled;
                let overlay_changed = settings.overlay != self.settings.overlay;
                self.lang = language_for(settings.general.language);
                self.controller
                    .on_settings(&settings, self.lang, self.env.clone());
                self.settings = settings;
                if turned_on {
                    self.controller.on_catalog(read_catalog());
                }
                if overlay_changed {
                    self.clear_host_start_failure();
                }
            }
            Input::Foreground(fg) => {
                self.last_foreground = Some(fg);
                self.controller.on_foreground(fg);
                self.controller
                    .on_foreground_monitor((self.monitor_of)(fg.hwnd));
                // Back to the game: its state (minimized, visible) may differ.
                if self.tracked.is_some_and(|t| t.hwnd == fg.hwnd) {
                    self.read_geometry();
                }
            }
            Input::Moved => {
                if self.moves.take() {
                    self.read_geometry();
                }
            }
            Input::Host(state) => self.controller.on_host(state),
            Input::Schema(schema) => self.controller.on_schema(schema),
            Input::Retry => {
                self.controller.retry();
                self.clear_host_start_failure();
            }
            Input::ReloadProfiles => self.controller.on_catalog(read_catalog()),
            Input::SetHidden(hidden) => {
                if self.controller.current_status().hidden_by_user != hidden {
                    self.controller.toggle_hidden();
                }
            }
            Input::ToggleHidden => self.controller.toggle_hidden(),
            Input::NextProfile => self.controller.next_profile(),
            Input::Hotkeys(hotkeys) => self.controller.on_hotkeys(hotkeys),
            Input::Shutdown => {}
        }
    }

    /// The catalog read at start: a step publishes it even while nothing
    /// else would step (the overlay off).
    fn load_catalog(&mut self, catalog: ProfileCatalog) {
        self.controller.on_catalog(catalog);
        let now = self.now_ms();
        self.step(now);
    }

    /// The watcher runs only while the frame engine is wanted (§11).
    fn sync_watcher(&mut self) {
        if !self.active() {
            self.watcher_failed = false;
            if self.watcher.take().is_some() {
                self.tracked = None;
            }
            return;
        }
        if self.watcher.is_some() || self.watcher_failed {
            return;
        }
        let sink = watcher_sink(self.tx.clone(), Arc::clone(&self.moves));
        match ForegroundWatcher::spawn(sink) {
            Ok(watcher) => self.watcher = Some(watcher),
            Err(err) => {
                tracing::warn!(%err, "overlay: no foreground watcher, no target");
                self.watcher_failed = true;
            }
        }
    }

    /// Once a second while the overlay or the engine is wanted, the
    /// foreground window is asked of Windows: an event lost or delivered out
    /// of order (the alt-tab switcher after the game) would otherwise leave
    /// the overlay hidden until the next one.
    fn check_foreground(&mut self, now: u64) {
        if !(self.host_wanted || self.engine_on)
            || now.saturating_sub(self.foreground_ms) < FOREGROUND_CHECK_MS
        {
            return;
        }
        self.foreground_ms = now;
        if let Some(fg) = (self.foreground_now)() {
            if Some(fg) != self.last_foreground {
                self.handle(Input::Foreground(fg));
            } else {
                // Same window, maybe moved to another monitor since.
                self.controller
                    .on_foreground_monitor((self.monitor_of)(fg.hwnd));
            }
        }
    }

    /// Reads the tracked window's geometry; a pending move is consumed.
    fn read_geometry(&mut self) {
        let Some(fg) = self.tracked else { return };
        self.moves.take();
        self.controller.on_geometry(window_geometry(fg.hwnd));
        self.geometry_ms = self.now_ms();
    }

    fn step(&mut self, now: u64) {
        let update = self.feed.drain();
        self.controller.on_service(update.connected);
        self.controller.on_frames(
            update.status.as_ref(),
            update.processes.as_ref(),
            &update.batches,
        );
        if self.tracked.is_some() && now.saturating_sub(self.geometry_ms) >= GEOMETRY_REFRESH_MS {
            self.read_geometry();
        }
        self.check_foreground(now);
        let out = self.controller.step(now);
        self.apply(out);
    }

    fn apply(&mut self, out: Outputs) {
        for command in out.link {
            if let LinkCommand::ConfigureFrames(config) = &command {
                self.engine_on = config.enabled;
            }
            (self.link)(command);
        }
        if let Some(window) = out.track {
            self.tracked = window;
            if let Some(watcher) = &self.watcher {
                watcher.track(window);
            }
            // Read at once, so the next step can place the overlay.
            self.read_geometry();
        }
        self.sync_host(out.want_process);
        if let Some(host) = &self.host {
            if out.retry_host {
                host.retry();
            }
            for msg in out.overlay {
                // Shown/hidden changes only: a moving window would log
                // every step.
                if let OverlayMessage::SetPlacement(p) = &msg {
                    let shown = p.area.is_some();
                    match p.area.filter(|_| !self.logged_shown) {
                        Some(a) => tracing::info!(
                            x = a.x,
                            y = a.y,
                            width = a.width,
                            height = a.height,
                            dpi = p.dpi,
                            "overlay shown"
                        ),
                        None if self.logged_shown && !shown => tracing::info!("overlay hidden"),
                        None => {}
                    }
                    self.logged_shown = shown;
                }
                host.send(msg);
            }
        }
        if let Some(plan) = out.values_plan {
            self.tap.lock().unwrap_or_else(PoisonError::into_inner).plan = plan;
        }
        if let Some(note) = &out.foreground_note {
            let exe = note.name.as_deref().unwrap_or("-");
            match &note.presenting {
                Some((fps, mode)) => {
                    tracing::info!(pid = note.pid, %exe, fps = %format!("{fps:.1}"), %mode, "foreground")
                }
                None => tracing::info!(pid = note.pid, %exe, "foreground: not presenting"),
            }
        }
        if let Some(mode) = &out.present_mode {
            tracing::info!(%mode, "target present mode");
        }
        if let Some(ToastRequest::ExclusiveFullscreen { exe }) = out.toast {
            tracing::info!(%exe, "the game runs in exclusive fullscreen: overlay not visible");
            self.toaster.show(
                t(self.lang, "overlay.exclusive.title", &[]),
                t(self.lang, "overlay.exclusive.body", &[]),
                launch_for_main(),
            );
        }
        if let Some(status) = out.status {
            let target = status.target.as_ref().map(|t| (t.name.as_str(), t.pid));
            if self.logged_target.as_ref().map(|(n, p)| (n.as_str(), *p)) != target {
                match target {
                    Some((exe, pid)) => tracing::info!(%exe, pid, "overlay target"),
                    None => tracing::info!("overlay target: none"),
                }
                self.logged_target = target.map(|(n, p)| (n.to_owned(), p));
            }
            // Stored first: a UI that reads it on the event gets this one.
            *self.status.lock().unwrap_or_else(PoisonError::into_inner) = status.clone();
            (self.on_status)(&status);
        }
        if let Some(line) = out.diagnostics_line {
            tracing::info!("{line}");
        }
    }

    /// Starts the host the first time the overlay is wanted, then turns the
    /// process on and off.
    fn sync_host(&mut self, wanted: bool) {
        if wanted == self.host_wanted {
            return;
        }
        if wanted && self.host.is_none() {
            if self.host_start_failed {
                return;
            }
            let exe = match std::env::current_exe() {
                Ok(current) => overlay_exe(&current),
                Err(err) => {
                    tracing::warn!(%err, "overlay: the app's own path is unknown");
                    self.fail_host_start();
                    return;
                }
            };
            let tx = self.tx.clone();
            match OverlayHost::start(exe, move |state| {
                let _ = tx.send(Input::Host(state));
            }) {
                Ok(host) => {
                    self.tap
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .sender = Some(host.sender());
                    self.host = Some(host);
                }
                Err(err) => {
                    tracing::warn!(%err, "overlay: the host thread could not start");
                    self.fail_host_start();
                    return;
                }
            }
        }
        if let Some(host) = &self.host {
            host.set_wanted(wanted);
        }
        self.host_wanted = wanted;
    }

    /// Latches a failed host start: the status shows a failed overlay
    /// process (through the channel, so it gets a step of its own).
    fn fail_host_start(&mut self) {
        self.host_start_failed = true;
        let _ = self.tx.send(Input::Host(HostState::Failed {
            reason: HostFailure::Crashing,
        }));
    }

    /// A new chance for a host that could not start; with the overlay off
    /// the failure no longer shows.
    fn clear_host_start_failure(&mut self) {
        if !std::mem::take(&mut self.host_start_failed) || self.host.is_some() {
            return;
        }
        if !self.settings.overlay.enabled {
            self.controller.on_host(HostState::Off);
        }
    }

    /// The exit order: the engine off (if we turned it on), then the overlay,
    /// then the watcher.
    fn shutdown(mut self) {
        if self.engine_on {
            (self.link)(LinkCommand::ConfigureFrames(FramesConfigure {
                enabled: false,
                track_pc_latency: false,
                track_gpu: false,
            }));
        }
        *self.tap.lock().unwrap_or_else(PoisonError::into_inner) = Tap::default();
        if let Some(host) = self.host.take() {
            host.stop();
        }
        self.watcher.take();
    }
}

/// The overlay's state, for the UI.
#[tauri::command]
pub fn get_overlay_status(state: State<'_, OverlayHandle>) -> OverlayStatus {
    state.status()
}

/// «Retry» on a failed frame engine or overlay process.
#[tauri::command]
pub fn overlay_retry(state: State<'_, OverlayHandle>) {
    state.send(Input::Retry);
}

/// Reads the profile folder again (DP17).
#[tauri::command]
pub fn overlay_reload_profiles(state: State<'_, OverlayHandle>) {
    state.send(Input::ReloadProfiles);
}

/// Hides or shows the overlay; not saved (DP11).
#[tauri::command]
pub fn set_overlay_hidden(state: State<'_, OverlayHandle>, hidden: bool) {
    state.send(Input::SetHidden(hidden));
}
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};

    use oma_core::overlay::Foreground;
    use oma_win::foreground::ForegroundEvent;

    use super::*;
    use crate::overlay::controller::Controller;
    use crate::overlay::profiles::ProfileDiagnostic;

    struct NoToasts;

    impl ToastSink for NoToasts {
        fn show(&self, _title: String, _body: String, _launch: String) {}
    }

    /// A controller thread's state, never run: no watcher, no host.
    fn ctl(settings: Settings) -> Ctl {
        let (tx, rx) = mpsc::channel();
        let controller = Controller::new(1, 10_000_000);
        let status = Arc::new(Mutex::new(controller.current_status()));
        Ctl {
            controller,
            rx,
            tx,
            link: Box::new(|_| {}),
            feed: FramesFeed::default(),
            toaster: Box::new(NoToasts),
            on_status: Box::new(|_| {}),
            tap: Arc::default(),
            status,
            env: None,
            settings: Arc::new(settings),
            lang: Lang::En,
            moves: Arc::default(),
            watcher: None,
            watcher_failed: false,
            host: None,
            host_wanted: false,
            host_start_failed: false,
            tracked: None,
            geometry_ms: 0,
            logged_target: None,
            last_foreground: None,
            foreground_ms: 0,
            foreground_now: current_foreground,
            monitor_of: window_monitor,
            logged_shown: false,
            engine_on: false,
            epoch: Instant::now(),
        }
    }

    #[test]
    fn foreground_is_checked_once_a_second_while_the_overlay_is_wanted() {
        static NOW: Mutex<Option<Foreground>> = Mutex::new(None);
        let game = Foreground {
            pid: 100,
            hwnd: 0x1000,
        };
        let switcher = Foreground {
            pid: 200,
            hwnd: 0x2000,
        };
        let mut ctl = ctl(Settings::default());
        ctl.foreground_now = || *NOW.lock().unwrap();
        ctl.host_wanted = true;
        // The last event named the alt-tab switcher; the game holds the
        // foreground.
        static MONITOR_READS: AtomicUsize = AtomicUsize::new(0);
        ctl.monitor_of = |_| {
            MONITOR_READS.fetch_add(1, Ordering::SeqCst);
            None
        };
        ctl.handle(Input::Foreground(switcher));
        *NOW.lock().unwrap() = Some(game);
        ctl.check_foreground(500);
        assert_eq!(ctl.last_foreground, Some(switcher), "not due yet");
        ctl.check_foreground(1_000);
        assert_eq!(ctl.last_foreground, Some(game));
        // The same window a second later: its monitor is read again (it may
        // have been moved onto the game's monitor).
        let reads = MONITOR_READS.load(Ordering::SeqCst);
        ctl.check_foreground(2_000);
        assert_eq!(MONITOR_READS.load(Ordering::SeqCst), reads + 1);
        // Not wanted: no check.
        ctl.host_wanted = false;
        *NOW.lock().unwrap() = Some(switcher);
        ctl.check_foreground(5_000);
        assert_eq!(ctl.last_foreground, Some(game));
    }

    #[test]
    fn host_start_failure_is_cleared_only_by_overlay_settings() {
        let mut ctl = ctl(Settings::default());
        ctl.host_start_failed = true;
        // Another section changed: the failure stays latched.
        let mut other = Settings::default();
        other.general.interval_ms = 2_000;
        ctl.handle(Input::Settings(Arc::new(other.clone())));
        assert!(ctl.host_start_failed);
        // The overlay's settings changed: a new chance.
        other.overlay.text_hz = 4;
        ctl.handle(Input::Settings(Arc::new(other)));
        assert!(!ctl.host_start_failed);
    }

    #[test]
    fn catalog_read_on_the_thread_is_published() {
        let published = Arc::new(Mutex::new(Vec::new()));
        let mut ctl = ctl(Settings::default());
        ctl.on_status = {
            let published = Arc::clone(&published);
            Box::new(move |s: &OverlayStatus| published.lock().unwrap().push(s.clone()))
        };
        let mut catalog = ProfileCatalog::builtins();
        catalog.diagnostics.push(ProfileDiagnostic {
            file: "x.json".into(),
            reason: "bad".into(),
        });
        ctl.load_catalog(catalog.clone());
        let status = ctl.status.lock().unwrap().clone();
        assert_eq!(status.diagnostics, catalog.diagnostics);
        assert_eq!(status.profiles, catalog.entries);
        assert_eq!(published.lock().unwrap().last(), Some(&status));
    }

    #[test]
    fn moves_are_coalesced() {
        let (tx, rx) = mpsc::channel();
        let moves = Arc::new(MoveCoalescer::default());
        let sink = watcher_sink(tx, Arc::clone(&moves));
        for _ in 0..100 {
            sink(ForegroundEvent::Moved { hwnd: 0x1000 });
        }
        let wakes: Vec<Input> = rx.try_iter().collect();
        assert_eq!(wakes.len(), 1, "one wake for 100 moves");
        let reads = AtomicUsize::new(0);
        for input in wakes {
            assert!(matches!(input, Input::Moved));
            if moves.take() {
                reads.fetch_add(1, Ordering::Relaxed);
            }
        }
        assert_eq!(reads.load(Ordering::Relaxed), 1, "one geometry read");
        // Once read, the next move wakes the thread again.
        sink(ForegroundEvent::Moved { hwnd: 0x1000 });
        assert_eq!(rx.try_iter().count(), 1);
        // A foreground change is never coalesced.
        let fg = Foreground {
            pid: 1,
            hwnd: 0x2000,
        };
        sink(ForegroundEvent::Foreground(fg));
        sink(ForegroundEvent::Foreground(fg));
        assert_eq!(rx.try_iter().count(), 2);
    }

    #[test]
    fn overlay_status_serializes_camel_case() {
        let status = Controller::new(1, 10_000_000).current_status();
        let json = serde_json::to_value(&status).unwrap();
        for key in [
            "enabled",
            "process",
            "processReason",
            "frames",
            "framesDetail",
            "target",
            "activeProfile",
            "profiles",
            "diagnostics",
            "hiddenByUser",
            "hotkeys",
        ] {
            assert!(json.get(key).is_some(), "{key} missing: {json}");
        }
        assert!(json["hotkeys"].get("nextProfile").is_some(), "{json}");
        assert_eq!(EVENT_OVERLAY_STATUS, "overlay-status");
    }
}
