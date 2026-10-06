//! File log of `oma-load`, configured like the overlay's: daily rotation, seven
//! files, the same `RUST_LOG` override, under
//! `%LOCALAPPDATA%\OpenMonitorAdvanced\logs` with the `oma-load` prefix.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

/// The folder of the logs, shared with the app and the service.
pub fn logs_dir() -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local_app_data)
            .join("OpenMonitorAdvanced")
            .join("logs"),
    )
}

/// The guard of the file log: dropping it flushes the buffered lines.
static GUARD: Mutex<Option<tracing_appender::non_blocking::WorkerGuard>> = Mutex::new(None);

/// Sets up the file log. The process has no console (windows subsystem), so
/// a missing `LOCALAPPDATA` or an appender that cannot be built leaves it
/// without a log instead of failing. Call [`flush`] before the process exits.
pub fn init() {
    let Some(logs) = logs_dir() else { return };
    let Ok(file) = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("oma-load")
        .max_log_files(7)
        .build(logs)
    else {
        return;
    };
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(writer)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "oma_core=debug,oma_win=info,oma_load=info".into()),
        )
        .init();
    *GUARD.lock().unwrap_or_else(PoisonError::into_inner) = Some(guard);
}

/// Writes out the buffered log lines; later lines are lost. Called right before
/// `std::process::exit`, which runs no destructors.
pub fn flush() {
    drop(GUARD.lock().unwrap_or_else(PoisonError::into_inner).take());
}
