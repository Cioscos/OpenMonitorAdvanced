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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

#[cfg(not(windows))]
use oma_ipc::ServiceState;
use oma_ipc::ServiceStatus;
use tauri::State;

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

/// Saves the flag atomically: a temporary file in the same directory, then a
/// rename over the target, so a crash between the two never leaves a
/// half-written file. The saved JSON is `{"antiCheat":<enabled>}`.
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
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&body)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
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
}

impl ToggleState {
    fn new(path: Option<PathBuf>) -> Self {
        let flag = path.as_deref().map(load_anti_cheat).unwrap_or(false);
        Self {
            path,
            flag: AtomicBool::new(flag),
            tray_item: Mutex::new(None),
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

    /// Saves `enabled`, then updates the in-memory flag and the tray
    /// checkbox to match: on success both read `enabled`; on a save error
    /// both are left at the previous preference and the error is returned,
    /// never a silent success.
    fn set(&self, enabled: bool) -> Result<(), String> {
        let result = self.save(enabled);
        let shown = if result.is_ok() {
            self.flag.store(enabled, Ordering::SeqCst);
            enabled
        } else {
            self.enabled()
        };
        if let Some(item) = self
            .tray_item
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            item.set_checked(shown);
        }
        result
    }

    fn save(&self, enabled: bool) -> Result<(), String> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| "no LOCALAPPDATA path for the anti-cheat flag".to_owned())?;
        save_anti_cheat(&path, enabled).map_err(|e| e.to_string())
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

    fn status(&self) -> ServiceStatus {
        #[cfg(windows)]
        {
            self.status_table.get().1
        }
        #[cfg(not(windows))]
        {
            ServiceStatus::new(ServiceState::NotInstalled, None)
        }
    }

    /// The one toggle path: the `set_anti_cheat` command and the tray's
    /// check item both call this and nothing else.
    pub(crate) fn set_anti_cheat(&self, enabled: bool) -> Result<ServiceStatus, String> {
        self.toggle.set(enabled)?;
        #[cfg(windows)]
        self.send_link(oma_win::svc::LinkCommand::SetAntiCheat(enabled));
        Ok(self.status())
    }

    #[cfg(windows)]
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
        #[cfg(windows)]
        self.send_link(oma_win::svc::LinkCommand::Start);
        self.status()
    }

    /// Stops the link within its own join wait, or detaches it (see
    /// [`oma_win::svc::ServiceLink::shutdown`]). A no-op off Windows.
    pub fn shutdown(&self) {
        #[cfg(windows)]
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

        let err = toggle.set(true).expect_err("save must fail");
        assert!(!err.is_empty());
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

        // Simulates the `set_anti_cheat` command: the caller passes the
        // desired value directly.
        toggle.set(true).expect("command path succeeds");
        assert!(toggle.enabled());
        assert!(load_anti_cheat(&path));

        // Simulates the tray's check item: the caller computes the opposite
        // of the current preference. Both go through `ToggleState::set`.
        let requested = !toggle.enabled();
        toggle.set(requested).expect("tray path succeeds");
        assert_eq!(toggle.enabled(), requested);
        assert_eq!(load_anti_cheat(&path), requested);

        assert_eq!(
            indicator.calls(),
            vec![false, true, false],
            "the indicator only ever reflects ToggleState::set's outcome"
        );

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
