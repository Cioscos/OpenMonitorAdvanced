//! The Performance view's Tauri commands (DA20); only the main window has them.

use std::sync::Arc;
use std::time::Duration;

use oma_core::csv::LocalTime;
use oma_core::load::{RunStatus, Session, SessionSummary, StartRequest};
use oma_core::sampler::unix_ms;
use oma_core::scores::{
    cpu_baseline, disk_baseline, export_bytes, gpu_baseline, BenchStatus, HostFacts, ScoreFile,
    ScoreSummary,
};
use oma_ipc::load::Plan;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use super::bench::DiskBenchRequest;
use super::board::{BoardService, BoardTable};
use super::runner::{PerformanceRunner, SystemInfo, VolumeChoice};
use super::store::parse_rfc3339_ms;
use crate::report::{documents_dir, local_now, write_report};
use crate::settings::SettingsStore;
use crate::window::{QuitSource, MAIN};

/// How long «Stop and quit» waits for the test to end (A21).
const QUIT_TIMEOUT: Duration = Duration::from_secs(3);

type Runner<'a> = State<'a, Arc<PerformanceRunner>>;

/// Runs `f` on the blocking pool: the volume list, the folder probe and the orphan
/// sweep block on the disks (DC6, DC11).
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&PerformanceRunner) -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<Arc<PerformanceRunner>>()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn performance_system(app: AppHandle) -> Result<SystemInfo, String> {
    blocking(app, PerformanceRunner::system).await
}

/// The plan of `request`; a disk request reads its volume from metadata only.
#[tauri::command]
pub async fn performance_preview(app: AppHandle, request: StartRequest) -> Result<Plan, String> {
    blocking(app, move |r| r.preview(&request).map_err(|e| e.wire())).await?
}

/// Starts a test; the session id, or why it cannot start (`busy`, `build:<code>`,
/// `disk:<code>` for a disk target that cannot be used, DC6).
#[tauri::command]
pub async fn performance_start(app: AppHandle, request: StartRequest) -> Result<String, String> {
    blocking(app, move |r| r.start(request).map_err(|e| e.wire())).await?
}

/// The volume of a folder the user picked or confirmed (DC6); `disk:remote`,
/// `disk:not_found` or `disk:not_writable`. Writes one tiny probe file, except on a
/// spun-down HDD (`standby`): call it on a choice, never while typing.
#[tauri::command]
pub async fn performance_disk_probe(
    app: AppHandle,
    folder: String,
) -> Result<VolumeChoice, String> {
    blocking(app, move |r| r.disk_probe(&folder).map_err(|e| e.wire())).await?
}

