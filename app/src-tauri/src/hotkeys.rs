//! Global hotkeys of the CSV log (spec M5 §4.6, L10) and of the overlay
//! (M7c, DP14): registered with `tauri-plugin-global-shortcut` from Rust only
//! (no capability reaches JavaScript), swapped safely, filtered against key
//! repeat and reported in `LogStatus.hotkeys` and `OverlayStatus.hotkeys`.
//!
//! Threads: the plugin runs `RegisterHotKey` on the main thread and waits
//! for it, so registrations run on a thread of their own, never on a
//! settings listener's or the main thread. Presses arrive on the main thread
//! (and releases on the plugin's threads); they only read the bindings and
//! queue the command: the log's for a worker, which waits for the writer, the
//! overlay's for its controller, without notifications.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use oma_core::hotkey::{parse_hotkey, Hotkey, HotkeyKey};
use oma_core::settings::Settings;
use tauri::{AppHandle, Manager, WindowEvent, Wry};
use tauri_plugin_global_shortcut::{Code, GlobalShortcut, Modifiers, Shortcut, ShortcutState};

use crate::i18n::{t, Lang};
use crate::log::commands::{failure_toast, toast_log};
use crate::log::session::{LogState, LogStatus};
use crate::log::{HotkeyState, HotkeyStatus, HotkeyStatuses, LogService};
use crate::notifier::SystemToaster;
use crate::settings::SettingsStore;
use crate::tray::language_for;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a global hotkey does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    LogToggle,
    LogPause,
    OverlayToggle,
    OverlayNextProfile,
}

/// How many actions have a hotkey.
const ACTIONS: usize = 4;

impl HotkeyAction {
    /// In order of precedence: a combination asked for by two actions goes
    /// to the first (DP14).
    pub const ALL: [Self; ACTIONS] = [
        Self::LogToggle,
        Self::LogPause,
        Self::OverlayToggle,
        Self::OverlayNextProfile,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::LogToggle => 0,
            Self::LogPause => 1,
            Self::OverlayToggle => 2,
            Self::OverlayNextProfile => 3,
        }
    }
}

/// The overlay's side of the hotkeys (its controller's handle): never blocks.
pub trait OverlayActions: Send + Sync {
    fn toggle_hidden(&self);
    fn next_profile(&self);
    /// The statuses of «show/hide» and «next profile», for `OverlayStatus`.
    fn set_hotkeys(&self, toggle: HotkeyStatus, next_profile: HotkeyStatus);
}

/// Hands an overlay press to the controller; a press of the log is returned
/// for the log's worker.
pub fn route_press(press: Press, overlay: Option<&dyn OverlayActions>) -> Option<Press> {
    match press.action {
        HotkeyAction::LogToggle | HotkeyAction::LogPause => Some(press),
        HotkeyAction::OverlayToggle => {
            if let Some(overlay) = overlay {
                overlay.toggle_hidden();
            }
            None
        }
        HotkeyAction::OverlayNextProfile => {
            if let Some(overlay) = overlay {
                overlay.next_profile();
            }
            None
        }
    }
}

/// The statuses by action: the log's two and the overlay's two.
fn split_statuses(statuses: [HotkeyStatus; ACTIONS]) -> (HotkeyStatuses, [HotkeyStatus; 2]) {
    let [toggle, pause, overlay_toggle, next_profile] = statuses;
    (
        HotkeyStatuses { toggle, pause },
        [overlay_toggle, next_profile],
    )
}

/// Gives the overlay its statuses; the log's are returned.
fn publish_statuses(
    statuses: [HotkeyStatus; ACTIONS],
    overlay: Option<&dyn OverlayActions>,
) -> HotkeyStatuses {
    let (log, [toggle, next_profile]) = split_statuses(statuses);
    if let Some(overlay) = overlay {
        overlay.set_hotkeys(toggle, next_profile);
    }
    log
}

/// Registers combinations with the operating system (faked in the tests).
pub trait HotkeyRegistrar: Send + Sync {
    fn register(&self, hotkey: Hotkey) -> Result<(), RegisterError>;
    fn unregister(&self, hotkey: Hotkey);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// Taken by another application (`ERROR_HOTKEY_ALREADY_REGISTERED`).
    InUse,
    Other(String),
}

impl RegisterError {
    /// Classifies the plugin's error text: global-hotkey's `AlreadyRegistered`
    /// reads "HotKey already registered: ...", and an OS error 1409
    /// (`ERROR_HOTKEY_ALREADY_REGISTERED`) that reaches it unmapped ends in
    /// "(os error 1409)".
    pub fn from_plugin(text: &str) -> Self {
        if text.to_ascii_lowercase().contains("already registered")
            || text.contains("os error 1409")
        {
            Self::InUse
        } else {
            Self::Other(text.to_owned())
        }
    }

    /// The i18n key shown as the reason.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::InUse => "log.hotkey.inUse",
            Self::Other(_) => "log.hotkey.failed",
        }
    }
}

/// Reacts to a press only after a release since the previous one (L10):
/// Windows repeats `WM_HOTKEY` while the keys are held.
#[derive(Debug, Default)]
pub struct PressFilter {
    down: bool,
}

impl PressFilter {
    pub fn accept(&mut self, pressed: bool) -> bool {
        let accept = pressed && !self.down;
        self.down = pressed;
        accept
    }
}

/// An accepted press: the action and the bindings generation it was read
/// from, both captured before the work is queued, so a later change of the
/// bindings does not reinterpret it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    pub action: HotkeyAction,
    pub generation: u64,
}

#[derive(Default)]
struct Bindings {
    /// Grows with every change of `actions`.
    generation: u64,
    /// The effective combination of each action, by [`HotkeyAction::index`].
    actions: [Option<Hotkey>; ACTIONS],
    /// One filter per bound combination, dropped when it is unbound.
    filters: HashMap<Hotkey, PressFilter>,
    /// Set while a hotkey capture box in the settings has focus.
    suspended: bool,
}

impl Bindings {
    fn action_for(&self, hotkey: Hotkey) -> Option<HotkeyAction> {
        HotkeyAction::ALL
            .into_iter()
            .find(|action| self.actions[action.index()] == Some(hotkey))
    }
}

/// The combination → action table the press callback reads. Its lock is
/// never held while the registrar is called, so a callback on the main
/// thread never waits for a registration that waits for the main thread.
#[derive(Clone, Default)]
pub struct Dispatch {
    inner: Arc<Mutex<Bindings>>,
}

