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

use oma_ipc::{ServiceState, ServiceStatus};
use tauri::State;

use crate::settings::SettingsStore;

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
// `#[cfg(not(windows))]` impl never calls `ToggleState::set` or
// `refresh_indicator`, so a click's native, self-toggling checkbox state is
// never corrected back to the (always-`false`, off Windows) in-memory flag:
// the checkbox can drift from it permanently. Harmless, since off Windows
// the flag drives no real behaviour, but worth knowing before wiring this
// checkbox to anything else off Windows.
impl ToggleIndicator for tauri::menu::CheckMenuItem<tauri::Wry> {
    /// `CheckMenuItem::set_checked` blocks the calling thread until the main
    /// thread runs it (`tauri::menu::run_item_main_thread!`), and if that
    /// call happened while this process held a lock the main thread's own
    /// event handler also needs, the two threads deadlock (reproduced by
    /// `tests::a_stuck_indicator_update_never_blocks_a_concurrent_toggle`).
    /// `run_on_main_thread` avoids that: it posts the closure and returns at
    /// once — synchronously, if already on the main thread, since
    /// `tauri-runtime-wry` runs same-thread posts in place — so this method
    /// never blocks its caller waiting for the main thread.
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

/// The anti-cheat preference: it lives in the settings store, and this keeps
/// the tray checkbox aligned with it. A store listener moves the checkbox on
/// every change of `sources.antiCheat`, whichever way it was made (the tray,
/// the `set_anti_cheat` command or the settings view). It is portable, with no
/// dependency on [`oma_win::svc::ServiceLink`] or a Tauri app, so it is tested
/// directly (see the `tests` module below).
struct ToggleState {
    store: Arc<SettingsStore>,
    /// Last value the listener saw. `Arc` so the reader handed to the
    /// indicator is `'static` and never needs `self`: it runs later, on the
    /// main thread, after every lock below has been released.
    flag: Arc<AtomicBool>,
    tray_item: TrayItem,
    /// Serializes the store change and the link command of one toggle, so
    /// concurrent toggles (the command runs on a tokio task, the tray click on
    /// the event-loop thread) reach the store and the link in the same order.
    /// Never held while waiting for the save, or across an `await`. Locked by
    /// `set` (which off Windows only a test calls) and, on Windows, by
    /// `ServiceShell::spawn_link`.
    #[cfg_attr(not(windows), allow(dead_code))]
    write: Mutex<()>,
}

impl ToggleState {
    fn new(store: Arc<SettingsStore>) -> Self {
        let flag = Arc::new(AtomicBool::new(store.settings().sources.anti_cheat));
        let tray_item: TrayItem = Arc::default();
        {
            let flag = Arc::clone(&flag);
            let tray_item = Arc::clone(&tray_item);
            // Runs on whichever thread changes the store, the settings writer
            // included: it only posts the checkbox update (see the
            // `CheckMenuItem` impl above), which never blocks.
            store.subscribe(Box::new(move |settings, _| {
                let now = settings.sources.anti_cheat;
                if flag.swap(now, Ordering::SeqCst) != now {
                    refresh_indicator(&tray_item, &flag);
                }
            }));
        }
        Self {
            store,
            flag,
            tray_item,
            write: Mutex::new(()),
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

    /// Stores `enabled` and runs `apply` (the caller's link command), in that
    /// order and under `write`; the checkbox follows through the store
    /// listener. Then waits for the save with `write` released. A failed save
    /// does not undo the mode: it stays applied and the store's persistence
    /// state shows the error.
    /// Off Windows only a test calls this (`ServiceShell::set_anti_cheat`'s
    /// `#[cfg(not(windows))]` impl never persists, so it never reaches here).
    #[cfg_attr(not(windows), allow(dead_code))]
    fn set(&self, enabled: bool, apply: impl FnOnce()) {
        {
            let _guard = self.write.lock().unwrap_or_else(PoisonError::into_inner);
            self.store
                .update_with(|settings| settings.sources.anti_cheat = enabled);
            apply();
        }
        if let Err(reason) = self.store.flush_now(FLUSH_TIMEOUT) {
            tracing::warn!(%reason, "the anti-cheat preference is applied but not saved");
        }
    }
}

/// Shared state the three commands and the tray read: the toggle above, and
/// on Windows the running link and the status table it writes.
pub struct ServiceShell {
    toggle: ToggleState,
    #[cfg(windows)]
    link: Mutex<Option<oma_win::svc::ServiceLink>>,
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
        Self {
            toggle: ToggleState::new(store),
            link: Mutex::new(None),
            status_table,
        }
    }

    /// Starts the link to the service (once; later calls do nothing). Called
    /// from Tauri's `.setup()` hook, which only the surviving instance
    /// reaches: `tauri_plugin_single_instance` ends a second launch while the
    /// app is being built, so that process never probes or starts the
    /// service, nor connects to its pipe (final review M2). Holds the
    /// toggle's `write` lock, so a concurrent toggle is either read here or
    /// sent to the new link, never lost in between (same lock order as
    /// `ToggleState::set`: `write`, then `link`).
    #[cfg(windows)]
    pub fn spawn_link(&self, feed: oma_win::svc::SvcFeed, interval_ms: u32) {
        use oma_win::svc::{pipe_connector, LinkSettings, ServiceLink, WindowsScm, SERVICE_NAME};

        let _write = self
            .toggle
            .write
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut link = self.link.lock().unwrap_or_else(PoisonError::into_inner);
        if link.is_some() {
            return;
        }
        *link = Some(ServiceLink::spawn(
            Arc::new(WindowsScm::new(SERVICE_NAME)),
            pipe_connector(),
            LinkSettings::new(oma_ipc::PIPE_NAME, interval_ms),
            self.toggle.enabled(),
            self.status_table.clone(),
            feed,
        ));
    }

    #[cfg(not(windows))]
    pub fn new(store: Arc<SettingsStore>) -> Self {
        Self {
            toggle: ToggleState::new(store),
        }
    }

    pub(crate) fn set_tray_item(&self, item: Arc<dyn ToggleIndicator>) {
        self.toggle.set_tray_item(item);
    }

    pub(crate) fn anti_cheat_enabled(&self) -> bool {
        self.toggle.enabled()
    }
}

#[cfg(windows)]
impl ServiceShell {
    fn status(&self) -> ServiceStatus {
        self.status_table.get().1
    }

