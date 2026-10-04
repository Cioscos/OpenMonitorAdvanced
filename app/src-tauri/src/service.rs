//! The sensor-service status shown by the shell: the anti-cheat preference
//! (kept in the settings store, `sources.antiCheat`), the three commands the
//! UI calls, and [`ServiceShell`], which owns the link to the service on
//! Windows and shares one toggle path between the `set_anti_cheat` command and
//! the tray's check item.
//!
//! Off Windows there is no service and no link: the commands still exist and
//! compile without importing any Win32 type, and always report
//! [`ServiceState::NotInstalled`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
#[cfg(any(windows, test))]
use std::time::Duration;

use oma_core::settings::Settings;
#[cfg(any(windows, test))]
use oma_ipc::Reconfiguration;
use oma_ipc::{ServiceState, ServiceStatus, SourceRequest};
use tauri::State;

use crate::settings::SettingsStore;
#[cfg(any(windows, test))]
use crate::settings::{Effect, EffectStatus};

/// How long a toggle waits for the store to save the preference.
#[cfg(any(windows, test))]
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// What every command answers off Windows: there is no service, so the
/// preference is never read or written. A cfg-independent function (no
/// `#[cfg(not(windows))]` on it) so the non-Windows behaviour is unit-tested
/// on any platform, including this Windows dev machine: `oma-app` cannot be
/// cross-checked for a non-Windows target here (Tauri's Linux build needs a
/// real sysroot with `libdbus-1-dev`/gtk, not just the Rust target — see the
/// Task 11 fix-round-1 report).
// Used by the `#[cfg(not(windows))]` `ServiceShell` impl below and, directly,
// by the tests: on this Windows machine only the tests reach it, hence the
// `allow` (see the module doc comment on why the non-Windows target itself
// cannot be built here).
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn not_installed_status() -> ServiceStatus {
    ServiceStatus::new(ServiceState::NotInstalled, None)
}

/// The `set_anti_cheat` command's body off Windows: `enabled` is ignored,
/// nothing is persisted, and the answer is always `Ok`.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn not_installed_set_anti_cheat(_enabled: bool) -> Result<ServiceStatus, String> {
    Ok(not_installed_status())
}

/// Event the sampler callback emits when the service status changes. Only
/// `main.rs`'s `#[cfg(windows)]` sampler callback reads this off the lib
/// itself (there is no status-change event to emit off Windows).
#[cfg_attr(not(windows), allow(dead_code))]
pub const EVENT_SERVICE: &str = "oma:service";

/// `%LOCALAPPDATA%\OpenMonitorAdvanced\service.json`, the M4 file that held
/// the anti-cheat flag; `None` without `LOCALAPPDATA` (never expected on a
/// real Windows session, but the shell must not panic on a stripped-down test
/// environment). Only the one-time migration into the settings reads it.
pub fn anti_cheat_path() -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local_app_data)
            .join("OpenMonitorAdvanced")
            .join("service.json"),
    )
}

/// The tray's anti-cheat check item, abstracted so [`ToggleState`] is
/// testable without a running Tauri app.
///
/// `refresh` runs from the settings-store listener, possibly on the settings
/// writer thread, so it must never block (see [`ToggleState`]).
///
/// `refresh` is given a way to read the flag's *current* value rather than
/// one fixed at the time it was scheduled: on Windows the real update must
/// run on the main thread (see the impl below), and by the time it gets
/// there a second, later toggle may already have changed the flag again —
/// reading fresh then, instead of showing whatever value was captured
/// earlier, converges on the right state regardless of scheduling order.
pub trait ToggleIndicator: Send + Sync {
    fn refresh(&self, current: Arc<dyn Fn() -> bool + Send + Sync>);
}

