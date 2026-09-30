//! Tauri commands of the CSV log and the environment the app gives the
//! coordinator. The commands run the coordinator's blocking methods on the
//! blocking pool, never on the async executor.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager};

use super::session::{LogEnv, LogService, LogStatus, EVENT_LOG};
use crate::notifier::{launch_for_main, SystemToaster};
use crate::window::MAIN;

/// Runs `call` on the blocking pool; a panic there answers the current status.
async fn blocking(app: AppHandle, call: fn(&LogService) -> LogStatus) -> LogStatus {
    let log = app.state::<Arc<LogService>>().inner().clone();
    let worker = log.clone();
    match tauri::async_runtime::spawn_blocking(move || call(&worker)).await {
        Ok(status) => status,
        Err(err) => {
            tracing::error!(%err, "a log command failed");
            log.status()
        }
    }
}

#[tauri::command]
pub async fn log_start(app: AppHandle) -> LogStatus {
    blocking(app, LogService::start).await
}

#[tauri::command]
pub async fn log_pause(app: AppHandle) -> LogStatus {
    blocking(app, LogService::pause).await
}

#[tauri::command]
pub async fn log_resume(app: AppHandle) -> LogStatus {
    blocking(app, LogService::resume).await
}

#[tauri::command]
pub async fn log_stop(app: AppHandle) -> LogStatus {
    blocking(app, LogService::stop).await
}

#[tauri::command]
pub async fn get_log_status(app: AppHandle) -> LogStatus {
    blocking(app, LogService::status).await
}

/// Opens the folder of the session in progress, otherwise the configured
/// one (L13). A folder that does not exist yet is not created: the error is
/// `log.error.folderMissing`; other errors are the system's text.
#[tauri::command]
pub async fn open_log_folder(app: AppHandle) -> Result<(), String> {
    let log = app.state::<Arc<LogService>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || open_folder(&log))
        .await
        .map_err(|err| err.to_string())?
}

fn open_folder(log: &LogService) -> Result<(), String> {
    let dir = log.folder().map_err(|err| {
        tracing::warn!(%err, "no log folder");
        err.to_string()
    })?;
    if !dir.is_dir() {
        return Err("log.error.folderMissing".to_owned());
    }
    crate::commands::shell_open(&dir).map_err(|err| {
        tracing::warn!(%err, "cannot open the log folder");
        err.to_string()
    })
}

/// A toast of the log: a click opens the main window (L8).
pub fn toast_log(toaster: &SystemToaster, title: String, body: String) {
    toaster.show(title, body, launch_for_main());
}

/// The app side of [`LogEnv`].
pub struct TauriEnv {
    app: AppHandle,
    toaster: Arc<SystemToaster>,
    #[cfg(windows)]
    offsets: std::sync::Mutex<oma_win::local_time::OffsetCache>,
}

impl TauriEnv {
    pub fn new(app: AppHandle, toaster: Arc<SystemToaster>) -> Self {
        Self {
            app,
            toaster,
            #[cfg(windows)]
            offsets: std::sync::Mutex::new(oma_win::local_time::OffsetCache::new()),
        }
    }
}

impl LogEnv for TauriEnv {
    fn default_dir(&self) -> io::Result<PathBuf> {
        #[cfg(windows)]
        {
            oma_win::known_folder::documents_dir()
        }
        #[cfg(not(windows))]
        {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            })
    }

    fn offset_minutes(&self, unix_ms: u64) -> i32 {
        #[cfg(windows)]
        {
            self.offsets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .offset(unix_ms)
        }
        #[cfg(not(windows))]
        {
            let _ = unix_ms;
            0
        }
    }

    /// Closed means destroyed, as for the sampler's events.
    fn window_open(&self) -> bool {
        self.app.get_webview_window(MAIN).is_some()
    }

    fn toast(&self, title: String, body: String) {
        toast_log(&self.toaster, title, body);
    }

    fn emit(&self, status: &LogStatus) {
        // Nobody listens while the window is closed.
        if self.window_open() {
            let _ = self.app.emit(EVENT_LOG, status);
        }
    }
}