    /// The one toggle path: the `set_anti_cheat` command and the tray's
    /// check item both call this and nothing else. Order: the store change
    /// (which moves the tray checkbox through its listener), the link command
    /// (`apply`, run by `ToggleState::set` while `write` is still held), then
    /// the wait for the save. A failed save does not undo the mode, so this
    /// answers `Ok` either way; the store's persistence state carries the
    /// error.
    pub(crate) fn set_anti_cheat(&self, enabled: bool) -> Result<ServiceStatus, String> {
        self.toggle.set(enabled, || {
            self.send_link(oma_win::svc::LinkCommand::SetAntiCheat(enabled));
        });
        Ok(self.status())
    }

    fn send_link(&self, command: oma_win::svc::LinkCommand) {
        if let Some(link) = self
            .link
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            link.send(command);
        }
    }

    pub(crate) fn start(&self) -> ServiceStatus {
        self.send_link(oma_win::svc::LinkCommand::Start);
        self.status()
    }

    /// Stops the link within its own join wait, or detaches it (see
    /// [`oma_win::svc::ServiceLink::shutdown`]).
    pub fn shutdown(&self) {
        if let Some(link) = self
            .link
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            link.shutdown();
        }
    }
}

#[cfg(not(windows))]
impl ServiceShell {
    fn status(&self) -> ServiceStatus {
        not_installed_status()
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

    fn new_store(fs: &Arc<FakeFs>) -> Arc<SettingsStore> {
        Arc::new(open_fast(fs))
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
        let toggle = ToggleState::new(store.clone());
        assert!(!toggle.enabled());
        set_anti_cheat_in_store(&store, true);
        assert!(toggle.enabled());

        let fs = FakeFs::new().with_file(
            &test_path(),
            br#"{"version":1,"sources":{"antiCheat":true}}"#,
        );
        assert!(ToggleState::new(new_store(&fs)).enabled());
    }

    #[test]
    fn anti_cheat_applies_even_when_the_flush_fails() {
        let fs = FakeFs::new();
        fs.fail_next_writes(usize::MAX);
        let store = new_store(&fs);
        let toggle = ToggleState::new(store.clone());
        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator.clone());
        assert_eq!(indicator.calls(), vec![false]);

        let applied = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&applied);
        toggle.set(true, move || sink.lock().unwrap().push(true));

