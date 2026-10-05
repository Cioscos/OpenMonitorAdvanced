//! Export of the anonymous sensor report (spec §3.3): the input is copied
//! under the engine lock, the JSON built by `oma_core::report`, and the file
//! written atomically where the user chose in the save dialog.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use oma_core::csv::LocalTime;
use oma_core::engine::Engine;
use oma_core::model::{Schema, Snapshot};
use oma_core::provider::Quality;
use oma_core::report::{build_report, ReportInput};
use oma_core::sampler::unix_ms;
use oma_core::settings::Sources;
use oma_core::stats::SensorStats;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::commands::StartupState;
use crate::service::ServiceShell;
use crate::settings::SettingsStore;
use crate::window::MAIN;
use crate::AppState;

/// `oma-report-YYYYMMDD-HHMMSS.json` in local time.
pub(crate) fn report_file_name(local: LocalTime) -> String {
    format!(
        "oma-report-{:04}{:02}{:02}-{:02}{:02}{:02}.json",
        local.year, local.month, local.day, local.hour, local.minute, local.second
    )
}

/// What the report needs from the engine, copied under one short lock.
pub(crate) struct EngineCopy {
    schema: Schema,
    latest: Option<(Snapshot, Vec<Quality>)>,
    stats: Vec<Option<SensorStats>>,
    stats_revision: u64,
}

impl EngineCopy {
    /// Schema, latest snapshot with its quality, and the statistics of every
    /// sensor, all read under the same lock, so they share one revision.
    pub(crate) fn take(engine: &Engine) -> Self {
        let schema = engine.schema().clone();
        let ids: Vec<String> = schema.sensors.iter().map(|s| s.id.clone()).collect();
        Self {
            stats: engine.stats().get(&ids),
            stats_revision: schema.revision,
            latest: engine
                .latest()
                .map(|(snapshot, quality)| (snapshot.clone(), quality.to_vec())),
            schema,
        }
    }
}

/// What the report needs from the rest of the shell.
pub(crate) struct ShellFacts {
    pub generated_at_ms: u64,
    pub app_version: String,
    pub service_version: Option<String>,
    pub os_version: Option<String>,
    pub service_state: String,
    pub anti_cheat: bool,
    pub safe_mode: bool,
    pub safe_mode_reason: Option<String>,
    pub sources: Sources,
    /// `(core device id, power state)` of every disk, the ids as in the schema.
    pub disk_states: Vec<(String, String)>,
}

pub(crate) fn build(copy: &EngineCopy, facts: &ShellFacts) -> Value {
    let disk_states: Vec<(String, &str)> = facts
        .disk_states
        .iter()
        .map(|(id, state)| (id.clone(), state.as_str()))
        .collect();
    build_report(&ReportInput {
        generated_at_ms: facts.generated_at_ms,
        app_version: &facts.app_version,
        service_version: facts.service_version.as_deref(),
        protocol_version: oma_ipc::PROTOCOL_VERSION,
        os_version: facts.os_version.as_deref(),
        service_state: &facts.service_state,
        anti_cheat: facts.anti_cheat,
        safe_mode: facts.safe_mode,
        safe_mode_reason: facts.safe_mode_reason.as_deref(),
        sources: &facts.sources,
        disk_states: &disk_states,
        schema: &copy.schema,
        snapshot: copy
            .latest
            .as_ref()
            .map(|(snapshot, quality)| (snapshot, quality.as_slice())),
        stats: &copy.stats,
        stats_revision: copy.stats_revision,
    })
}

