//! The sensor-service status shown by the shell: the persisted anti-cheat
//! preference (`%LOCALAPPDATA%\OpenMonitorAdvanced\service.json`), the three
//! commands the UI calls, and [`ServiceShell`], which owns the link to the
//! service on Windows and shares one toggle path between the `set_anti_cheat`
//! command and the tray's check item (Task 11).
//!
//! Off Windows there is no service and no link: the commands still exist and
//! compile without importing any Win32 type, and always report
//! [`ServiceState::NotInstalled`].

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use oma_ipc::{ServiceState, ServiceStatus};
use tauri::State;

/// Stable, non-localized error code the UI matches to show its own
/// `service.action.failed` text (Task 12); the `io::Error` detail behind it
/// only ever reaches the log.
const ERR_PERSIST_FAILED: &str = "persist_failed";

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

/// Event the sampler callback emits when the service status changes.
pub const EVENT_SERVICE: &str = "oma:service";

/// `%LOCALAPPDATA%\OpenMonitorAdvanced\service.json`; `None` without
/// `LOCALAPPDATA` (never expected on a real Windows session, but the shell
/// must not panic on a stripped-down test environment).
pub fn anti_cheat_path() -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local_app_data)
            .join("OpenMonitorAdvanced")
            .join("service.json"),
    )
}

#[derive(serde::Serialize, serde::Deserialize)]
struct AntiCheatFlag {
    #[serde(rename = "antiCheat")]
    anti_cheat: bool,
}

/// A missing or unreadable (corrupted, wrong shape) file reads as off: the
/// service link starts unless the user has explicitly asked to keep it away.
pub fn load_anti_cheat(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<AntiCheatFlag>(&text).ok())
        .map(|flag| flag.anti_cheat)
        .unwrap_or(false)
}

/// Distinguishes the temp file of one `save_anti_cheat` call from another so
/// concurrent writers (an async command and the tray handler, say) never
/// share, and so never clobber, the same temp file.
static TMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Saves the flag atomically: a temporary file (unique to this process and
/// call, in the same directory) written and `fsync`ed, then a rename over
/// the target, so a crash between the two never leaves a half-written file.
/// The saved JSON is `{"antiCheat":<enabled>}`. The temp file is removed if
/// the write or the rename fails; a failed `create_dir_all` never creates one.
pub fn save_anti_cheat(path: &Path, enabled: bool) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the anti-cheat flag path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_vec(&AntiCheatFlag {
        anti_cheat: enabled,
    })
    .expect("AntiCheatFlag always serializes");
    let unique = TMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}.{unique}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&body)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The tray's anti-cheat check item, abstracted so [`ToggleState`] is
/// testable without a running Tauri app.
pub trait ToggleIndicator: Send + Sync {
    fn set_checked(&self, checked: bool);
}

#[cfg(windows)]
impl ToggleIndicator for tauri::menu::CheckMenuItem<tauri::Wry> {
    fn set_checked(&self, checked: bool) {
        let _ = tauri::menu::CheckMenuItem::set_checked(self, checked);
    }
}

/// Persistence, in-memory flag and tray checkbox of the anti-cheat
/// preference: the portable core shared by the command and the tray, with no
/// dependency on [`oma_win::svc::ServiceLink`] or a Tauri app, so it is
/// tested directly (see the `tests` module below).
struct ToggleState {
    path: Option<PathBuf>,
    flag: AtomicBool,
    tray_item: Mutex<Option<Arc<dyn ToggleIndicator>>>,
    /// Serializes the whole sequence below: the `set_anti_cheat` command runs
    /// on a tokio task, the tray's click handler runs on the event-loop
    /// thread, and both call [`Self::set`]. Held across the save, the
    /// in-memory flag, `apply` and the tray checkbox — never across an
    /// `await`, since nothing here is `async` — so the two can never
    /// interleave their steps or write the same temp file at once (each
    /// `save_anti_cheat` call also picks its own unique temp name regardless).
    write: Mutex<()>,
}