impl Dispatch {
    /// One press event of `hotkey`: the action when the filter accepts it.
    /// Nothing while suspended.
    pub fn press(&self, hotkey: Hotkey, pressed: bool) -> Option<Press> {
        let mut bindings = lock(&self.inner);
        if bindings.suspended {
            return None;
        }
        let action = bindings.action_for(hotkey)?;
        let generation = bindings.generation;
        let accepted = bindings.filters.entry(hotkey).or_default().accept(pressed);
        accepted.then_some(Press { action, generation })
    }

    /// Ignores every press from now on, before the hotkeys thread has
    /// released the combinations, or accepts them again.
    pub fn set_suspended(&self, suspended: bool) {
        lock(&self.inner).suspended = suspended;
    }

    // Presses go through `press`; the lookup alone serves the tests.
    #[cfg(test)]
    fn action_for(&self, hotkey: Hotkey) -> Option<HotkeyAction> {
        lock(&self.inner).action_for(hotkey)
    }

    fn publish(&self, actions: [Option<Hotkey>; ACTIONS]) {
        let mut bindings = lock(&self.inner);
        if bindings.actions == actions {
            return;
        }
        bindings.generation += 1;
        bindings.actions = actions;
        bindings
            .filters
            .retain(|hotkey, _| actions.contains(&Some(*hotkey)));
    }
}

/// What the `oma-hotkeys` thread is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyRequest {
    /// The requested combinations changed.
    Settings(Requested),
    /// A capture box gained (`true`) or lost focus.
    Suspend(bool),
}

/// Owns the registered combinations of every action.
pub struct HotkeyManager<R: HotkeyRegistrar> {
    registrar: R,
    /// Registered by us, by [`HotkeyAction::index`]; never the same twice.
    effective: [Option<Hotkey>; ACTIONS],
    dispatch: Dispatch,
    /// The latest [`HotkeyRequest::Settings`].
    requested: Option<Requested>,
    /// The latest [`HotkeyRequest::Suspend`].
    suspended: bool,
}

impl<R: HotkeyRegistrar> HotkeyManager<R> {
    pub fn new(registrar: R, dispatch: Dispatch) -> Self {
        Self {
            registrar,
            effective: [None; ACTIONS],
            dispatch,
            requested: None,
            suspended: false,
        }
    }

    /// One batch of requests; only the latest of each kind matters. While
    /// suspended our combinations are released, so the capture box receives
    /// their keys (`RegisterHotKey` would consume them), and a settings
    /// change is only remembered; on resume the latest request goes through
    /// [`Self::apply`]. The statuses to publish, none while suspended (the
    /// published ones stay as they were).
    pub fn handle(
        &mut self,
        requests: impl IntoIterator<Item = HotkeyRequest>,
    ) -> Option<[HotkeyStatus; ACTIONS]> {
        for request in requests {
            match request {
                HotkeyRequest::Settings(requested) => self.requested = Some(requested),
                HotkeyRequest::Suspend(suspended) => self.suspended = suspended,
            }
        }
        if self.suspended {
            if self.effective != [None; ACTIONS] {
                self.apply([None; ACTIONS]);
            }
            return None;
        }
        let requested = self.requested.clone()?;
        Some(self.apply(requested.each_ref().map(Option::as_deref)))
    }

    /// Registers the new combination first, releases the old one only after
    /// success (spec M5 §4.6). Every action is reconciled together: a
    /// combination we own already is rebound without registering it again
    /// (also when two actions swap), and an old one still effective for
    /// another action is not released. `texts` and the statuses go by
    /// [`HotkeyAction::index`].
    pub fn apply(&mut self, texts: [Option<&str>; ACTIONS]) -> [HotkeyStatus; ACTIONS] {
        let old = self.effective;
        // Per action: the combination to bind (`None` = unset) or the
        // reason it cannot be.
        let mut outcome: [Result<Option<Hotkey>, &'static str>; ACTIONS] = texts.map(|text| {
            text.map_or(Ok(None), |text| {
                parse_hotkey(text).map(Some).map_err(|err| {
                    tracing::warn!(hotkey = text, ?err, "unreadable hotkey");
                    "log.hotkey.failed"
                })
            })
        });
        // The settings never give two actions the same combination; if they
        // did, the later action would lose it (DP14).
        for i in 1..ACTIONS {
            if let Ok(Some(hotkey)) = outcome[i] {
                if outcome[..i].contains(&Ok(Some(hotkey))) {
                    outcome[i] = Err("log.hotkey.inUse");
                }
            }
        }
        for action in HotkeyAction::ALL {
            let slot = &mut outcome[action.index()];
            let Ok(Some(hotkey)) = *slot else { continue };
            if old.contains(&Some(hotkey)) {
                continue;
            }
            if let Err(err) = self.registrar.register(hotkey) {
                tracing::warn!(?action, %hotkey, ?err, "cannot register the hotkey");
                *slot = Err(err.reason());
            }
        }
        let mut effective = outcome.map(|result| result.ok().flatten());
        // A failed action keeps its previous combination, unless another
        // action has just taken it.
        for i in 0..ACTIONS {
            if outcome[i].is_err() && old[i].is_some() && !effective.contains(&old[i]) {
                effective[i] = old[i];
            }
        }
        self.dispatch.publish(effective);
        for hotkey in old.into_iter().flatten() {
            if !effective.contains(&Some(hotkey)) {
                self.registrar.unregister(hotkey);
            }
        }
        self.effective = effective;
        HotkeyAction::ALL.map(|action| {
            let i = action.index();
            HotkeyStatus {
                requested: texts[i].map(str::to_owned),
                effective: effective[i].map(|hotkey| hotkey.to_string()),
                state: match outcome[i] {
                    Ok(None) => HotkeyState::Unset,
                    Ok(Some(_)) => HotkeyState::Active,
                    Err(_) => HotkeyState::Failed,
                },
                reason: outcome[i].err().map(str::to_owned),
            }
        })
    }

    /// The action bound to `hotkey` now.
    #[cfg(test)]
    pub fn action_for(&self, hotkey: Hotkey) -> Option<HotkeyAction> {
        self.dispatch.action_for(hotkey)
    }
}

/// A command of the coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogCommand {
    Start,
    Pause,
    Resume,
    Stop,
}

/// What `action` does to the log in `state`, if anything (nothing for the
/// overlay's actions).
pub fn command_for(action: HotkeyAction, state: LogState) -> Option<LogCommand> {
    match (action, state) {
        (HotkeyAction::LogToggle, LogState::Idle | LogState::Error) => Some(LogCommand::Start),
        (HotkeyAction::LogToggle, LogState::Recording | LogState::Paused) => Some(LogCommand::Stop),
        (HotkeyAction::LogPause, LogState::Recording) => Some(LogCommand::Pause),
        (HotkeyAction::LogPause, LogState::Paused) => Some(LogCommand::Resume),
        (HotkeyAction::LogPause, LogState::Idle | LogState::Error) => None,
        (HotkeyAction::OverlayToggle | HotkeyAction::OverlayNextProfile, _) => None,
    }
}