// `CheckMenuItem<Wry>` and `AppHandle` are portable Tauri types (not Win32),
// so this impl is not `#[cfg(windows)]`: `Arc::new(item) as Arc<dyn
// ToggleIndicator>` in `tray.rs` — which is not cfg-gated either — compiles
// on every platform. Off Windows, `ServiceShell::set_anti_cheat`'s
// `#[cfg(not(windows))]` impl never calls `ToggleState::set`, so the store
// never changes and a click's native, self-toggling checkbox state is never
// corrected back to the (always-`false`, off Windows) stored flag: the
// checkbox can drift from it permanently. Harmless, since off Windows
// the flag drives no real behaviour, but worth knowing before wiring this
// checkbox to anything else off Windows.
impl ToggleIndicator for tauri::menu::CheckMenuItem<tauri::Wry> {
    /// The checkbox follows the settings-store listener (as the link
    /// command does), which may run on any thread that changes the store, so
    /// `refresh` must never block. `CheckMenuItem::set_checked` would: it
    /// waits until the main thread runs it (`tauri::menu::run_item_main_thread!`),
    /// and a caller that holds a lock the main thread's own event handler
    /// needs would deadlock with it. `run_on_main_thread` posts the closure
    /// and returns at once — synchronously, if already on the main thread,
    /// since `tauri-runtime-wry` runs same-thread posts in place.
    fn refresh(&self, current: Arc<dyn Fn() -> bool + Send + Sync>) {
        let item = self.clone();
        let _ = self.app_handle().run_on_main_thread(move || {
            let _ = item.set_checked(current());
        });
    }
}

type TrayItem = Arc<Mutex<Option<Arc<dyn ToggleIndicator>>>>;

/// Hands the indicator a way to read the flag whenever it actually updates
/// the checkbox. Holds `tray_item` only long enough to clone the `Arc` out of
/// it, then calls into the indicator with the lock released.
fn refresh_indicator(tray_item: &Mutex<Option<Arc<dyn ToggleIndicator>>>, flag: &Arc<AtomicBool>) {
    let item = tray_item
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(item) = item {
        let flag = Arc::clone(flag);
        item.refresh(Arc::new(move || flag.load(Ordering::SeqCst)));
    }
}

/// What follows a change of the anti-cheat preference besides the tray
/// checkbox: the command to the service link. Must never block.
type LinkSink = Box<dyn Fn(bool) + Send + Sync>;

/// The anti-cheat preference: it lives in the settings store (`sources.antiCheat`),
/// and one store listener makes everything else follow it, so the tray, the
/// settings view, the `set_anti_cheat` command and the file cannot disagree.
/// On every change of the value, whichever way it was made, the listener
/// moves the tray checkbox and sends the new value to the service link.
///
/// The listener may run on any thread that changes the store, the settings
/// writer included, and always in store order. Both effects must therefore
/// never block: `ToggleIndicator::refresh` only posts to the main thread, and
/// the link sink only takes the link lock and sends on a channel.
///
/// It is portable, with no dependency on [`oma_win::svc::ServiceLink`] or a
/// Tauri app, so it is tested directly (see the `tests` module below).
struct ToggleState {
    store: Arc<SettingsStore>,
    /// Last value the listener saw, so only a change fires the effects. `Arc`
    /// so the reader handed to the indicator is `'static` and never needs
    /// `self`: it runs later, on the main thread, after every lock has been
    /// released.
    flag: Arc<AtomicBool>,
    tray_item: TrayItem,
}

impl ToggleState {
    fn new(store: Arc<SettingsStore>, link: LinkSink) -> Self {
        let flag = Arc::new(AtomicBool::new(store.settings().sources.anti_cheat));
        let tray_item: TrayItem = Arc::default();
        {
            let flag = Arc::clone(&flag);
            let tray_item = Arc::clone(&tray_item);
            store.subscribe(Box::new(move |settings, _| {
                let now = settings.sources.anti_cheat;
                if flag.swap(now, Ordering::SeqCst) != now {
                    refresh_indicator(&tray_item, &flag);
                    link(now);
                }
            }));
        }
        Self {
            store,
            flag,
            tray_item,
        }
    }

    fn enabled(&self) -> bool {
        self.store.settings().sources.anti_cheat
    }

    fn set_tray_item(&self, item: Arc<dyn ToggleIndicator>) {
        *self
            .tray_item
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(item);
        refresh_indicator(&self.tray_item, &self.flag);
    }

    /// Stores `enabled`; the checkbox and the link follow through the store
    /// listener. Then waits for the save. A failed save does not undo the
    /// mode: it stays applied and the store's persistence state shows the
    /// error.
    /// Off Windows only a test calls this (`ServiceShell::set_anti_cheat`'s
    /// `#[cfg(not(windows))]` impl never persists, so it never reaches here).
    #[cfg_attr(not(windows), allow(dead_code))]
    fn set(&self, enabled: bool) {
        self.store
            .update_with(|settings| settings.sources.anti_cheat = enabled);
        if let Err(reason) = self.store.flush_now(FLUSH_TIMEOUT) {
            tracing::warn!(%reason, "the anti-cheat preference is applied but not saved");
        }
    }
}

