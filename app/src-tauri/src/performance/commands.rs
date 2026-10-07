//! The Performance view's Tauri commands (DA20); only the main window has them.

use std::sync::Arc;
use std::time::Duration;

use oma_core::csv::LocalTime;
use oma_core::load::{RunStatus, Session, SessionSummary, StartRequest};
use oma_core::sampler::unix_ms;
use oma_core::scores::{cpu_baseline, BenchStatus, ScoreFile, ScoreSummary};
use oma_ipc::load::Plan;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use super::runner::{PerformanceRunner, SystemInfo};
use super::store::parse_rfc3339_ms;
use crate::report::{documents_dir, local_now, write_report};
use crate::window::{QuitSource, MAIN};

/// How long «Stop and quit» waits for the test to end (A21).
const QUIT_TIMEOUT: Duration = Duration::from_secs(3);

type Runner<'a> = State<'a, Arc<PerformanceRunner>>;

#[tauri::command(async)]
pub fn performance_system(runner: Runner<'_>) -> SystemInfo {
    runner.system()
}

#[tauri::command(async)]
pub fn performance_preview(runner: Runner<'_>, request: StartRequest) -> Result<Plan, String> {
    runner.preview(&request).map_err(|e| e.wire())
}

/// Starts a test; the session id, or why it cannot start.
#[tauri::command(async)]
pub fn performance_start(runner: Runner<'_>, request: StartRequest) -> Result<String, String> {
    runner.start(request).map_err(|e| e.wire())
}

#[tauri::command]
pub fn performance_stop(runner: Runner<'_>) {
    runner.stop();
}

#[tauri::command]
pub fn performance_status(runner: Runner<'_>) -> RunStatus {
    runner.status()
}

#[tauri::command(async)]
pub fn performance_history(runner: Runner<'_>) -> Vec<SessionSummary> {
    runner.store().list()
}

#[tauri::command(async)]
pub fn performance_session(runner: Runner<'_>, id: String) -> Result<Option<Session>, String> {
    runner.store().load(&id).map_err(|e| e.to_string())
}

/// Deletes a saved session; not the one still running.
#[tauri::command(async)]
pub fn performance_delete(runner: Runner<'_>, id: String) -> Result<(), String> {
    if runner.stress_running() && runner.status().session_id == id {
        return Err("the session is still running".into());
    }
    runner.store().delete(&id).map_err(|e| e.to_string())
}

/// Starts the CPU benchmark; the score id, or `busy` (a test or a benchmark
/// runs) or the text of a system error.
#[tauri::command(async)]
pub fn performance_bench_start(runner: Runner<'_>) -> Result<String, String> {
    runner.start_bench().map_err(|e| e.wire())
}

/// Stops the benchmark; nothing is saved.
#[tauri::command]
pub fn performance_bench_stop(runner: Runner<'_>) {
    runner.stop_bench();
}

/// The benchmark in progress or the last one; `null` before any.
#[tauri::command]
pub fn performance_bench_status(runner: Runner<'_>) -> Option<BenchStatus> {
    runner.bench_status()
}

/// The saved scores, newest first.
#[tauri::command(async)]
pub fn performance_scores(runner: Runner<'_>) -> Vec<ScoreSummary> {
    runner.store().list_scores()
}

#[tauri::command(async)]
pub fn performance_score(runner: Runner<'_>, id: String) -> Result<Option<ScoreFile>, String> {
    runner.store().load_score(&id).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn performance_score_delete(runner: Runner<'_>, id: String) -> Result<(), String> {
    runner.store().delete_score(&id).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct BaselineInfo {
    /// The scale is not calibrated yet: the points will change.
    provisional: bool,
}

#[tauri::command]
pub fn performance_baseline() -> BaselineInfo {
    BaselineInfo {
        provisional: cpu_baseline().provisional,
    }
}

/// `oma-stress-YYYYMMDD-HHMMSS.json`, from the session's start in local time.
fn export_file_name(local: LocalTime) -> String {
    format!(
        "oma-stress-{:04}{:02}{:02}-{:02}{:02}{:02}.json",
        local.year, local.month, local.day, local.hour, local.minute, local.second
    )
}

fn export(app: &AppHandle, id: &str) -> Result<Option<String>, String> {
    let runner = app.state::<Arc<PerformanceRunner>>();
    let session = runner
        .store()
        .load(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no such session".to_owned())?;
    let bytes = serde_json::to_vec_pretty(&session).map_err(|e| e.to_string())?;
    let started = parse_rfc3339_ms(&session.started_at).map_or(unix_ms(), |ms| ms.max(0) as u64);
    let mut dialog = app
        .dialog()
        .file()
        .set_file_name(export_file_name(local_now(started)))
        .add_filter("JSON", &["json"]);
    if let Some(documents) = documents_dir() {
        dialog = dialog.set_directory(documents);
    }
    if let Some(window) = app.get_webview_window(MAIN) {
        dialog = dialog.set_parent(&window);
    }
    let Some(picked) = dialog.blocking_save_file() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    write_report(&path, &bytes).map_err(|err| {
        tracing::warn!(%err, "cannot export the stress session");
        err.to_string()
    })?;
    Ok(Some(
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    ))
}

/// Saves the whole session as JSON where the user chooses; the file name, or
/// `None` when the dialog is cancelled. The dialog runs on the blocking pool.
#[tauri::command]
pub async fn performance_export(app: AppHandle, id: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || export(&app, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// «Stop and quit»: the test ends (`stopped_user`), then the tray's «Quit» goes on.
#[tauri::command]
pub async fn performance_quit_confirmed(app: AppHandle) {
    let _ = tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<PerformanceRunner>>().shutdown(QUIT_TIMEOUT);
        crate::window::quit(&app, QuitSource::Tray);
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_file_name_uses_local_time() {
        let local = LocalTime {
            year: 2026,
            month: 10,
            day: 6,
            hour: 9,
            minute: 5,
            second: 7,
            millis: 999,
        };
        assert_eq!(export_file_name(local), "oma-stress-20261006-090507.json");
    }
}