/// The toast after a hotkey command, once the writer has answered: started
/// or stopped with the file name, or the failure (never a confirmation);
/// nothing for a pause or a resume that went through.
fn outcome_toast(lang: Lang, command: LogCommand, status: &LogStatus) -> Option<(String, String)> {
    if let Some(failure) = failure_toast(lang, status) {
        return Some(failure);
    }
    let key = match (command, status.state) {
        (LogCommand::Start, LogState::Recording) => "log.toast.started",
        (LogCommand::Stop, LogState::Idle) => "log.toast.stopped",
        _ => return None,
    };
    let file = status
        .path
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some((t(lang, key, &[]), file))
}

/// The combinations the settings ask for, by [`HotkeyAction::index`];
/// `overlay.hotkeyBenchmark` is registered by the M7d (DP11).
type Requested = [Option<String>; ACTIONS];

fn requested(settings: &Settings) -> Requested {
    [
        settings.log.hotkey_toggle.clone(),
        settings.log.hotkey_pause.clone(),
        settings.overlay.hotkey_toggle.clone(),
        settings.overlay.hotkey_next_profile.clone(),
    ]
}

/// Calls `apply` with the requested combinations once right after
/// subscribing and then on every change of them. Unlike `rules.rs`, the
/// catch-up reads the store after taking the dedup lock, so it can never
/// apply a value older than one a listener applied before it.
fn follow_hotkeys(store: &Arc<SettingsStore>, apply: impl Fn(Requested) + Send + Sync + 'static) {
    let seen: Mutex<Option<Requested>> = Mutex::new(None);
    let offer = move |read: &dyn Fn() -> Requested| {
        let mut seen = lock(&seen);
        let now = read();
        if seen.as_ref() == Some(&now) {
            return;
        }
        apply(now.clone());
        *seen = Some(now);
    };
    let offer = Arc::new(offer);
    let listener = Arc::clone(&offer);
    store.subscribe(Box::new(move |settings, _| {
        listener(&|| requested(settings));
    }));
    // `snapshot` takes and releases the store lock; listeners run outside it.
    offer(&|| requested(&store.snapshot()));
}

/// The plugin as a [`HotkeyRegistrar`]: each registration carries a handler
/// that hands its events to `on_event` (combination, pressed).
struct PluginRegistrar {
    app: AppHandle,
    on_event: Arc<dyn Fn(Hotkey, bool) + Send + Sync>,
}

/// The plugin's form of a combination (`Code::KeyA`…, `Digit0`…, `F1`…).
fn shortcut_of(hotkey: Hotkey) -> Option<Shortcut> {
    let mut mods = Modifiers::empty();
    for (on, modifier) in [
        (hotkey.ctrl, Modifiers::CONTROL),
        (hotkey.alt, Modifiers::ALT),
        (hotkey.shift, Modifiers::SHIFT),
    ] {
        if on {
            mods |= modifier;
        }
    }
    let code = match hotkey.key {
        HotkeyKey::Letter(letter) => format!("Key{letter}"),
        HotkeyKey::Digit(digit) => format!("Digit{digit}"),
        HotkeyKey::Function(n) => format!("F{n}"),
    };
    let code: Code = code.parse().ok()?;
    Some(Shortcut::new(Some(mods), code))
}

impl HotkeyRegistrar for PluginRegistrar {
    fn register(&self, hotkey: Hotkey) -> Result<(), RegisterError> {
        let shortcut = shortcut_of(hotkey)
            .ok_or_else(|| RegisterError::Other(format!("no key code for {hotkey}")))?;
        let plugin = self
            .app
            .try_state::<GlobalShortcut<Wry>>()
            .ok_or_else(|| RegisterError::Other("the global shortcut plugin is missing".into()))?;
        let on_event = Arc::clone(&self.on_event);
        plugin
            .on_shortcut(shortcut, move |_, _, event| {
                on_event(hotkey, event.state == ShortcutState::Pressed);
            })
            .map_err(|err| RegisterError::from_plugin(&err.to_string()))
    }

    fn unregister(&self, hotkey: Hotkey) {
        let (Some(shortcut), Some(plugin)) = (
            shortcut_of(hotkey),
            self.app.try_state::<GlobalShortcut<Wry>>(),
        ) else {
            return;
        };
        if let Err(err) = plugin.unregister(shortcut) {
            tracing::warn!(%hotkey, %err, "cannot release the hotkey");
        }
    }
}

/// Runs one accepted press on the action worker, then toasts the outcome.
fn run_press(app: &AppHandle, store: &SettingsStore, log: &LogService, press: Press) {
    let state = log.status().state;
    let Some(command) = command_for(press.action, state) else {
        return;
    };
    tracing::info!(
        action = ?press.action,
        generation = press.generation,
        ?command,
        "log hotkey"
    );
    let status = match command {
        LogCommand::Start => log.start(),
        LogCommand::Pause => log.pause(),
        LogCommand::Resume => log.resume(),
        LogCommand::Stop => log.stop(),
    };
    let lang = language_for(store.snapshot().general.language);
    if let Some((title, body)) = outcome_toast(lang, command, &status) {
        if let Some(toaster) = app.try_state::<Arc<SystemToaster>>() {
            toast_log(&toaster, title, body);
        }
    }
}

/// Suspends every global hotkey, the log's and the overlay's (DP14), while a
/// capture box in the settings has focus (managed state of
/// [`set_log_hotkeys_suspended`]).
pub struct HotkeyControl {
    dispatch: Dispatch,
    /// Under its lock the flag and the request change in the same order.
    requests: Mutex<Sender<HotkeyRequest>>,
}

impl HotkeyControl {
    fn new(dispatch: Dispatch, requests: Sender<HotkeyRequest>) -> Self {
        Self {
            dispatch,
            requests: Mutex::new(requests),
        }
    }

    /// Presses are ignored at once; the hotkeys thread then releases our
    /// combinations, or registers them again.
    pub fn set_suspended(&self, suspended: bool) {
        let requests = lock(&self.requests);
        self.dispatch.set_suspended(suspended);
        let _ = requests.send(HotkeyRequest::Suspend(suspended));
    }
}