/// What the user chose, as the link and the provider take it: the names of
/// the service modules switched off, the core ids of the disks whose SMART is
/// off and of the default-off disks switched on. A disk in both lists is
/// switched off.
pub(crate) fn request_of(settings: &Settings) -> SourceRequest {
    let disabled = &settings.sources.smart_disabled_drives;
    SourceRequest {
        disabled_modules: settings
            .sources
            .service_modules
            .disabled()
            .into_iter()
            .map(str::to_owned)
            .collect(),
        smart_disabled_drives: disabled.clone(),
        smart_enabled_drives: settings
            .sources
            .smart_enabled_drives
            .iter()
            .filter(|id| !disabled.contains(id))
            .cloned()
            .collect(),
    }
}

/// What follows a change of the source request: the command to the service
/// link. Must never block.
type SourcesSink = Box<dyn Fn(SourceRequest) + Send + Sync>;

/// Sends the request to `sink` whenever `sources.serviceModules`,
/// `sources.smartDisabledDrives` or `sources.smartEnabledDrives` change. It keeps the last request it saw and
/// acts only when the new one differs; the listener gets only the new state,
/// may run on any thread that changes the store (the settings writer
/// included) and always in store order, so `sink` must be quick: it takes the
/// link lock and pushes on a channel.
///
/// A change made before a link exists reaches nobody here: the link is
/// spawned with the request read from the store under the same lock the sink
/// takes, so it is either read there or sent to the new link, never lost.
fn follow_sources(store: &Arc<SettingsStore>, sink: SourcesSink) {
    let last = Mutex::new(request_of(&store.settings()));
    store.subscribe(Box::new(move |settings, _| {
        let now = request_of(settings);
        let mut last = last.lock().unwrap_or_else(PoisonError::into_inner);
        if *last != now {
            last.clone_from(&now);
            sink(now);
        }
    }));
}

/// What the settings view shows for the service: nothing while there is no
/// service to ask, otherwise how the service stands with our request.
#[cfg(any(windows, test))]
fn service_effect(status: &ServiceStatus) -> EffectStatus {
    match status.sources.as_ref().map(|s| s.reconfiguration) {
        None => EffectStatus::Idle,
        Some(Reconfiguration::Applied) => EffectStatus::Applied,
        Some(Reconfiguration::Pending) => EffectStatus::Pending,
        Some(Reconfiguration::Failed) => EffectStatus::Failed {
            reason: "reconfigurationFailed".to_owned(),
        },
    }
}

/// Keeps `applyStatus.service` in step with the service status, on every
/// change and whether or not a window is open. The observer runs on the link
/// thread; `set_effect` only takes the store lock for a moment.
#[cfg(windows)]
fn follow_service_effect(store: &Arc<SettingsStore>, table: &oma_win::svc::ServiceStatusTable) {
    let store = Arc::clone(store);
    table.subscribe(Box::new(move |status| {
        store.set_effect(Effect::Service, service_effect(status));
    }));
}

/// Shared state the three commands and the tray read: the toggle above, and
/// on Windows the running link and the status table it writes.
pub struct ServiceShell {
    toggle: ToggleState,
    #[cfg(windows)]
    link: Arc<Mutex<Option<oma_win::svc::ServiceLink>>>,
    #[cfg(windows)]
    status_table: oma_win::svc::ServiceStatusTable,
}

impl ServiceShell {
    /// Reads the preference from the settings store. The link is not started here but by
    /// [`Self::spawn_link`], from Tauri's `.setup()` hook: `status_table` is
    /// the one `default_providers` hands to the `svc` provider through
    /// [`oma_win::ServiceHandles`], and the link writes it once running.
    #[cfg(windows)]
    pub fn new(store: Arc<SettingsStore>, status_table: oma_win::svc::ServiceStatusTable) -> Self {
        let link: Arc<Mutex<Option<oma_win::svc::ServiceLink>>> = Arc::default();
        let sink = Arc::clone(&link);
        let sources_sink = Arc::clone(&link);
        follow_sources(
            &store,
            Box::new(move |request| {
                send_to_link(
                    &sources_sink,
                    oma_win::svc::LinkCommand::SetSources(request),
                );
            }),
        );
        follow_service_effect(&store, &status_table);
        Self {
            toggle: ToggleState::new(
                store,
                Box::new(move |enabled| {
                    send_to_link(&sink, oma_win::svc::LinkCommand::SetAntiCheat(enabled));
                }),
            ),
            link,
            status_table,
        }
    }