/// The camelCase or snake_case name of a serialized unit variant
/// (`"standby"`, `"connected"`, `"crash"`).
fn variant_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Writes `bytes` to a temporary file next to `path`, then replaces `path`
/// with it, so a failure never leaves a half-written report behind.
pub(crate) fn write_report(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let written = std::fs::File::create(&tmp)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| replace(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

#[cfg(windows)]
fn replace(tmp: &Path, target: &Path) -> std::io::Result<()> {
    oma_win::fsutil::replace_file(tmp, target)
}

#[cfg(not(windows))]
fn replace(tmp: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::rename(tmp, target)
}

/// The folder of the last report exported in this session.
#[derive(Default)]
pub(crate) struct ReportState(Mutex<Option<PathBuf>>);

impl ReportState {
    fn set(&self, folder: PathBuf) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(folder);
    }

    fn folder(&self) -> Option<PathBuf> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The reply of `export_sensor_report`: only the name, never the full path.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedReport {
    pub file_name: String,
}

#[cfg(windows)]
fn os_version() -> Option<String> {
    oma_win::os_version::os_version()
}

#[cfg(not(windows))]
fn os_version() -> Option<String> {
    None
}

#[cfg(windows)]
pub(crate) fn local_now(now_ms: u64) -> LocalTime {
    let offset = oma_win::local_time::utc_offset_minutes(now_ms).unwrap_or(0);
    oma_core::csv::local_time(now_ms, offset)
}

#[cfg(not(windows))]
pub(crate) fn local_now(now_ms: u64) -> LocalTime {
    oma_core::csv::local_time(now_ms, 0)
}

#[cfg(windows)]
fn documents_dir() -> Option<PathBuf> {
    oma_win::known_folder::documents_dir().ok()
}

#[cfg(not(windows))]
fn documents_dir() -> Option<PathBuf> {
    None
}

fn shell_facts(app: &AppHandle, now_ms: u64) -> ShellFacts {
    let service = app.state::<ServiceShell>();
    let startup = app.state::<StartupState>().current();
    // The table already holds core device ids, the ones the schema uses.
    let disk_states = app
        .state::<AppState>()
        .disk_states
        .get()
        .1
        .iter()
        .map(|(id, power)| (id.clone(), variant_name(power)))
        .collect();
    ShellFacts {
        generated_at_ms: now_ms,
        app_version: app.package_info().version.to_string(),
        service_version: service.service_version(),
        os_version: os_version(),
        service_state: variant_name(&service.status().state),
        anti_cheat: service.anti_cheat_enabled(),
        safe_mode: startup.safe_mode,
        safe_mode_reason: startup.reason.as_ref().map(variant_name),
        sources: app.state::<Arc<SettingsStore>>().settings().sources.clone(),
        disk_states,
    }
}

/// Builds the report, asks where to save it and writes it. Runs on a
/// blocking thread: the dialog waits for the user.
fn export(app: &AppHandle) -> Result<Option<ExportedReport>, String> {
    let now_ms = unix_ms();
    // The guard is a temporary of this statement: the engine is unlocked
    // again before anything else, the dialog included.
    let copy = EngineCopy::take(
        &app.state::<AppState>()
            .engine
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    let report = build(&copy, &shell_facts(app, now_ms));
    let bytes = serde_json::to_vec_pretty(&report).map_err(|err| err.to_string())?;

    let mut dialog = app
        .dialog()
        .file()
        .set_file_name(report_file_name(local_now(now_ms)))
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
    let path = picked.into_path().map_err(|err| err.to_string())?;
    write_report(&path, &bytes).map_err(|err| {
        tracing::warn!(%err, "cannot write the sensor report");
        err.to_string()
    })?;
    if let Some(folder) = path.parent() {
        app.state::<ReportState>().set(folder.to_path_buf());
    }
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Some(ExportedReport { file_name }))
}

/// Saves the anonymous sensor report where the user chooses; `None` when the
/// dialog is cancelled. The error is the system's text. The work runs on the
/// blocking pool: the save dialog must not hold an async worker.
#[tauri::command]
pub async fn export_sensor_report(app: AppHandle) -> Result<Option<ExportedReport>, String> {
    tauri::async_runtime::spawn_blocking(move || export(&app))
        .await
        .map_err(|err| err.to_string())?
}

/// Opens the folder of the last exported report; the UI never passes a path.
#[tauri::command(async)]
pub fn reveal_sensor_report(state: tauri::State<'_, ReportState>) -> Result<(), String> {
    let folder = state
        .folder()
        .ok_or_else(|| "no report exported yet".to_owned())?;
    crate::commands::open_path(&folder)
        .inspect_err(|err| tracing::warn!(%err, "cannot open the report folder"))
}
#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
    use oma_core::provider::{Inventory, Provider, ProviderError};

    use super::*;

    #[test]
    fn report_file_name_uses_local_time() {
        let local = LocalTime {
            year: 2026,
            month: 10,
            day: 4,
            hour: 9,
            minute: 5,
            second: 7,
            millis: 999,
        };
        assert_eq!(report_file_name(local), "oma-report-20261004-090507.json");
    }

    struct Scripted(VecDeque<f64>);

    impl Provider for Scripted {
        fn name(&self) -> &'static str {
            "scripted"
        }
        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            Ok(Inventory {
                devices: vec![Device {
                    id: "cpu/0".into(),
                    kind: DeviceKind::Cpu,
                    name: "CPU".into(),
                    vendor: None,
                    properties: Default::default(),
                }],
                sensors: vec![Sensor::new(
                    "cpu/0",
                    SensorKind::Load,
                    "total",
                    Unit::Percent,
                    Label::new("cpu.load.total"),
                    Source::Mock,
                )],
            })
        }
        fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
            Ok(vec![self.0.pop_front()])
        }
    }

    fn facts() -> ShellFacts {
        ShellFacts {
            generated_at_ms: 0,
            app_version: "0.4.0".into(),
            service_version: None,
            os_version: Some("10.0.26300".into()),
            service_state: "notInstalled".into(),
            anti_cheat: false,
            safe_mode: false,
            safe_mode_reason: None,
            sources: Sources::default(),
            disk_states: vec![("storage/device-abc".into(), "standby".into())],
        }
    }

    #[test]
    fn gather_input_without_snapshot_still_builds() {
        // No tick at all: an empty schema, still a report.
        let engine = Engine::new(vec![Box::new(Scripted(VecDeque::new()))], 60);
        let report = build(&EngineCopy::take(&engine), &facts());
        assert_eq!(report["format"], 1);
        assert_eq!(report["os"]["version"], "10.0.26300");
        assert_eq!(report["sensors"], serde_json::json!([]));
        assert_eq!(report["state"]["disks"][0]["deviceId"], "storage/disk-1");
        assert_eq!(report["state"]["disks"][0]["state"], "standby");

        // Sensors known, snapshot missing: the value is null, stats stay.
        let mut engine = Engine::new(vec![Box::new(Scripted(VecDeque::from([42.0])))], 60);
        engine.tick(1_000, 1_000);
        let mut copy = EngineCopy::take(&engine);
        assert!(copy.latest.is_some());
        copy.latest = None;
        let report = build(&copy, &facts());
        let sensor = &report["sensors"][0];
        assert_eq!(sensor["id"], "cpu/0/load/total");
        assert_eq!(sensor["value"], Value::Null);
        assert_eq!(sensor["quality"], "fresh");
        assert_eq!(sensor["stats"]["max"], 42.0);
    }

    #[test]
    fn write_report_replaces_the_file_and_leaves_no_temporary() {
        let dir = std::env::temp_dir().join(format!("oma-report-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("oma-report.json");
        write_report(&path, b"first").unwrap();
        write_report(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("oma-report.json")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn variant_names_are_the_serialized_names() {
        use crate::commands::SafeModeReason;
        assert_eq!(variant_name(&SafeModeReason::Crash), "crash");
        assert_eq!(
            variant_name(&oma_ipc::ServiceState::NotInstalled),
            "notInstalled"
        );
        #[cfg(windows)]
        assert_eq!(
            variant_name(&crate::commands::DiskPower::Standby),
            "standby"
        );
        assert_eq!(variant_name(&1.5), "unknown");
    }
}