/// Called by the settings with `true` when a hotkey capture box gains focus
/// and `false` when it loses it, so typing a combination we registered is
/// captured instead of acted on. Despite its name it suspends all four
/// hotkeys (DP14).
#[tauri::command]
pub fn set_log_hotkeys_suspended(app: AppHandle, suspended: bool) {
    if let Some(control) = app.try_state::<HotkeyControl>() {
        control.set_suspended(suspended);
    }
}

/// Whether a main window event means no capture box can hold focus any more:
/// the window lost focus or went away (closed, or its page with it), when
/// the page's own blur or teardown may never arrive.
pub fn window_event_resumes(event: &WindowEvent) -> bool {
    matches!(event, WindowEvent::Focused(false) | WindowEvent::Destroyed)
}

/// Resumes the hotkeys after [`window_event_resumes`] events of the main
/// window; the page suspends them again when a focused capture box regains
/// focus.
pub fn resume_on_window_event(app: &AppHandle, event: &WindowEvent) {
    if window_event_resumes(event) {
        if let Some(control) = app.try_state::<HotkeyControl>() {
            control.set_suspended(false);
        }
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(err) = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
    {
        tracing::error!(%err, thread = name, "cannot start a hotkey thread");
    }
}

/// From here on `log.hotkeyToggle`, `log.hotkeyPause`, `overlay.hotkeyToggle`
/// and `overlay.hotkeyNextProfile` drive the global hotkeys; their state goes
/// to `LogStatus.hotkeys` and, through `overlay`, to `OverlayStatus.hotkeys`.
/// The plugin must be registered on the builder (`main.rs`) and the shared
/// toaster managed. Without `overlay` (off Windows) its presses do nothing.
pub fn install_hotkeys(
    app: &AppHandle,
    store: &Arc<SettingsStore>,
    log: Arc<LogService>,
    overlay: Option<Arc<dyn OverlayActions>>,
) {
    let dispatch = Dispatch::default();
    let (presses, pressed) = channel::<Press>();
    let on_event: Arc<dyn Fn(Hotkey, bool) + Send + Sync> = {
        let dispatch = dispatch.clone();
        let overlay = overlay.clone();
        Arc::new(move |hotkey, down| {
            let Some(press) = dispatch.press(hotkey, down) else {
                return;
            };
            // The overlay's go straight to its controller's channel, so they
            // never wait behind a log command.
            if let Some(press) = route_press(press, overlay.as_deref()) {
                let _ = presses.send(press);
            }
        })
    };
    let registrar = PluginRegistrar {
        app: app.clone(),
        on_event,
    };
    let mut manager = HotkeyManager::new(registrar, dispatch.clone());
    let (requests, request_rx): (_, Receiver<HotkeyRequest>) = channel();
    app.manage(HotkeyControl::new(dispatch, requests.clone()));
    let status_log = Arc::clone(&log);
    spawn("oma-hotkeys", move || {
        while let Ok(first) = request_rx.recv() {
            let batch = std::iter::once(first).chain(request_rx.try_iter());
            if let Some(statuses) = manager.handle(batch) {
                status_log.set_hotkeys(publish_statuses(statuses, overlay.as_deref()));
            }
        }
    });
    let action_app = app.clone();
    let action_store = Arc::clone(store);
    spawn("oma-hotkey-actions", move || {
        for press in pressed {
            run_press(&action_app, &action_store, &log, press);
        }
    });
    follow_hotkeys(store, move |requested| {
        let _ = requests.send(HotkeyRequest::Settings(requested));
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use oma_core::settings::Settings;
    use serde_json::json;

    use super::*;
    use crate::log::session::LogError;
    use crate::settings::fake_fs::{open_fast, wait_until, FakeFs};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Register(String),
        Unregister(String),
    }

    /// Records the calls; refuses the combinations it is told to.
    #[derive(Clone, Default)]
    struct FakeRegistrar {
        calls: Arc<Mutex<Vec<Call>>>,
        refuse: Arc<Mutex<HashMap<String, RegisterError>>>,
    }

    impl FakeRegistrar {
        fn refuse(&self, hotkey: &str, error: RegisterError) {
            self.refuse.lock().unwrap().insert(hotkey.to_owned(), error);
        }

        fn take(&self) -> Vec<Call> {
            std::mem::take(&mut *self.calls.lock().unwrap())
        }
    }

    impl HotkeyRegistrar for FakeRegistrar {
        fn register(&self, hotkey: Hotkey) -> Result<(), RegisterError> {
            let text = hotkey.to_string();
            self.calls
                .lock()
                .unwrap()
                .push(Call::Register(text.clone()));
            match self.refuse.lock().unwrap().get(&text) {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        fn unregister(&self, hotkey: Hotkey) {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Unregister(hotkey.to_string()));
        }
    }

    fn manager() -> (HotkeyManager<FakeRegistrar>, FakeRegistrar) {
        let fake = FakeRegistrar::default();
        (HotkeyManager::new(fake.clone(), Dispatch::default()), fake)
    }

    fn key(text: &str) -> Hotkey {
        parse_hotkey(text).unwrap()
    }

    fn reg(text: &str) -> Call {
        Call::Register(text.to_owned())
    }

    fn unreg(text: &str) -> Call {
        Call::Unregister(text.to_owned())
    }

    fn active(text: &str) -> HotkeyStatus {
        HotkeyStatus {
            requested: Some(text.to_owned()),
            effective: Some(text.to_owned()),
            state: HotkeyState::Active,
            reason: None,
        }
    }

    #[test]
    fn default_toggle_is_registered_and_pause_is_unset() {
        let (mut manager, fake) = manager();
        let log = Settings::default().log;
        let statuses = manager.apply([
            log.hotkey_toggle.as_deref(),
            log.hotkey_pause.as_deref(),
            None,
            None,
        ]);
        assert_eq!(fake.take(), [reg("Ctrl+Alt+Shift+R")]);
        assert_eq!(statuses[0], active("Ctrl+Alt+Shift+R"));
        assert_eq!(statuses[1], HotkeyStatus::default());
        assert_eq!(statuses[1].state, HotkeyState::Unset);
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+Shift+R")),
            Some(HotkeyAction::LogToggle)
        );
    }

    #[test]
    fn change_registers_new_before_releasing_old() {
        let (mut manager, fake) = manager();
        manager.apply([Some("Ctrl+Alt+R"), None, None, None]);
        fake.take();
        let statuses = manager.apply([Some("Ctrl+Alt+T"), None, None, None]);
        assert_eq!(fake.take(), [reg("Ctrl+Alt+T"), unreg("Ctrl+Alt+R")]);
        assert_eq!(statuses[0], active("Ctrl+Alt+T"));
        assert_eq!(manager.action_for(key("Ctrl+Alt+R")), None);
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+T")),
            Some(HotkeyAction::LogToggle)
        );
    }

    #[test]
    fn failed_change_keeps_the_old_combination() {
        let (mut manager, fake) = manager();
        manager.apply([Some("Ctrl+Alt+R"), None, None, None]);
        fake.refuse("Ctrl+Alt+T", RegisterError::InUse);
        fake.take();
        let statuses = manager.apply([Some("Ctrl+Alt+T"), None, None, None]);
        // The old one is not released.
        assert_eq!(fake.take(), [reg("Ctrl+Alt+T")]);
        assert_eq!(
            statuses[0],
            HotkeyStatus {
                requested: Some("Ctrl+Alt+T".to_owned()),
                effective: Some("Ctrl+Alt+R".to_owned()),
                state: HotkeyState::Failed,
                reason: Some("log.hotkey.inUse".to_owned()),
            }
        );
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+R")),
            Some(HotkeyAction::LogToggle)
        );
        assert_eq!(manager.action_for(key("Ctrl+Alt+T")), None);
    }

    #[test]
    fn clearing_a_hotkey_releases_it() {
        let (mut manager, fake) = manager();
        manager.apply([Some("Ctrl+Alt+R"), Some("Ctrl+Alt+P"), None, None]);
        fake.take();
        let statuses = manager.apply([Some("Ctrl+Alt+R"), None, None, None]);
        assert_eq!(fake.take(), [unreg("Ctrl+Alt+P")]);
        assert_eq!(statuses[0], active("Ctrl+Alt+R"));
        assert_eq!(statuses[1], HotkeyStatus::default());
        assert_eq!(manager.action_for(key("Ctrl+Alt+P")), None);
    }

    #[test]
    fn initial_failure_leaves_it_inactive() {
        let (mut manager, fake) = manager();
        fake.refuse("Ctrl+Alt+Shift+R", RegisterError::Other("boom".to_owned()));
        let statuses = manager.apply([Some("Ctrl+Alt+Shift+R"), None, None, None]);
        assert_eq!(
            statuses[0],
            HotkeyStatus {
                requested: Some("Ctrl+Alt+Shift+R".to_owned()),
                effective: None,
                state: HotkeyState::Failed,
                reason: Some("log.hotkey.failed".to_owned()),
            }
        );
        assert_eq!(manager.action_for(key("Ctrl+Alt+Shift+R")), None);
        // Nothing registered, nothing to release later.
        fake.take();
        manager.apply([None, None, None, None]);
        assert_eq!(fake.take(), []);
    }

    #[test]
    fn held_hotkey_acts_once() {
        let dispatch = Dispatch::default();
        let mut manager = HotkeyManager::new(FakeRegistrar::default(), dispatch.clone());
        manager.apply([Some("Ctrl+Alt+R"), Some("Ctrl+Alt+P"), None, None]);
        let r = key("Ctrl+Alt+R");
        let p = key("Ctrl+Alt+P");
        let acted = [true, true, true, false, true]
            .into_iter()
            .filter_map(|pressed| dispatch.press(r, pressed))
            .count();
        assert_eq!(acted, 2);
        // `r` is still held: the other combination has its own filter.
        assert_eq!(
            dispatch.press(p, true).map(|press| press.action),
            Some(HotkeyAction::LogPause)
        );
        assert_eq!(dispatch.press(r, true), None);
        // A combination that is not bound does nothing.
        assert_eq!(dispatch.press(key("Ctrl+Alt+X"), true), None);
    }

    #[test]
    fn hotkey_swap_keeps_actions_consistent() {
        let dispatch = Dispatch::default();
        let fake = FakeRegistrar::default();
        let mut manager = HotkeyManager::new(fake.clone(), dispatch.clone());
        manager.apply([Some("Ctrl+Alt+R"), Some("Ctrl+Alt+P"), None, None]);
        fake.take();
        let before = dispatch.press(key("Ctrl+Alt+R"), true).unwrap();
        assert_eq!(before.action, HotkeyAction::LogToggle);

        let statuses = manager.apply([Some("Ctrl+Alt+P"), Some("Ctrl+Alt+R"), None, None]);
        // Both are owned already: rebound, not registered twice nor released.
        assert_eq!(fake.take(), []);
        assert_eq!(statuses[0], active("Ctrl+Alt+P"));
        assert_eq!(statuses[1], active("Ctrl+Alt+R"));
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+P")),
            Some(HotkeyAction::LogToggle)
        );
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+R")),
            Some(HotkeyAction::LogPause)
        );
        // The press captured before the swap keeps its action.
        assert_eq!(before.action, HotkeyAction::LogToggle);
        dispatch.press(key("Ctrl+Alt+R"), false);
        let after = dispatch.press(key("Ctrl+Alt+R"), true).unwrap();
        assert_eq!(after.action, HotkeyAction::LogPause);
        assert!(after.generation > before.generation);

        // A swap where one side is new: the other is kept, not released.
        let statuses = manager.apply([Some("Ctrl+Alt+R"), Some("Ctrl+Alt+Q"), None, None]);
        assert_eq!(fake.take(), [reg("Ctrl+Alt+Q"), unreg("Ctrl+Alt+P")]);
        assert_eq!(statuses[0], active("Ctrl+Alt+R"));
        assert_eq!(statuses[1], active("Ctrl+Alt+Q"));
    }

    #[test]
    fn failed_change_does_not_keep_a_combination_the_other_action_took() {
        let (mut manager, fake) = manager();
        manager.apply([Some("Ctrl+Alt+R"), Some("Ctrl+Alt+P"), None, None]);
        fake.refuse("Ctrl+Alt+Q", RegisterError::InUse);
        fake.take();
        // Toggle takes pause's combination; pause's new one fails.
        let statuses = manager.apply([Some("Ctrl+Alt+P"), Some("Ctrl+Alt+Q"), None, None]);
        assert_eq!(fake.take(), [reg("Ctrl+Alt+Q"), unreg("Ctrl+Alt+R")]);
        assert_eq!(statuses[0], active("Ctrl+Alt+P"));
        assert_eq!(statuses[1].effective, None);
        assert_eq!(statuses[1].state, HotkeyState::Failed);
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+P")),
            Some(HotkeyAction::LogToggle)
        );
    }

    #[test]
    fn hotkey_change_resets_pressed_state() {
        let dispatch = Dispatch::default();
        let mut manager = HotkeyManager::new(FakeRegistrar::default(), dispatch.clone());
        manager.apply([Some("Ctrl+Alt+R"), None, None, None]);
        let r = key("Ctrl+Alt+R");
        assert!(dispatch.press(r, true).is_some());
        assert!(dispatch.press(r, true).is_none());
        // Removed while held, then bound again: it starts from released.
        manager.apply([Some("Ctrl+Alt+T"), None, None, None]);
        assert!(dispatch.press(r, true).is_none());
        manager.apply([Some("Ctrl+Alt+R"), None, None, None]);
        assert!(dispatch.press(r, true).is_some());
    }

    fn settings(toggle: &str, pause: &str) -> HotkeyRequest {
        HotkeyRequest::Settings([Some(toggle.to_owned()), Some(pause.to_owned()), None, None])
    }

    #[test]
    fn suspended_presses_do_nothing() {
        let dispatch = Dispatch::default();
        let mut manager = HotkeyManager::new(FakeRegistrar::default(), dispatch.clone());
        manager.apply([Some("Ctrl+Alt+R"), None, None, None]);
        let r = key("Ctrl+Alt+R");
        dispatch.set_suspended(true);
        assert_eq!(dispatch.press(r, true), None);
        dispatch.press(r, false);
        dispatch.set_suspended(false);
        assert_eq!(
            dispatch.press(r, true).map(|press| press.action),
            Some(HotkeyAction::LogToggle)
        );
    }

    #[test]
    fn suspension_releases_and_resume_registers_again() {
        let (mut manager, fake) = manager();
        let statuses = manager.handle([settings("Ctrl+Alt+R", "Ctrl+Alt+P")]);
        assert_eq!(fake.take(), [reg("Ctrl+Alt+R"), reg("Ctrl+Alt+P")]);
        assert_eq!(statuses.unwrap()[0], active("Ctrl+Alt+R"));

        // Suspended: ours are released so the capture box sees them, and
        // the published statuses stay as they were.
        assert_eq!(manager.handle([HotkeyRequest::Suspend(true)]), None);
        assert_eq!(fake.take(), [unreg("Ctrl+Alt+R"), unreg("Ctrl+Alt+P")]);
        assert_eq!(manager.action_for(key("Ctrl+Alt+R")), None);

        // A change while suspended is only remembered.
        assert_eq!(manager.handle([settings("Ctrl+Alt+R", "Ctrl+Alt+Q")]), None);
        assert_eq!(fake.take(), []);

        // Resumed: the latest request is registered.
        let statuses = manager.handle([HotkeyRequest::Suspend(false)]).unwrap();
        assert_eq!(fake.take(), [reg("Ctrl+Alt+R"), reg("Ctrl+Alt+Q")]);
        assert_eq!(statuses[0], active("Ctrl+Alt+R"));
        assert_eq!(statuses[1], active("Ctrl+Alt+Q"));
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+Q")),
            Some(HotkeyAction::LogPause)
        );

        // Focus moving between two boxes ends suspended; a blur and focus
        // again in one batch changes nothing.
        manager.handle([HotkeyRequest::Suspend(false), HotkeyRequest::Suspend(true)]);
        assert_eq!(fake.take(), [unreg("Ctrl+Alt+R"), unreg("Ctrl+Alt+Q")]);
        manager.handle([HotkeyRequest::Suspend(false)]);
        fake.take();
        manager.handle([HotkeyRequest::Suspend(true), HotkeyRequest::Suspend(false)]);
        assert_eq!(fake.take(), []);
    }

    #[test]
    fn main_window_leaving_resumes_the_hotkeys() {
        use tauri::WindowEvent;
        assert!(window_event_resumes(&WindowEvent::Focused(false)));
        assert!(window_event_resumes(&WindowEvent::Destroyed));
        assert!(!window_event_resumes(&WindowEvent::Focused(true)));
        assert!(!window_event_resumes(&WindowEvent::Resized(
            tauri::PhysicalSize::new(800, 600)
        )));
    }

    #[test]
    fn control_suspends_the_dispatch_at_once() {
        let dispatch = Dispatch::default();
        let mut manager = HotkeyManager::new(FakeRegistrar::default(), dispatch.clone());
        manager.apply([Some("Ctrl+Alt+R"), None, None, None]);
        let (sender, requests) = channel();
        let control = HotkeyControl::new(dispatch.clone(), sender);
        control.set_suspended(true);
        // Before the hotkeys thread has released anything.
        assert_eq!(dispatch.press(key("Ctrl+Alt+R"), true), None);
        assert_eq!(requests.try_recv(), Ok(HotkeyRequest::Suspend(true)));
        control.set_suspended(false);
        assert_eq!(requests.try_recv(), Ok(HotkeyRequest::Suspend(false)));
        assert!(dispatch.press(key("Ctrl+Alt+R"), true).is_some());
    }

    #[test]
    fn unreadable_hotkey_fails_without_registering() {
        let (mut manager, fake) = manager();
        let statuses = manager.apply([Some("Ctrl+Nope"), None, None, None]);
        assert_eq!(fake.take(), []);
        assert_eq!(statuses[0].state, HotkeyState::Failed);
        assert_eq!(statuses[0].reason.as_deref(), Some("log.hotkey.failed"));
    }

    #[test]
    fn press_filter_needs_a_release_between_presses() {
        let mut filter = PressFilter::default();
        let accepted: Vec<bool> = [true, true, false, false, true, false, true]
            .into_iter()
            .map(|pressed| filter.accept(pressed))
            .collect();
        assert_eq!(accepted, [true, false, false, false, true, false, true]);
    }

    #[test]
    fn plugin_errors_map_to_in_use() {
        assert_eq!(
            RegisterError::from_plugin("HotKey already registered: HotKey { .. }"),
            RegisterError::InUse
        );
        assert_eq!(
            RegisterError::from_plugin("Hot key is already registered. (os error 1409)"),
            RegisterError::InUse
        );
        assert_eq!(
            RegisterError::from_plugin("Access is denied. (os error 5)"),
            RegisterError::Other("Access is denied. (os error 5)".to_owned())
        );
        assert_eq!(RegisterError::InUse.reason(), "log.hotkey.inUse");
        assert_eq!(
            RegisterError::Other(String::new()).reason(),
            "log.hotkey.failed"
        );
    }

    #[test]
    fn every_key_converts_to_a_shortcut() {
        let mut texts: Vec<String> = ('A'..='Z').map(|c| format!("Ctrl+Alt+{c}")).collect();
        texts.extend((0..=9).map(|d| format!("Alt+Shift+{d}")));
        texts.extend((1..=24).map(|n| format!("Ctrl+Alt+Shift+F{n}")));
        for text in texts {
            let shortcut = shortcut_of(key(&text));
            // The plugin's own parser spells the same combination.
            let parsed: Shortcut = text.parse().unwrap();
            assert_eq!(shortcut, Some(parsed), "{text}");
        }
    }

    #[test]
    fn toggle_and_pause_actions_follow_the_log_state() {
        use HotkeyAction::{LogPause as Pause, LogToggle as Toggle};
        use LogState::{Error, Idle, Paused, Recording};
        assert_eq!(command_for(Toggle, Idle), Some(LogCommand::Start));
        assert_eq!(command_for(Toggle, Error), Some(LogCommand::Start));
        assert_eq!(command_for(Toggle, Recording), Some(LogCommand::Stop));
        assert_eq!(command_for(Toggle, Paused), Some(LogCommand::Stop));
        assert_eq!(command_for(Pause, Recording), Some(LogCommand::Pause));
        assert_eq!(command_for(Pause, Paused), Some(LogCommand::Resume));
        assert_eq!(command_for(Pause, Idle), None);
        assert_eq!(command_for(Pause, Error), None);
    }

    fn status(state: LogState, path: Option<&str>, error: Option<LogError>) -> LogStatus {
        LogStatus {
            revision: 1,
            state,
            session: 1,
            path: path.map(str::to_owned),
            part: 1,
            part_bytes: 0,
            recorded_ms: 0,
            rows: 0,
            bytes: 0,
            dropped: 0,
            error,
            hotkeys: HotkeyStatuses::default(),
        }
    }

    #[test]
    fn hotkey_commands_toast_their_outcome() {
        let path = r"C:\logs\oma-20260930-101500.csv";
        assert_eq!(
            outcome_toast(
                Lang::En,
                LogCommand::Start,
                &status(LogState::Recording, Some(path), None)
            ),
            Some((
                "Recording started".to_owned(),
                "oma-20260930-101500.csv".to_owned()
            ))
        );
        assert_eq!(
            outcome_toast(
                Lang::It,
                LogCommand::Stop,
                &status(LogState::Idle, Some(path), None)
            )
            .map(|(title, _)| title),
            Some("Registrazione terminata".to_owned())
        );
        // A failure shows the reason, never a confirmation.
        let full = LogError {
            key: "log.error.diskFull".to_owned(),
            detail: None,
        };
        assert_eq!(
            outcome_toast(
                Lang::En,
                LogCommand::Start,
                &status(LogState::Error, None, Some(full))
            ),
            Some(("Recording stopped".to_owned(), "Disk full".to_owned()))
        );
        // Pause and resume that went through: no toast.
        assert_eq!(
            outcome_toast(
                Lang::En,
                LogCommand::Pause,
                &status(LogState::Paused, Some(path), None)
            ),
            None
        );
    }

    type Seen = Arc<Mutex<Vec<Requested>>>;

    fn texts(texts: [Option<&str>; 4]) -> Requested {
        texts.map(|text| text.map(str::to_owned))
    }

    #[test]
    fn hotkeys_follow_settings_changes() {
        let store = Arc::new(open_fast(&FakeFs::new()));
        let seen: Seen = Arc::default();
        let sink = Arc::clone(&seen);
        follow_hotkeys(&store, move |requested| {
            sink.lock().unwrap().push(requested)
        });
        assert_eq!(
            *seen.lock().unwrap(),
            [texts([Some("Ctrl+Alt+Shift+R"), None, None, None])],
            "the initial catch-up"
        );
        store
            .update(&json!({"log": {"hotkeyPause": "Ctrl+Alt+P"}}))
            .unwrap();
        wait_until("the new pause hotkey", || seen.lock().unwrap().len() == 2);
        assert_eq!(
            seen.lock().unwrap()[1],
            texts([Some("Ctrl+Alt+Shift+R"), Some("Ctrl+Alt+P"), None, None])
        );
        // The overlay's hotkeys follow too; the benchmark one is not
        // registered before M7d (DP11).
        store
            .update(&json!({"overlay": {
                "hotkeyToggle": "Ctrl+Alt+F1",
                "hotkeyNextProfile": "Ctrl+Alt+F2",
                "hotkeyBenchmark": "Ctrl+Alt+F3"
            }}))
            .unwrap();
        wait_until("the overlay hotkeys", || seen.lock().unwrap().len() == 3);
        assert_eq!(
            seen.lock().unwrap()[2],
            texts([
                Some("Ctrl+Alt+Shift+R"),
                Some("Ctrl+Alt+P"),
                Some("Ctrl+Alt+F1"),
                Some("Ctrl+Alt+F2")
            ])
        );
        // Other settings do not touch the hotkeys.
        store
            .update(&json!({"general": {"temperatureUnit": "f"}}))
            .unwrap();
        store
            .update(&json!({"overlay": {"hotkeyBenchmark": "Ctrl+Alt+F4"}}))
            .unwrap();
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[test]
    fn log_hotkeys_unchanged() {
        // The log's hotkeys behave as before with the overlay's unset: the
        // default toggle alone, with the same registration and statuses.
        let (mut manager, fake) = manager();
        let log = Settings::default().log;
        let statuses = manager.apply([
            log.hotkey_toggle.as_deref(),
            log.hotkey_pause.as_deref(),
            None,
            None,
        ]);
        assert_eq!(fake.take(), [reg("Ctrl+Alt+Shift+R")]);
        assert_eq!(statuses[0], active("Ctrl+Alt+Shift+R"));
        assert_eq!(statuses[1], HotkeyStatus::default());
        assert_eq!(statuses[2], HotkeyStatus::default());
        assert_eq!(statuses[3], HotkeyStatus::default());
        let (log_statuses, overlay) = split_statuses(statuses);
        assert_eq!(log_statuses.toggle, active("Ctrl+Alt+Shift+R"));
        assert_eq!(log_statuses.pause, HotkeyStatus::default());
        assert_eq!(overlay, [HotkeyStatus::default(), HotkeyStatus::default()]);
        // Adding an overlay hotkey leaves the log's registration alone.
        let statuses = manager.apply([Some("Ctrl+Alt+Shift+R"), None, Some("Ctrl+Alt+F1"), None]);
        assert_eq!(fake.take(), [reg("Ctrl+Alt+F1")]);
        assert_eq!(statuses[0], active("Ctrl+Alt+Shift+R"));
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+Shift+R")),
            Some(HotkeyAction::LogToggle)
        );
        // The overlay's actions are no log commands.
        assert_eq!(
            command_for(HotkeyAction::OverlayToggle, LogState::Idle),
            None
        );
        assert_eq!(
            command_for(HotkeyAction::OverlayNextProfile, LogState::Recording),
            None
        );
    }

    #[test]
    fn four_actions_register_and_dispatch() {
        let dispatch = Dispatch::default();
        let fake = FakeRegistrar::default();
        let mut manager = HotkeyManager::new(fake.clone(), dispatch.clone());
        let combos = ["Ctrl+Alt+R", "Ctrl+Alt+P", "Ctrl+Alt+F1", "Ctrl+Alt+F2"];
        let statuses = manager.apply(combos.map(Some));
        assert_eq!(fake.take(), combos.map(reg));
        assert_eq!(statuses, combos.map(active));
        for (action, combo) in HotkeyAction::ALL.into_iter().zip(combos) {
            assert_eq!(action, HotkeyAction::ALL[action.index()]);
            assert_eq!(
                dispatch.press(key(combo), true).map(|press| press.action),
                Some(action),
                "{combo}"
            );
        }
        // One combination, one action: a later action loses it (DP14).
        fake.refuse("Ctrl+Alt+F9", RegisterError::InUse);
        let statuses = manager.apply([
            Some("Ctrl+Alt+R"),
            Some("Ctrl+Alt+P"),
            Some("Ctrl+Alt+P"),
            Some("Ctrl+Alt+F9"),
        ]);
        assert_eq!(fake.take(), [reg("Ctrl+Alt+F9")]);
        assert_eq!(statuses[1], active("Ctrl+Alt+P"));
        assert_eq!(statuses[2].state, HotkeyState::Failed);
        assert_eq!(statuses[2].reason.as_deref(), Some("log.hotkey.inUse"));
        // Its old combination stays its own: nobody else took it.
        assert_eq!(statuses[2].effective.as_deref(), Some("Ctrl+Alt+F1"));
        // A failed registration keeps the previous combination.
        assert_eq!(statuses[3].state, HotkeyState::Failed);
        assert_eq!(statuses[3].effective.as_deref(), Some("Ctrl+Alt+F2"));
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+P")),
            Some(HotkeyAction::LogPause)
        );
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+F2")),
            Some(HotkeyAction::OverlayNextProfile)
        );
        // Released, so a new press would act...
        dispatch.press(key("Ctrl+Alt+F1"), false);
        // ...but suspension releases all four (DP14) and nothing acts.
        assert_eq!(manager.handle([HotkeyRequest::Suspend(true)]), None);
        assert_eq!(
            fake.take(),
            ["Ctrl+Alt+R", "Ctrl+Alt+P", "Ctrl+Alt+F1", "Ctrl+Alt+F2"].map(unreg)
        );
        assert_eq!(dispatch.press(key("Ctrl+Alt+F1"), true), None);
    }

    #[test]
    fn swap_between_log_and_overlay_does_not_reregister() {
        let dispatch = Dispatch::default();
        let fake = FakeRegistrar::default();
        let mut manager = HotkeyManager::new(fake.clone(), dispatch.clone());
        manager.apply([Some("Ctrl+Alt+R"), None, Some("Ctrl+Alt+F1"), None]);
        fake.take();
        let statuses = manager.apply([Some("Ctrl+Alt+F1"), None, Some("Ctrl+Alt+R"), None]);
        assert_eq!(fake.take(), []);
        assert_eq!(statuses[0], active("Ctrl+Alt+F1"));
        assert_eq!(statuses[2], active("Ctrl+Alt+R"));
        assert_eq!(
            dispatch
                .press(key("Ctrl+Alt+R"), true)
                .map(|press| press.action),
            Some(HotkeyAction::OverlayToggle)
        );
        assert_eq!(
            dispatch
                .press(key("Ctrl+Alt+F1"), true)
                .map(|press| press.action),
            Some(HotkeyAction::LogToggle)
        );
        // The log's combination moves to «next profile»: rebound only.
        let statuses = manager.apply([None, None, Some("Ctrl+Alt+R"), Some("Ctrl+Alt+F1")]);
        assert_eq!(fake.take(), []);
        assert_eq!(statuses[0], HotkeyStatus::default());
        assert_eq!(statuses[3], active("Ctrl+Alt+F1"));
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+F1")),
            Some(HotkeyAction::OverlayNextProfile)
        );
    }

    #[derive(Default)]
    struct FakeOverlay {
        calls: Mutex<Vec<&'static str>>,
        hotkeys: Mutex<Vec<(HotkeyStatus, HotkeyStatus)>>,
    }

    impl OverlayActions for FakeOverlay {
        fn toggle_hidden(&self) {
            self.calls.lock().unwrap().push("toggle_hidden");
        }

        fn next_profile(&self) {
            self.calls.lock().unwrap().push("next_profile");
        }

        fn set_hotkeys(&self, toggle: HotkeyStatus, next_profile: HotkeyStatus) {
            self.hotkeys.lock().unwrap().push((toggle, next_profile));
        }
    }

    #[test]
    fn overlay_toggle_press_goes_to_the_controller() {
        let overlay = FakeOverlay::default();
        let press = |action| Press {
            action,
            generation: 1,
        };
        // Overlay presses go to the controller, never to the log worker.
        assert_eq!(
            route_press(press(HotkeyAction::OverlayToggle), Some(&overlay)),
            None
        );
        assert_eq!(
            route_press(press(HotkeyAction::OverlayNextProfile), Some(&overlay)),
            None
        );
        assert_eq!(
            *overlay.calls.lock().unwrap(),
            ["toggle_hidden", "next_profile"]
        );
        // Log presses go on to the log worker untouched.
        for action in [HotkeyAction::LogToggle, HotkeyAction::LogPause] {
            assert_eq!(
                route_press(press(action), Some(&overlay)),
                Some(press(action))
            );
        }
        assert_eq!(overlay.calls.lock().unwrap().len(), 2);
        // Without an overlay (off Windows) its presses do nothing.
        assert_eq!(route_press(press(HotkeyAction::OverlayToggle), None), None);
    }

    #[test]
    fn statuses_go_to_the_log_and_the_overlay() {
        let statuses = ["Ctrl+Alt+R", "Ctrl+Alt+P", "Ctrl+Alt+F1", "Ctrl+Alt+F2"].map(active);
        let (log, overlay) = split_statuses(statuses.clone());
        assert_eq!(log.toggle, active("Ctrl+Alt+R"));
        assert_eq!(log.pause, active("Ctrl+Alt+P"));
        assert_eq!(overlay, [active("Ctrl+Alt+F1"), active("Ctrl+Alt+F2")]);
        let fake = FakeOverlay::default();
        let log = publish_statuses(statuses, Some(&fake));
        assert_eq!(log.toggle, active("Ctrl+Alt+R"));
        assert_eq!(
            *fake.hotkeys.lock().unwrap(),
            [(active("Ctrl+Alt+F1"), active("Ctrl+Alt+F2"))]
        );
    }
}