    /// Starts the link to the service (once; later calls do nothing). Called
    /// from Tauri's `.setup()` hook, which only the surviving instance
    /// reaches: `tauri_plugin_single_instance` ends a second launch while the
    /// app is being built, so that process never probes or starts the
    /// service, nor connects to its pipe (final review M2). Holds the
    /// `link` lock while it reads the preferences (anti-cheat mode, sampling
    /// interval and the sources turned off), and the store listeners send
    /// under that same lock: a change is either read here or sent to the new
    /// link (at worst both, with the same value), never lost in between.
    /// `drives` is the table the link translates disk ids with.
    #[cfg(windows)]
    pub fn spawn_link(&self, feed: oma_win::svc::SvcFeed, drives: oma_win::storage::DriveIdTable) {
        use oma_win::svc::{pipe_connector, LinkSettings, ServiceLink, WindowsScm, SERVICE_NAME};

        let mut link = self.link.lock().unwrap_or_else(PoisonError::into_inner);
        if link.is_some() {
            return;
        }
        *link = Some(ServiceLink::spawn(
            Arc::new(WindowsScm::new(SERVICE_NAME)),
            pipe_connector(),
            {
                let settings = self.toggle.store.settings();
                LinkSettings {
                    sources: request_of(&settings),
                    drives,
                    ..LinkSettings::new(oma_ipc::PIPE_NAME, settings.general.interval_ms)
                }
            },
            self.toggle.enabled(),
            self.status_table.clone(),
            feed,
        ));
    }

    #[cfg(not(windows))]
    pub fn new(store: Arc<SettingsStore>) -> Self {
        Self {
            toggle: ToggleState::new(store, Box::new(|_| {})),
        }
    }

    /// What tells the service link about a new sampling interval (for
    /// [`crate::interval::follow_interval`]). It only takes the link lock and
    /// sends on a channel, so it never blocks. Off Windows there is no link.
    pub(crate) fn interval_sink(&self) -> Box<dyn Fn(u32) + Send + Sync> {
        #[cfg(windows)]
        {
            let link = Arc::clone(&self.link);
            Box::new(move |ms| send_to_link(&link, oma_win::svc::LinkCommand::SetInterval(ms)))
        }
        #[cfg(not(windows))]
        {
            Box::new(|_| {})
        }
    }

    pub(crate) fn set_tray_item(&self, item: Arc<dyn ToggleIndicator>) {
        self.toggle.set_tray_item(item);
    }

    pub(crate) fn anti_cheat_enabled(&self) -> bool {
        self.toggle.enabled()
    }
}

/// Sends `command` to the running link, if any. Only takes the `link` lock and
/// tries to push on a bounded channel, so the store listener may call it from
/// any thread: with the queue full the command is dropped and logged, never
/// waited for.
#[cfg(windows)]
fn send_to_link(
    link: &Mutex<Option<oma_win::svc::ServiceLink>>,
    command: oma_win::svc::LinkCommand,
) {
    if let Some(link) = link.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        if link.send(command.clone()).is_err() {
            tracing::warn!("service link queue full; {command:?} dropped");
        }
    }
}

/// Takes the value out of `slot` and hands it to `finish` with the lock
/// released: [`ServiceShell::shutdown`] joins the link thread in `finish`, and
/// that thread may be in a store listener that sends to the link (and so
/// takes this lock) until it stops. Holding the lock there would stall the
/// exit for the whole join wait.
#[cfg(any(windows, test))]
fn take_then<T>(slot: &Mutex<Option<T>>, finish: impl FnOnce(T)) {
    let value = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(value) = value {
        finish(value);
    }
}

#[cfg(windows)]
impl ServiceShell {
    pub(crate) fn status(&self) -> ServiceStatus {
        self.status_table.get().1
    }

    /// The version the last service that said `Hello` reported.
    pub(crate) fn service_version(&self) -> Option<String> {
        self.status_table.service_version()
    }

    /// The one toggle path: the `set_anti_cheat` command and the tray's
    /// check item both call this and nothing else. It changes the store and
    /// waits for the save; the tray checkbox and the link command follow the
    /// store's listener (`ToggleState`), as they do for a change made from the
    /// settings view. A failed save does not undo the mode, so this answers
    /// `Ok` either way; the store's persistence state carries the error.
    pub(crate) fn set_anti_cheat(&self, enabled: bool) -> Result<ServiceStatus, String> {
        self.toggle.set(enabled);
        Ok(self.status())
    }

    fn send_link(&self, command: oma_win::svc::LinkCommand) {
        send_to_link(&self.link, command);
    }