        // The mode is on, in the link, in the store and on the checkbox.
        assert_eq!(*applied.lock().unwrap(), vec![true]);
        assert!(toggle.enabled());
        assert!(store.settings().sources.anti_cheat);
        assert_eq!(indicator.calls(), vec![false, true]);
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
        let toggle = ToggleState::new(store.clone());
        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator.clone());

        // A change made from the settings view moves the tray checkbox.
        set_anti_cheat_in_store(&store, true);
        assert_eq!(indicator.calls(), vec![false, true]);
        assert!(toggle.enabled());
        // An unrelated change leaves it alone.
        store
            .update(&serde_json::json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        set_anti_cheat_in_store(&store, false);
        assert_eq!(indicator.calls(), vec![false, true, false]);
    }

    #[test]
    fn tray_and_command_use_the_same_toggle_path() {
        let fs = FakeFs::new();
        let store = new_store(&fs);
        let toggle = ToggleState::new(store.clone());
        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator.clone());
        let applied = Arc::new(Mutex::new(Vec::new()));

        // Simulates the `set_anti_cheat` command: the caller passes the
        // desired value directly.
        let applied_clone = Arc::clone(&applied);
        toggle.set(true, move || applied_clone.lock().unwrap().push(true));
        assert!(toggle.enabled());
        assert_eq!(stored_json(&fs)["sources"]["antiCheat"], true);

        // Simulates the tray's check item: the caller computes the opposite
        // of the current preference. Both go through `ToggleState::set`.
        let requested = !toggle.enabled();
        let applied_clone = Arc::clone(&applied);
        toggle.set(requested, move || {
            applied_clone.lock().unwrap().push(requested)
        });
        assert_eq!(toggle.enabled(), requested);
        assert_eq!(stored_json(&fs)["sources"]["antiCheat"], requested);

        assert_eq!(
            indicator.calls(),
            vec![false, true, false],
            "the indicator only ever reflects the store"
        );
        assert_eq!(
            *applied.lock().unwrap(),
            vec![true, false],
            "apply (the link command) ran once per set, with the saved value,              regardless of which caller asked"
        );
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
        let toggle = Arc::new(ToggleState::new(store.clone()));
        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator.clone());
        let applied = Arc::new(Mutex::new(Vec::new()));

        let handles: Vec<_> = (0..8u32)
            .map(|i| {
                let toggle = Arc::clone(&toggle);
                let applied = Arc::clone(&applied);
                std::thread::spawn(move || {
                    for j in 0..50u32 {
                        let enabled = (i + j) % 2 == 0;
                        let applied = Arc::clone(&applied);
                        toggle.set(enabled, move || applied.lock().unwrap().push(enabled));
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
        // The last link command is the last stored value: `write` orders them.
        assert_eq!(applied.lock().unwrap().last().copied(), Some(final_flag));
        // The checkbox ends on the stored value, too.
        assert_eq!(indicator.calls().last().copied(), Some(final_flag));
    }
}