impl ToggleState {
    fn new(path: Option<PathBuf>) -> Self {
        let flag = path.as_deref().map(load_anti_cheat).unwrap_or(false);
        Self {
            path,
            flag: AtomicBool::new(flag),
            tray_item: Mutex::new(None),
            write: Mutex::new(()),
        }
    }

    fn enabled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    fn set_tray_item(&self, item: Arc<dyn ToggleIndicator>) {
        item.set_checked(self.enabled());
        *self
            .tray_item
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(item);
    }

    fn set_checked(&self, checked: bool) {
        if let Some(item) = self
            .tray_item
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            item.set_checked(checked);
        }
    }

    /// Saves `enabled`, then, in order: updates the in-memory flag, runs
    /// `apply` (the caller's link command, only on success) and updates the
    /// tray checkbox. On a save error nothing else runs, the previous
    /// preference is kept and shown, and the error is returned — never a
    /// silent success. The whole sequence runs under one lock (see `write`).
    fn set(&self, enabled: bool, apply: impl FnOnce()) -> Result<(), String> {
        let _guard = self.write.lock().unwrap_or_else(PoisonError::into_inner);
        let result = self.save(enabled);
        let shown = match result {
            Ok(()) => {
                self.flag.store(enabled, Ordering::SeqCst);
                apply();
                enabled
            }
            Err(_) => self.enabled(),
        };
        self.set_checked(shown);
        result
    }

    fn save(&self, enabled: bool) -> Result<(), String> {
        let Some(path) = self.path.clone() else {
            tracing::warn!("cannot persist the anti-cheat flag: no LOCALAPPDATA path");
            return Err(ERR_PERSIST_FAILED.to_owned());
        };
        save_anti_cheat(&path, enabled).map_err(|error| {
            tracing::warn!(%error, "cannot persist the anti-cheat flag");
            ERR_PERSIST_FAILED.to_owned()
        })
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
    /// Reads the persisted preference, starts the link (Windows) and sets
    /// the initial status on `status_table` (the same one `default_providers`
    /// hands to the `svc` provider through [`oma_win::ServiceHandles`]).
    #[cfg(windows)]
    pub fn new(
        path: Option<PathBuf>,
        status_table: oma_win::svc::ServiceStatusTable,
        feed: oma_win::svc::SvcFeed,
        interval_ms: u32,
    ) -> Self {
        use oma_win::svc::{pipe_connector, LinkSettings, ServiceLink, WindowsScm, SERVICE_NAME};

        let toggle = ToggleState::new(path);
        let anti_cheat = toggle.enabled();
        let link = ServiceLink::spawn(
            Arc::new(WindowsScm::new(SERVICE_NAME)),
            pipe_connector(),
            LinkSettings::new(oma_ipc::PIPE_NAME, interval_ms),
            anti_cheat,
            status_table.clone(),
            feed,
        );
        Self {
            toggle,
            link: Mutex::new(Some(link)),
            status_table,
        }
    }

    #[cfg(not(windows))]
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            toggle: ToggleState::new(path),
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
    /// check item both call this and nothing else. Order: save, then the
    /// link command, then the tray checkbox (via `ToggleState::set`'s `apply`).
    pub(crate) fn set_anti_cheat(&self, enabled: bool) -> Result<ServiceStatus, String> {
        self.toggle.set(enabled, || {
            self.send_link(oma_win::svc::LinkCommand::SetAntiCheat(enabled));
        })?;
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
    use super::*;

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
        fn set_checked(&self, checked: bool) {
            self.calls.lock().unwrap().push(checked);
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "oma-app-service-test-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        dir.join("service.json")
    }

    #[test]
    fn anti_cheat_flag_round_trips_and_defaults_off() {
        let path = temp_path("round-trip");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        // No file yet: off.
        assert!(!load_anti_cheat(&path));

        // A corrupted file also reads as off.
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not json").unwrap();
        assert!(!load_anti_cheat(&path));

        save_anti_cheat(&path, true).unwrap();
        assert!(load_anti_cheat(&path));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"antiCheat":true}"#
        );

        save_anti_cheat(&path, false).unwrap();
        assert!(!load_anti_cheat(&path));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"antiCheat":false}"#
        );

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn failed_save_does_not_toggle_or_claim_persistence() {
        let path = temp_path("failed-save");
        let dir = path.parent().unwrap().to_owned();
        let _ = std::fs::remove_dir_all(&dir);
        // A regular file where the flag's directory should be: create_dir_all
        // fails, so every save through this path fails predictably.
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        std::fs::write(&dir, b"blocking file").unwrap();

        let toggle = ToggleState::new(Some(path.clone()));
        assert!(!toggle.enabled());

        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator.clone());
        assert_eq!(indicator.calls(), vec![false]);

        let err = toggle
            .set(true, || panic!("apply must not run on a failed save"))
            .expect_err("save must fail");
        assert_eq!(err, ERR_PERSIST_FAILED);
        assert!(!toggle.enabled(), "the previous preference is kept");
        assert_eq!(
            indicator.calls(),
            vec![false, false],
            "the checkbox is restored to the previous preference, never left checked"
        );
        assert!(!load_anti_cheat(&path), "nothing was written");

        std::fs::remove_file(&dir).unwrap();
    }

    #[test]
    fn tray_and_command_use_the_same_toggle_path() {
        let path = temp_path("same-path");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        let toggle = ToggleState::new(Some(path.clone()));
        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator.clone());
        let applied = Arc::new(Mutex::new(Vec::new()));

        // Simulates the `set_anti_cheat` command: the caller passes the
        // desired value directly.
        let applied_clone = Arc::clone(&applied);
        toggle
            .set(true, move || applied_clone.lock().unwrap().push(true))
            .expect("command path succeeds");
        assert!(toggle.enabled());
        assert!(load_anti_cheat(&path));

        // Simulates the tray's check item: the caller computes the opposite
        // of the current preference. Both go through `ToggleState::set`.
        let requested = !toggle.enabled();
        let applied_clone = Arc::clone(&applied);
        toggle
            .set(requested, move || {
                applied_clone.lock().unwrap().push(requested)
            })
            .expect("tray path succeeds");
        assert_eq!(toggle.enabled(), requested);
        assert_eq!(load_anti_cheat(&path), requested);

        assert_eq!(
            indicator.calls(),
            vec![false, true, false],
            "the indicator only ever reflects ToggleState::set's outcome"
        );
        assert_eq!(
            *applied.lock().unwrap(),
            vec![true, false],
            "apply (the link command) ran once per successful set, with the saved value, \
             regardless of which caller asked"
        );

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn not_installed_answers_ok_without_touching_persistence() {
        // The non-Windows behaviour of every command: exercised directly
        // here since `oma-app` cannot be cross-checked for a non-Windows
        // target on this machine (see the module doc comment).
        let path = temp_path("not-installed");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        assert_eq!(
            not_installed_set_anti_cheat(true),
            Ok(not_installed_status())
        );
        assert_eq!(not_installed_status().state, ServiceState::NotInstalled);
        assert_eq!(not_installed_status().detail, None);
        assert!(
            !path.exists(),
            "the off-Windows answer never touches the filesystem"
        );
    }

    #[test]
    fn concurrent_toggles_serialize_to_a_consistent_final_state() {
        let path = temp_path("concurrent");
        let dir = path.parent().unwrap().to_owned();
        let _ = std::fs::remove_dir_all(&dir);

        let toggle = Arc::new(ToggleState::new(Some(path.clone())));
        let indicator = Arc::new(FakeIndicator::default());
        toggle.set_tray_item(indicator);

        let handles: Vec<_> = (0..8u32)
            .map(|i| {
                let toggle = Arc::clone(&toggle);
                std::thread::spawn(move || {
                    for j in 0..50u32 {
                        let enabled = (i + j) % 2 == 0;
                        // Errors are possible in principle (none expected
                        // here), but never a torn write: `set` either fully
                        // applies or fully leaves the previous state.
                        let _ = toggle.set(enabled, || {});
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let final_flag = toggle.enabled();
        assert_eq!(
            load_anti_cheat(&path),
            final_flag,
            "the file and the in-memory flag must agree after every writer is done"
        );

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no temp file survives a run of concurrent writers: {leftovers:?}"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