    pub(crate) fn start(&self) -> ServiceStatus {
        self.send_link(oma_win::svc::LinkCommand::Start);
        self.status()
    }

    /// Stops the link within its own join wait, or detaches it (see
    /// [`oma_win::svc::ServiceLink::shutdown`]).
    pub fn shutdown(&self) {
        take_then(&self.link, oma_win::svc::ServiceLink::shutdown);
    }
}

#[cfg(not(windows))]
impl ServiceShell {
    pub(crate) fn status(&self) -> ServiceStatus {
        not_installed_status()
    }

    /// There is no service off Windows.
    pub(crate) fn service_version(&self) -> Option<String> {
        None
    }

    /// Off Windows there is no service and no link: `enabled` is ignored,
    /// nothing is persisted, the answer is always `Ok(NotInstalled)`.
    pub(crate) fn set_anti_cheat(&self, enabled: bool) -> Result<ServiceStatus, String> {
        not_installed_set_anti_cheat(enabled)
    }

    pub(crate) fn start(&self) -> ServiceStatus {
        not_installed_status()
    }

    /// A no-op: there is no link to stop.
    pub fn shutdown(&self) {}
}

#[tauri::command(async)]
pub fn get_service_status(state: State<'_, ServiceShell>) -> ServiceStatus {
    state.status()
}

#[tauri::command(async)]
pub fn set_anti_cheat(
    state: State<'_, ServiceShell>,
    enabled: bool,
) -> Result<ServiceStatus, String> {
    state.set_anti_cheat(enabled)
}

