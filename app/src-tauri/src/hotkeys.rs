//! Global hotkeys of the CSV log (spec M5 §4.6, L10): registered with
//! `tauri-plugin-global-shortcut` from Rust only (no capability reaches
//! JavaScript), swapped safely, filtered against key repeat and reported in
//! `LogStatus.hotkeys`.
//!
//! Threads: the plugin runs `RegisterHotKey` on the main thread and waits
//! for it, so registrations run on a thread of their own, never on a
//! settings listener's or the main thread. Presses arrive on the main thread
//! (and releases on the plugin's threads); they only read the bindings and
//! queue the command for a worker, which waits for the writer.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use oma_core::hotkey::{parse_hotkey, Hotkey, HotkeyKey};
use oma_core::settings::Settings;
use tauri::{AppHandle, Manager, Wry};
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

/// What a hotkey of the log does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    Toggle,
    Pause,
}

impl HotkeyAction {
    const ALL: [Self; 2] = [Self::Toggle, Self::Pause];

    fn index(self) -> usize {
        match self {
            Self::Toggle => 0,
            Self::Pause => 1,
        }
    }
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
    actions: [Option<Hotkey>; 2],
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

    fn publish(&self, actions: [Option<Hotkey>; 2]) {
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

/// Owns the registered combinations of both actions.
pub struct HotkeyManager<R: HotkeyRegistrar> {
    registrar: R,
    /// Registered by us, by [`HotkeyAction::index`]; never the same twice.
    effective: [Option<Hotkey>; 2],
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
            effective: [None; 2],
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
    ) -> Option<HotkeyStatuses> {
        for request in requests {
            match request {
                HotkeyRequest::Settings(requested) => self.requested = Some(requested),
                HotkeyRequest::Suspend(suspended) => self.suspended = suspended,
            }
        }
        if self.suspended {
            if self.effective != [None; 2] {
                self.apply(None, None);
            }
            return None;
        }
        let (toggle, pause) = self.requested.clone()?;
        Some(self.apply(toggle.as_deref(), pause.as_deref()))
    }

    /// Registers the new combination first, releases the old one only after
    /// success (spec M5 §4.6). Both actions are reconciled together: a
    /// combination we own already is rebound without registering it again
    /// (also when toggle and pause swap), and an old one still effective for
    /// the other action is not released.
    pub fn apply(&mut self, toggle: Option<&str>, pause: Option<&str>) -> HotkeyStatuses {
        let texts = [toggle, pause];
        let old = self.effective;
        // Per action: the combination to bind (`None` = unset) or the
        // reason it cannot be.
        let mut outcome: [Result<Option<Hotkey>, &'static str>; 2] = texts.map(|text| {
            text.map_or(Ok(None), |text| {
                parse_hotkey(text).map(Some).map_err(|err| {
                    tracing::warn!(hotkey = text, ?err, "unreadable log hotkey");
                    "log.hotkey.failed"
                })
            })
        });
        // The settings never give both the same combination; if they did,
        // pause would lose it.
        if let [Ok(Some(toggle)), Ok(Some(pause))] = outcome {
            if toggle == pause {
                outcome[1] = Err("log.hotkey.inUse");
            }
        }
        for action in HotkeyAction::ALL {
            let slot = &mut outcome[action.index()];
            let Ok(Some(hotkey)) = *slot else { continue };
            if old.contains(&Some(hotkey)) {
                continue;
            }
            if let Err(err) = self.registrar.register(hotkey) {
                tracing::warn!(?action, %hotkey, ?err, "cannot register the log hotkey");
                *slot = Err(err.reason());
            }
        }
        let mut effective = outcome.map(|result| result.ok().flatten());
        // A failed action keeps its previous combination, unless the other
        // action has just taken it.
        for action in HotkeyAction::ALL {
            let i = action.index();
            if outcome[i].is_err() && old[i].is_some() && old[i] != effective[1 - i] {
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
        let status = |action: HotkeyAction| {
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
        };
        HotkeyStatuses {
            toggle: status(HotkeyAction::Toggle),
            pause: status(HotkeyAction::Pause),
        }
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

/// What `action` does in `state`, if anything.
pub fn command_for(action: HotkeyAction, state: LogState) -> Option<LogCommand> {
    match (action, state) {
        (HotkeyAction::Toggle, LogState::Idle | LogState::Error) => Some(LogCommand::Start),
        (HotkeyAction::Toggle, LogState::Recording | LogState::Paused) => Some(LogCommand::Stop),
        (HotkeyAction::Pause, LogState::Recording) => Some(LogCommand::Pause),
        (HotkeyAction::Pause, LogState::Paused) => Some(LogCommand::Resume),
        (HotkeyAction::Pause, LogState::Idle | LogState::Error) => None,
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

type Requested = (Option<String>, Option<String>);

fn requested(settings: &Settings) -> Requested {
    (
        settings.log.hotkey_toggle.clone(),
        settings.log.hotkey_pause.clone(),
    )
}

/// Calls `apply` with the requested combinations once right after
/// subscribing and then on every change of them. Unlike `rules.rs`, the
/// catch-up reads the store after taking the dedup lock, so it can never
/// apply a value older than one a listener applied before it.
fn follow_hotkeys(
    store: &Arc<SettingsStore>,
    apply: impl Fn(Option<String>, Option<String>) + Send + Sync + 'static,
) {
    let seen: Mutex<Option<Requested>> = Mutex::new(None);
    let offer = move |read: &dyn Fn() -> Requested| {
        let mut seen = lock(&seen);
        let now = read();
        if seen.as_ref() == Some(&now) {
            return;
        }
        apply(now.0.clone(), now.1.clone());
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
            tracing::warn!(%hotkey, %err, "cannot release the log hotkey");
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

/// Suspends the log hotkeys while a capture box in the settings has focus
/// (managed state of [`set_log_hotkeys_suspended`]).
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
/// captured instead of acted on.
#[tauri::command]
pub fn set_log_hotkeys_suspended(app: AppHandle, suspended: bool) {
    if let Some(control) = app.try_state::<HotkeyControl>() {
        control.set_suspended(suspended);
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

/// From here on `log.hotkeyToggle` and `log.hotkeyPause` drive the global
/// hotkeys; their state goes to `LogStatus.hotkeys`. The plugin must be
/// registered on the builder (`main.rs`) and the shared toaster managed.
pub fn install_hotkeys(app: &AppHandle, store: &Arc<SettingsStore>, log: Arc<LogService>) {
    let dispatch = Dispatch::default();
    let (presses, pressed) = channel::<Press>();
    let on_event: Arc<dyn Fn(Hotkey, bool) + Send + Sync> = {
        let dispatch = dispatch.clone();
        Arc::new(move |hotkey, down| {
            if let Some(press) = dispatch.press(hotkey, down) {
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
                status_log.set_hotkeys(statuses);
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
    follow_hotkeys(store, move |toggle, pause| {
        let _ = requests.send(HotkeyRequest::Settings((toggle, pause)));
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
        let statuses = manager.apply(log.hotkey_toggle.as_deref(), log.hotkey_pause.as_deref());
        assert_eq!(fake.take(), [reg("Ctrl+Alt+Shift+R")]);
        assert_eq!(statuses.toggle, active("Ctrl+Alt+Shift+R"));
        assert_eq!(statuses.pause, HotkeyStatus::default());
        assert_eq!(statuses.pause.state, HotkeyState::Unset);
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+Shift+R")),
            Some(HotkeyAction::Toggle)
        );
    }

    #[test]
    fn change_registers_new_before_releasing_old() {
        let (mut manager, fake) = manager();
        manager.apply(Some("Ctrl+Alt+R"), None);
        fake.take();
        let statuses = manager.apply(Some("Ctrl+Alt+T"), None);
        assert_eq!(fake.take(), [reg("Ctrl+Alt+T"), unreg("Ctrl+Alt+R")]);
        assert_eq!(statuses.toggle, active("Ctrl+Alt+T"));
        assert_eq!(manager.action_for(key("Ctrl+Alt+R")), None);
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+T")),
            Some(HotkeyAction::Toggle)
        );
    }

    #[test]
    fn failed_change_keeps_the_old_combination() {
        let (mut manager, fake) = manager();
        manager.apply(Some("Ctrl+Alt+R"), None);
        fake.refuse("Ctrl+Alt+T", RegisterError::InUse);
        fake.take();
        let statuses = manager.apply(Some("Ctrl+Alt+T"), None);
        // The old one is not released.
        assert_eq!(fake.take(), [reg("Ctrl+Alt+T")]);
        assert_eq!(
            statuses.toggle,
            HotkeyStatus {
                requested: Some("Ctrl+Alt+T".to_owned()),
                effective: Some("Ctrl+Alt+R".to_owned()),
                state: HotkeyState::Failed,
                reason: Some("log.hotkey.inUse".to_owned()),
            }
        );
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+R")),
            Some(HotkeyAction::Toggle)
        );
        assert_eq!(manager.action_for(key("Ctrl+Alt+T")), None);
    }

    #[test]
    fn clearing_a_hotkey_releases_it() {
        let (mut manager, fake) = manager();
        manager.apply(Some("Ctrl+Alt+R"), Some("Ctrl+Alt+P"));
        fake.take();
        let statuses = manager.apply(Some("Ctrl+Alt+R"), None);
        assert_eq!(fake.take(), [unreg("Ctrl+Alt+P")]);
        assert_eq!(statuses.toggle, active("Ctrl+Alt+R"));
        assert_eq!(statuses.pause, HotkeyStatus::default());
        assert_eq!(manager.action_for(key("Ctrl+Alt+P")), None);
    }

    #[test]
    fn initial_failure_leaves_it_inactive() {
        let (mut manager, fake) = manager();
        fake.refuse("Ctrl+Alt+Shift+R", RegisterError::Other("boom".to_owned()));
        let statuses = manager.apply(Some("Ctrl+Alt+Shift+R"), None);
        assert_eq!(
            statuses.toggle,
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
        manager.apply(None, None);
        assert_eq!(fake.take(), []);
    }

    #[test]
    fn held_hotkey_acts_once() {
        let dispatch = Dispatch::default();
        let mut manager = HotkeyManager::new(FakeRegistrar::default(), dispatch.clone());
        manager.apply(Some("Ctrl+Alt+R"), Some("Ctrl+Alt+P"));
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
            Some(HotkeyAction::Pause)
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
        manager.apply(Some("Ctrl+Alt+R"), Some("Ctrl+Alt+P"));
        fake.take();
        let before = dispatch.press(key("Ctrl+Alt+R"), true).unwrap();
        assert_eq!(before.action, HotkeyAction::Toggle);

        let statuses = manager.apply(Some("Ctrl+Alt+P"), Some("Ctrl+Alt+R"));
        // Both are owned already: rebound, not registered twice nor released.
        assert_eq!(fake.take(), []);
        assert_eq!(statuses.toggle, active("Ctrl+Alt+P"));
        assert_eq!(statuses.pause, active("Ctrl+Alt+R"));
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+P")),
            Some(HotkeyAction::Toggle)
        );
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+R")),
            Some(HotkeyAction::Pause)
        );
        // The press captured before the swap keeps its action.
        assert_eq!(before.action, HotkeyAction::Toggle);
        dispatch.press(key("Ctrl+Alt+R"), false);
        let after = dispatch.press(key("Ctrl+Alt+R"), true).unwrap();
        assert_eq!(after.action, HotkeyAction::Pause);
        assert!(after.generation > before.generation);

        // A swap where one side is new: the other is kept, not released.
        let statuses = manager.apply(Some("Ctrl+Alt+R"), Some("Ctrl+Alt+Q"));
        assert_eq!(fake.take(), [reg("Ctrl+Alt+Q"), unreg("Ctrl+Alt+P")]);
        assert_eq!(statuses.toggle, active("Ctrl+Alt+R"));
        assert_eq!(statuses.pause, active("Ctrl+Alt+Q"));
    }

    #[test]
    fn failed_change_does_not_keep_a_combination_the_other_action_took() {
        let (mut manager, fake) = manager();
        manager.apply(Some("Ctrl+Alt+R"), Some("Ctrl+Alt+P"));
        fake.refuse("Ctrl+Alt+Q", RegisterError::InUse);
        fake.take();
        // Toggle takes pause's combination; pause's new one fails.
        let statuses = manager.apply(Some("Ctrl+Alt+P"), Some("Ctrl+Alt+Q"));
        assert_eq!(fake.take(), [reg("Ctrl+Alt+Q"), unreg("Ctrl+Alt+R")]);
        assert_eq!(statuses.toggle, active("Ctrl+Alt+P"));
        assert_eq!(statuses.pause.effective, None);
        assert_eq!(statuses.pause.state, HotkeyState::Failed);
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+P")),
            Some(HotkeyAction::Toggle)
        );
    }

    #[test]
    fn hotkey_change_resets_pressed_state() {
        let dispatch = Dispatch::default();
        let mut manager = HotkeyManager::new(FakeRegistrar::default(), dispatch.clone());
        manager.apply(Some("Ctrl+Alt+R"), None);
        let r = key("Ctrl+Alt+R");
        assert!(dispatch.press(r, true).is_some());
        assert!(dispatch.press(r, true).is_none());
        // Removed while held, then bound again: it starts from released.
        manager.apply(Some("Ctrl+Alt+T"), None);
        assert!(dispatch.press(r, true).is_none());
        manager.apply(Some("Ctrl+Alt+R"), None);
        assert!(dispatch.press(r, true).is_some());
    }

    fn settings(toggle: &str, pause: &str) -> HotkeyRequest {
        HotkeyRequest::Settings((Some(toggle.to_owned()), Some(pause.to_owned())))
    }

    #[test]
    fn suspended_presses_do_nothing() {
        let dispatch = Dispatch::default();
        let mut manager = HotkeyManager::new(FakeRegistrar::default(), dispatch.clone());
        manager.apply(Some("Ctrl+Alt+R"), None);
        let r = key("Ctrl+Alt+R");
        dispatch.set_suspended(true);
        assert_eq!(dispatch.press(r, true), None);
        dispatch.press(r, false);
        dispatch.set_suspended(false);
        assert_eq!(
            dispatch.press(r, true).map(|press| press.action),
            Some(HotkeyAction::Toggle)
        );
    }

    #[test]
    fn suspension_releases_and_resume_registers_again() {
        let (mut manager, fake) = manager();
        let statuses = manager.handle([settings("Ctrl+Alt+R", "Ctrl+Alt+P")]);
        assert_eq!(fake.take(), [reg("Ctrl+Alt+R"), reg("Ctrl+Alt+P")]);
        assert_eq!(statuses.unwrap().toggle, active("Ctrl+Alt+R"));

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
        assert_eq!(statuses.toggle, active("Ctrl+Alt+R"));
        assert_eq!(statuses.pause, active("Ctrl+Alt+Q"));
        assert_eq!(
            manager.action_for(key("Ctrl+Alt+Q")),
            Some(HotkeyAction::Pause)
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
    fn control_suspends_the_dispatch_at_once() {
        let dispatch = Dispatch::default();
        let mut manager = HotkeyManager::new(FakeRegistrar::default(), dispatch.clone());
        manager.apply(Some("Ctrl+Alt+R"), None);
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
        let statuses = manager.apply(Some("Ctrl+Nope"), None);
        assert_eq!(fake.take(), []);
        assert_eq!(statuses.toggle.state, HotkeyState::Failed);
        assert_eq!(statuses.toggle.reason.as_deref(), Some("log.hotkey.failed"));
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
        use HotkeyAction::{Pause, Toggle};
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

    type Seen = Arc<Mutex<Vec<(Option<String>, Option<String>)>>>;

    #[test]
    fn hotkeys_follow_settings_changes() {
        let store = Arc::new(open_fast(&FakeFs::new()));
        let seen: Seen = Arc::default();
        let sink = Arc::clone(&seen);
        follow_hotkeys(&store, move |toggle, pause| {
            sink.lock().unwrap().push((toggle, pause))
        });
        assert_eq!(
            *seen.lock().unwrap(),
            [(Some("Ctrl+Alt+Shift+R".to_owned()), None)],
            "the initial catch-up"
        );
        store
            .update(&json!({"log": {"hotkeyPause": "Ctrl+Alt+P"}}))
            .unwrap();
        wait_until("the new pause hotkey", || seen.lock().unwrap().len() == 2);
        assert_eq!(
            seen.lock().unwrap()[1],
            (
                Some("Ctrl+Alt+Shift+R".to_owned()),
                Some("Ctrl+Alt+P".to_owned())
            )
        );
        // Other settings do not touch the hotkeys.
        store
            .update(&json!({"general": {"temperatureUnit": "f"}}))
            .unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);
    }
}