/// «Choose folder…»: the folder picker over the main window; `None` when cancelled.
#[tauri::command]
pub async fn performance_disk_pick(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app.dialog().file();
        if let Some(window) = app.get_webview_window(MAIN) {
            dialog = dialog.set_parent(&window);
        }
        match dialog.blocking_pick_folder() {
            None => Ok(None),
            Some(path) => path
                .into_path()
                .map(|p| Some(p.to_string_lossy().into_owned()))
                .map_err(|e| e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Starts the disk benchmark; the score id, or `busy`, `disk:<code>` (DC6) or the text
/// of a system error.
#[tauri::command]
pub async fn performance_disk_bench_start(
    app: AppHandle,
    request: DiskBenchRequest,
) -> Result<String, String> {
    blocking(app, move |r| {
        r.start_disk_bench(request).map_err(|e| e.wire())
    })
    .await?
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

/// Starts the benchmark of GPU `device_id` (one of `performance_system`'s); the
/// score id, or `busy`, `build:no_gpu` (no such GPU) or the text of a system error.
#[tauri::command(async)]
pub fn performance_gpu_bench_start(
    runner: Runner<'_>,
    device_id: String,
) -> Result<String, String> {
    runner.start_gpu_bench(&device_id).map_err(|e| e.wire())
}

/// Stops the benchmark (CPU or GPU); nothing is saved.
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
#[serde(rename_all = "camelCase")]
pub struct BaselineInfo {
    /// The CPU scale is not calibrated yet: the points will change.
    provisional: bool,
    /// The same for the GPU scale.
    gpu_provisional: bool,
    /// The same for the disk scale.
    disk_provisional: bool,
}

/// The leaderboard table from disk only (DZ10).
#[tauri::command]
pub async fn performance_board(
    board: State<'_, Arc<BoardService>>,
    settings: State<'_, Arc<SettingsStore>>,
) -> Result<BoardTable, String> {
    let (board, enabled) = (
        board.inner().clone(),
        settings.snapshot().performance.community_table,
    );
    tauri::async_runtime::spawn_blocking(move || board.table(enabled))
        .await
        .map_err(|e| e.to_string())
}

/// Downloads the community table when due, or now when `manual`; nothing is requested
/// with `communityTable` off (DZ9).
#[tauri::command]
pub async fn performance_board_refresh(
    board: State<'_, Arc<BoardService>>,
    settings: State<'_, Arc<SettingsStore>>,
    manual: bool,
) -> Result<BoardTable, String> {
    let (board, enabled) = (
        board.inner().clone(),
        settings.snapshot().performance.community_table,
    );
    tauri::async_runtime::spawn_blocking(move || board.refresh(enabled, manual, unix_ms()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn performance_baseline() -> BaselineInfo {
    BaselineInfo {
        provisional: cpu_baseline().provisional,
        gpu_provisional: gpu_baseline().provisional,
        disk_provisional: disk_baseline().provisional,
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

/// What the submission and the export say about this machine (DZ5), read now: the score
/// file does not hold it. Missing data fails with `invalid`.
fn host_facts() -> Result<HostFacts, String> {
    Ok(HostFacts {
        ram_gb: oma_win::memory::installed_ram_gb().ok_or("invalid")?,
        os_build: oma_win::os_version::os_build().ok_or("invalid")?,
    })
}

/// The exact text that «Send» will post (DZ4), for the preview dialog.
#[tauri::command]
pub async fn performance_share_preview(
    runner: Runner<'_>,
    board: State<'_, Arc<BoardService>>,
    id: String,
    overclock: bool,
) -> Result<String, String> {
    let (runner, board) = (runner.inner().clone(), board.inner().clone());
    tauri::async_runtime::spawn_blocking(move || {
        let file = runner
            .store()
            .load_score(&id)
            .map_err(|_| "invalid".to_owned())?
            .ok_or_else(|| "not_found".to_owned())?;
        board.preview(&file, &host_facts()?, overclock)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Posts the submission and, on success, marks the score shared (DZ11). The error is a
/// code the UI translates (DZ12).
#[tauri::command]
pub async fn performance_share_send(
    runner: Runner<'_>,
    board: State<'_, Arc<BoardService>>,
    id: String,
    overclock: bool,
) -> Result<(), String> {
    let (runner, board) = (runner.inner().clone(), board.inner().clone());
    tauri::async_runtime::spawn_blocking(move || {
        board.share_stored(runner.store(), &id, &host_facts()?, overclock)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `oma-score-<category>-YYYYMMDD-HHMMSS.json`, from the score's time in local time.
fn score_export_file_name(category: &str, local: LocalTime) -> String {
    format!(
        "oma-score-{category}-{:04}{:02}{:02}-{:02}{:02}{:02}.json",
        local.year, local.month, local.day, local.hour, local.minute, local.second
    )
}

fn score_export(app: &AppHandle, id: &str) -> Result<Option<String>, String> {
    let runner = app.state::<Arc<PerformanceRunner>>();
    let file = runner
        .store()
        .load_score(id)
        .map_err(|e| {
            tracing::warn!(%e, "cannot read the score to export");
            "invalid".to_owned()
        })?
        .ok_or_else(|| "not_found".to_owned())?;
    let bytes = export_bytes(&file, &host_facts()?);
    let at = parse_rfc3339_ms(&file.at).map_or(unix_ms(), |ms| ms.max(0) as u64);
    let mut dialog = app
        .dialog()
        .file()
        .set_file_name(score_export_file_name(&file.category, local_now(at)))
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
    let path = picked.into_path().map_err(|e| {
        tracing::warn!(%e, "cannot use the chosen export path");
        "invalid".to_owned()
    })?;
    write_report(&path, &bytes).map_err(|err| {
        tracing::warn!(%err, "cannot export the score");
        "invalid".to_owned()
    })?;
    Ok(Some(
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    ))
}

/// Saves the score as the shareable JSON (section 8.5, no `overclock`) where the user
/// chooses; the file name, or `None` when the dialog is cancelled.
#[tauri::command]
pub async fn performance_score_export(
    app: AppHandle,
    id: String,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || score_export(&app, &id))
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

    #[test]
    fn score_export_file_name_uses_local_time() {
        let local = LocalTime {
            year: 2026,
            month: 10,
            day: 9,
            hour: 9,
            minute: 30,
            second: 0,
            millis: 0,
        };
        assert_eq!(
            score_export_file_name("cpu", local),
            "oma-score-cpu-20261009-093000.json"
        );
    }
}