#[tauri::command(async)]
pub fn start_service(state: State<'_, ServiceShell>) -> Result<ServiceStatus, String> {
    Ok(state.start())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::settings::fake_fs::{open_fast, stored_json, test_path, wait_until, FakeFs};
    use crate::settings::Persistence;

    #[derive(Default)]
    struct FakeIndicator {
        calls: Mutex<Vec<bool>>,
    }

    impl FakeIndicator {
        fn calls(&self) -> Vec<bool> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl ToggleIndicator for FakeIndicator {
        fn refresh(&self, current: Arc<dyn Fn() -> bool + Send + Sync>) {
            self.calls.lock().unwrap().push(current());
        }
    }

    type Sent = Arc<Mutex<Vec<bool>>>;

    fn new_store(fs: &Arc<FakeFs>) -> Arc<SettingsStore> {
        Arc::new(open_fast(fs))
    }

    /// A toggle whose link commands land in the returned list.
    fn toggle_with_sink(store: &Arc<SettingsStore>) -> (ToggleState, Sent) {
        let sent = Sent::default();
        let sink = Arc::clone(&sent);
        let toggle = ToggleState::new(
            Arc::clone(store),
            Box::new(move |enabled| sink.lock().unwrap().push(enabled)),
        );
        (toggle, sent)
    }

    fn snapshot<T: Clone>(list: &Mutex<Vec<T>>) -> Vec<T> {
        list.lock().unwrap().clone()
    }

    fn set_anti_cheat_in_store(store: &SettingsStore, enabled: bool) {
        store
            .update(&serde_json::json!({"sources": {"antiCheat": enabled}}))
            .unwrap();
    }

    #[test]
    fn anti_cheat_follows_the_store_and_defaults_off() {
        let fs = FakeFs::new();
        let store = new_store(&fs);
        let (toggle, _) = toggle_with_sink(&store);
        assert!(!toggle.enabled());
        set_anti_cheat_in_store(&store, true);
        assert!(toggle.enabled());

        let fs = FakeFs::new().with_file(
            &test_path(),
            br#"{"version":1,"sources":{"antiCheat":true}}"#,
        );
        assert!(toggle_with_sink(&new_store(&fs)).0.enabled());
    }

    #[test]
    fn anti_cheat_applies_even_when_the_flush_fails() {
        let fs = FakeFs::new();
        fs.fail_next_writes(usize::MAX);
        let store = new_store(&fs);
        let (toggle, sent) = toggle_with_sink(&store);
        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator.clone());
        assert_eq!(indicator.calls(), vec![false]);

        toggle.set(true);

        // The mode is on, in the link, in the store and on the checkbox.
        assert!(toggle.enabled());
        assert!(store.settings().sources.anti_cheat);
        wait_until("the link command and the checkbox", || {
            snapshot(&sent) == [true] && indicator.calls() == [false, true]
        });
        // The failed save shows in the persistence state instead.
        wait_until("the save error", || {
            matches!(store.state().persistence, Persistence::Error { .. })
        });
        assert_eq!(fs.file(&test_path()), None, "nothing was written");
    }

    #[test]
    fn tray_and_settings_view_agree_on_anti_cheat() {
        let fs = FakeFs::new();
        let store = new_store(&fs);
        let (toggle, sent) = toggle_with_sink(&store);
        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator.clone());

        // A change made from the settings view moves the checkbox and
        // reaches the service link, exactly like a tray click.
        set_anti_cheat_in_store(&store, true);
        wait_until("the change reaching the link and the checkbox", || {
            snapshot(&sent) == [true] && indicator.calls() == [false, true]
        });
        assert!(toggle.enabled());
        // An unrelated change leaves both alone.
        store
            .update(&serde_json::json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        set_anti_cheat_in_store(&store, false);
        wait_until("the second change", || {
            snapshot(&sent) == [true, false] && indicator.calls() == [false, true, false]
        });
    }

    #[test]
    fn setting_the_same_value_sends_no_link_command() {
        let fs = FakeFs::new();
        let store = new_store(&fs);
        let (toggle, sent) = toggle_with_sink(&store);
        toggle.set(false); // already off
        toggle.set(true);
        toggle.set(true);
        wait_until("the single command", || snapshot(&sent) == [true]);
        store.flush_now(Duration::from_secs(5)).unwrap();
        assert_eq!(snapshot(&sent), vec![true]);
    }

    #[test]
    fn tray_and_command_use_the_same_toggle_path() {
        let fs = FakeFs::new();
        let store = new_store(&fs);
        let (toggle, sent) = toggle_with_sink(&store);
        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator.clone());

        // Simulates the `set_anti_cheat` command: the caller passes the
        // desired value directly.
        toggle.set(true);
        assert!(toggle.enabled());
        assert_eq!(stored_json(&fs)["sources"]["antiCheat"], true);

        // Simulates the tray's check item: the caller computes the opposite
        // of the current preference. Both go through `ToggleState::set`.
        let requested = !toggle.enabled();
        toggle.set(requested);
        assert_eq!(toggle.enabled(), requested);
        assert_eq!(stored_json(&fs)["sources"]["antiCheat"], requested);

        wait_until("the checkbox and the link", || {
            indicator.calls() == [false, true, false] && snapshot(&sent) == [true, false]
        });
        assert_eq!(store.state().persistence, Persistence::Ok);
    }

    /// Final review M2: a second launch (the Start menu shortcut while the
    /// app sits in the tray) must never probe or start the service, nor
    /// connect to its pipe. `tauri_plugin_single_instance` ends that process
    /// while the app is being built, before the `.setup()` hook runs, so the
    /// link is spawned there and nowhere earlier. Checked on the source: a
    /// runtime check would drive this machine's real SCM.
    #[test]
    fn the_link_starts_only_in_the_setup_hook() {
        let main = include_str!("main.rs");
        let builder = main.find("tauri::Builder::default()").expect("the builder");
        let setup = main.find(".setup(move |app|").expect("the setup hook");
        let spawn = main.find(".spawn_link(").expect("spawn_link is called");
        assert!(
            builder < setup && setup < spawn,
            "spawn_link runs inside .setup()"
        );
        assert_eq!(main.matches(".spawn_link(").count(), 1);
        let before_builder = &main[..builder];
        assert!(
            !before_builder.contains("svc_feed,\n") && !before_builder.contains("ServiceLink"),
            "nothing before the builder hands the feed to a link"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_new_shell_has_no_link_until_it_is_spawned() {
        let shell = ServiceShell::new(
            new_store(&FakeFs::new()),
            oma_win::svc::ServiceStatusTable::default(),
        );
        assert!(shell.link.lock().unwrap().is_none());
        shell.shutdown(); // nothing to stop
    }

    #[test]
    fn not_installed_answers_ok_without_touching_persistence() {
        // The non-Windows behaviour of every command: exercised directly
        // here since `oma-app` cannot be cross-checked for a non-Windows
        // target on this machine (see the module doc comment).
        let fs = FakeFs::new();
        let store = new_store(&fs);

        assert_eq!(
            not_installed_set_anti_cheat(true),
            Ok(not_installed_status())
        );
        assert_eq!(not_installed_status().state, ServiceState::NotInstalled);
        assert_eq!(not_installed_status().detail, None);
        assert_eq!(store.state().revision, 0);
        assert_eq!(
            fs.write_attempts(),
            0,
            "the off-Windows answer never touches the filesystem"
        );
    }

    #[test]
    fn concurrent_toggles_serialize_to_a_consistent_final_state() {
        let fs = FakeFs::new();
        let store = new_store(&fs);
        let (toggle, sent) = toggle_with_sink(&store);
        let toggle = Arc::new(toggle);
        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator.clone());

        let handles: Vec<_> = (0..8u32)
            .map(|i| {
                let toggle = Arc::clone(&toggle);
                std::thread::spawn(move || {
                    for j in 0..50u32 {
                        toggle.set((i + j) % 2 == 0);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let final_flag = toggle.enabled();
        store.flush_now(Duration::from_secs(5)).unwrap();
        assert_eq!(
            stored_json(&fs)["sources"]["antiCheat"],
            final_flag,
            "the file and the in-memory flag must agree after every writer is done"
        );
        // Listeners run in store order, so the last link command and the last
        // checkbox update are the last stored value.
        wait_until("the link and the checkbox to settle", || {
            snapshot(&sent).last().copied() == Some(final_flag)
                && indicator.calls().last().copied() == Some(final_flag)
        });
    }

    // ---- source requests and the service effect ----

    fn sources_status(reconfiguration: Reconfiguration) -> ServiceStatus {
        ServiceStatus {
            state: ServiceState::Connected,
            detail: None,
            pawn_io: Some(oma_ipc::PawnIoStatus::Ok),
            sources: Some(oma_ipc::ServiceSources {
                active_modules: vec!["cpu".to_owned()],
                requested_disabled_modules: Vec::new(),
                smart_disabled_drives: Vec::new(),
                reconfiguration,
                drives: Vec::new(),
            }),
        }
    }

    #[test]
    fn service_effect_follows_the_reconfiguration() {
        assert_eq!(
            service_effect(&ServiceStatus::new(ServiceState::Unreachable, None)),
            EffectStatus::Idle,
            "no service, nothing to apply"
        );
        assert_eq!(
            service_effect(&sources_status(Reconfiguration::Pending)),
            EffectStatus::Pending
        );
        assert_eq!(
            service_effect(&sources_status(Reconfiguration::Applied)),
            EffectStatus::Applied
        );
        assert_eq!(
            service_effect(&sources_status(Reconfiguration::Failed)),
            EffectStatus::Failed {
                reason: "reconfigurationFailed".to_owned()
            }
        );
    }

    #[cfg(windows)]
    fn apply_status_of(store: &SettingsStore) -> EffectStatus {
        store.state().apply_status.service
    }

    #[cfg(windows)]
    #[test]
    fn apply_status_goes_pending_then_applied() {
        let store = new_store(&FakeFs::new());
        let table = oma_win::svc::ServiceStatusTable::default();
        follow_service_effect(&store, &table);
        assert_eq!(apply_status_of(&store), EffectStatus::Idle);

        table.set(&sources_status(Reconfiguration::Applied));
        assert_eq!(apply_status_of(&store), EffectStatus::Applied);
        table.set(&sources_status(Reconfiguration::Pending));
        assert_eq!(apply_status_of(&store), EffectStatus::Pending);
        table.set(&sources_status(Reconfiguration::Applied));
        assert_eq!(apply_status_of(&store), EffectStatus::Applied);

        // The service goes away: nothing is pending or applied any more.
        table.set(&ServiceStatus::new(ServiceState::Unreachable, None));
        assert_eq!(apply_status_of(&store), EffectStatus::Idle);
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_reconfiguration_shows_in_the_apply_status() {
        let store = new_store(&FakeFs::new());
        let table = oma_win::svc::ServiceStatusTable::default();
        follow_service_effect(&store, &table);
        table.set(&sources_status(Reconfiguration::Failed));
        assert_eq!(
            apply_status_of(&store),
            EffectStatus::Failed {
                reason: "reconfigurationFailed".to_owned()
            }
        );
    }

    #[test]
    fn take_then_releases_the_slot_before_finishing() {
        // The link thread may still send (the store listeners take this lock)
        // while shutdown joins it: the join must not hold the lock.
        let slot = Mutex::new(Some(1));
        let mut finished = None;
        take_then(&slot, |value| {
            assert!(slot.try_lock().is_ok(), "the slot is still locked");
            finished = Some(value);
        });
        assert_eq!(finished, Some(1));
        assert!(slot.lock().unwrap().is_none());

        take_then(&slot, |_: i32| {
            panic!("an empty slot has nothing to finish")
        });
    }

    type Requests = Arc<Mutex<Vec<SourceRequest>>>;

    fn follow_into(store: &Arc<SettingsStore>) -> Requests {
        let requests = Requests::default();
        let sink = Arc::clone(&requests);
        follow_sources(
            store,
            Box::new(move |request| sink.lock().unwrap().push(request)),
        );
        requests
    }

    #[test]
    fn service_modules_changes_are_sent_as_a_request() {
        let store = new_store(&FakeFs::new());
        let requests = follow_into(&store);
        store
            .update(
                &serde_json::json!({"sources": {"serviceModules": {"psu": false, "cpu": false}}}),
            )
            .unwrap();
        store
            .update(&serde_json::json!({"sources": {"serviceModules": {"psu": true}}}))
            .unwrap();
        assert_eq!(
            snapshot(&requests),
            vec![
                SourceRequest {
                    disabled_modules: vec!["cpu".to_owned(), "psu".to_owned()],
                    smart_disabled_drives: Vec::new(),
                    smart_enabled_drives: Vec::new(),
                },
                SourceRequest {
                    disabled_modules: vec!["cpu".to_owned()],
                    smart_disabled_drives: Vec::new(),
                    smart_enabled_drives: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn smart_disabled_drives_changes_are_sent_as_a_request() {
        let store = new_store(&FakeFs::new());
        let requests = follow_into(&store);
        store
            .update(&serde_json::json!({"sources": {"smartDisabledDrives": ["storage/device-a"]}}))
            .unwrap();
        assert_eq!(
            snapshot(&requests),
            vec![SourceRequest {
                disabled_modules: Vec::new(),
                smart_enabled_drives: Vec::new(),
                smart_disabled_drives: vec!["storage/device-a".to_owned()],
            }]
        );
    }

    #[test]
    fn smart_enabled_drives_changes_are_sent_as_a_request() {
        let store = new_store(&FakeFs::new());
        let requests = follow_into(&store);
        store
            .update(&serde_json::json!({"sources": {"smartEnabledDrives": ["storage/device-a"]}}))
            .unwrap();
        store
            .update(&serde_json::json!({"sources": {"smartEnabledDrives": []}}))
            .unwrap();
        assert_eq!(
            snapshot(&requests),
            vec![
                SourceRequest {
                    disabled_modules: Vec::new(),
                    smart_disabled_drives: Vec::new(),
                    smart_enabled_drives: vec!["storage/device-a".to_owned()],
                },
                SourceRequest::default(),
            ]
        );
    }

    #[test]
    fn an_id_in_both_lists_is_requested_as_disabled_only() {
        let mut settings = Settings::default();
        settings.sources.smart_disabled_drives = vec!["storage/device-a".to_owned()];
        settings.sources.smart_enabled_drives =
            vec!["storage/device-a".to_owned(), "storage/device-b".to_owned()];
        assert_eq!(
            request_of(&settings),
            SourceRequest {
                disabled_modules: Vec::new(),
                smart_disabled_drives: vec!["storage/device-a".to_owned()],
                smart_enabled_drives: vec!["storage/device-b".to_owned()],
            }
        );
    }

    #[test]
    fn unrelated_and_repeated_changes_send_no_request() {
        let store = new_store(&FakeFs::new());
        let requests = follow_into(&store);
        store
            .update(&serde_json::json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        set_anti_cheat_in_store(&store, true);
        // The same value again is no change.
        store
            .update(&serde_json::json!({"sources": {"serviceModules": {"psu": true}}}))
            .unwrap();
        assert!(snapshot(&requests).is_empty());
    }

    #[test]
    fn the_request_starts_from_the_stored_settings() {
        let fs = FakeFs::new().with_file(
            &test_path(),
            br#"{"version":1,"sources":{"serviceModules":{"memory":false},"smartDisabledDrives":["storage/x"]}}"#,
        );
        let store = new_store(&fs);
        assert_eq!(
            request_of(&store.settings()),
            SourceRequest {
                disabled_modules: vec!["memory".to_owned()],
                smart_enabled_drives: Vec::new(),
                smart_disabled_drives: vec!["storage/x".to_owned()],
            }
        );
        // Nothing changed since the listener started: nothing is sent.
        let requests = follow_into(&store);
        store
            .update(&serde_json::json!({"sources": {"serviceModules": {"memory": false}}}))
            .unwrap();
        assert!(snapshot(&requests).is_empty());
    }
}
