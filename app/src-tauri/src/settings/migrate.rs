//! One-time migrations into the settings store (spec §2.4): the M4 anti-cheat
//! file at startup and the web view's `localStorage` state through a command.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use oma_core::settings::{ViewKind, WINDOW_VALUES};
use serde::Deserialize;

use super::{Persistence, SettingsFs, SettingsState, SettingsStore};

/// Stable, non-localized error codes the UI matches.
pub const ERR_PERSIST_FAILED: &str = "persist_failed";
pub const ERR_READ_ONLY: &str = "read_only";

/// How long a migration waits for its marker to reach the disk.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Imports the M4 `service.json` anti-cheat flag.
///
/// Runs once at startup, right after the store is opened. `file_existed` says
/// whether the current settings file was there before that: it then wins over
/// the legacy file. The marker is saved together with the imported value, and
/// the legacy file is deleted only after the save is confirmed; otherwise the
/// next start retries. A read-only or errored store is left alone, legacy
/// file included.
pub fn migrate_service_v1(
    store: &SettingsStore,
    fs: &dyn SettingsFs,
    legacy: Option<&Path>,
    file_existed: bool,
) {
    let state = store.state();
    if matches!(
        state.persistence,
        Persistence::ReadOnly { .. } | Persistence::Error { .. }
    ) {
        return;
    }
    if store.settings().migrations.service_v1 {
        // Migrated on an earlier run: only a leftover legacy file remains.
        if state.persistence == Persistence::Ok {
            remove_legacy(fs, legacy);
        }
        return;
    }
    let mut imported = None;
    if !file_existed {
        if let Some(path) = legacy {
            match fs.read(path) {
                Ok(Some(bytes)) => imported = Some(parse_legacy_anti_cheat(&bytes)),
                Ok(None) => {}
                Err(error) => {
                    // Not marked: the next start tries again.
                    tracing::warn!(%error, "cannot read the legacy anti-cheat file");
                    return;
                }
            }
        }
    }
    store.update_with(|settings| {
        if let Some(anti_cheat) = imported {
            settings.sources.anti_cheat = anti_cheat;
        }
        settings.migrations.service_v1 = true;
    });
    match store.flush_now(FLUSH_TIMEOUT) {
        Ok(()) => remove_legacy(fs, legacy),
        Err(reason) => {
            tracing::warn!(%reason, "the anti-cheat migration is not saved; retrying at the next start");
        }
    }
}

/// M4 read a missing key or a wrong shape as off.
fn parse_legacy_anti_cheat(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| value.get("antiCheat").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

fn remove_legacy(fs: &dyn SettingsFs, legacy: Option<&Path>) {
    let Some(path) = legacy else { return };
    if let Err(error) = fs.remove(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(%error, "cannot delete the legacy anti-cheat file");
        }
    }
}

/// What the web view kept in `localStorage` before M5 (camelCase JSON).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LegacyWebviewState {
    pub section: Option<String>,
    pub window: Option<u32>,
    pub series: BTreeMap<String, Vec<String>>,
    pub view: Option<String>,
}

/// Imports the web view state once; `Ok` only after the marker is on disk.
///
/// Fills only the fields still unset and the series keys still absent, so a
/// value set in the file or in this session always wins. Called again after a
/// failed save it flushes again: the marker may already be in memory.
pub fn import_webview(
    store: &SettingsStore,
    legacy: LegacyWebviewState,
) -> Result<SettingsState, String> {
    let state = store.state();
    if store.settings().migrations.webview_v1 && state.revision <= state.persisted_revision {
        return Ok(state);
    }
    if matches!(state.persistence, Persistence::ReadOnly { .. }) {
        return Err(ERR_READ_ONLY.to_owned());
    }
    store.update_with(|settings| {
        if settings.migrations.webview_v1 {
            return;
        }
        if settings.advanced.section.is_none() {
            settings.advanced.section = legacy.section;
        }
        if settings.advanced.window.is_none() {
            settings.advanced.window = legacy.window.filter(|w| WINDOW_VALUES.contains(w));
        }
        if settings.view.last.is_none() {
            settings.view.last = legacy.view.as_deref().and_then(ViewKind::parse);
        }
        for (section, ids) in legacy.series {
            settings.advanced.series.entry(section).or_insert(ids);
        }
        settings.migrations.webview_v1 = true;
    });
    match store.flush_now(FLUSH_TIMEOUT) {
        Ok(()) => Ok(store.state()),
        Err(reason) => {
            tracing::warn!(%reason, "the web view import is not saved");
            Err(ERR_PERSIST_FAILED.to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use oma_core::settings::ViewKind;
    use serde_json::json;

    use super::*;
    use crate::settings::fake_fs::{open_fast, stored_json, test_path, FakeFs};
    use crate::settings::Persistence;

    const LONG: Duration = Duration::from_secs(5);

    fn legacy_path() -> PathBuf {
        PathBuf::from("C:/oma-test/service.json")
    }

    fn migrate(store: &SettingsStore, fs: &Arc<FakeFs>, file_existed: bool) {
        migrate_service_v1(store, fs.as_ref(), Some(&legacy_path()), file_existed);
    }

    fn with_legacy(anti_cheat: bool) -> Arc<FakeFs> {
        FakeFs::new().with_file(
            &legacy_path(),
            format!(r#"{{"antiCheat":{anti_cheat}}}"#).as_bytes(),
        )
    }

    #[test]
    fn legacy_anti_cheat_is_imported_when_no_current_file() {
        let fs = with_legacy(true);
        let store = open_fast(&fs);
        migrate(&store, &fs, false);
        let settings = store.settings();
        assert!(settings.sources.anti_cheat);
        assert!(settings.migrations.service_v1);
        // Value and marker were saved together, then the legacy file went.
        let saved = stored_json(&fs);
        assert_eq!(saved["sources"]["antiCheat"], true);
        assert_eq!(saved["migrations"]["serviceV1"], true);
        assert_eq!(fs.file(&legacy_path()), None);
        assert_eq!(store.state().persistence, Persistence::Ok);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn current_file_wins_over_legacy() {
        let fs = with_legacy(true).with_file(
            &test_path(),
            br#"{"version":1,"sources":{"antiCheat":false}}"#,
        );
        let store = open_fast(&fs);
        migrate(&store, &fs, true);
        let settings = store.settings();
        assert!(!settings.sources.anti_cheat);
        assert!(settings.migrations.service_v1);
        assert_eq!(fs.file(&legacy_path()), None);
        let saved = stored_json(&fs);
        assert_eq!(saved["sources"]["antiCheat"], false);
        assert_eq!(saved["migrations"]["serviceV1"], true);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn legacy_is_kept_when_the_flush_fails() {
        let fs = with_legacy(true);
        fs.fail_next_writes(1);
        let store = open_fast(&fs);
        migrate(&store, &fs, false);
        assert!(fs.file(&legacy_path()).is_some(), "not confirmed on disk");
        // The value is applied in memory meanwhile.
        assert!(store.settings().sources.anti_cheat);
    }

    #[test]
    fn migration_retries_on_next_start() {
        let fs = with_legacy(true);
        fs.fail_next_writes(usize::MAX);
        let store = open_fast(&fs);
        migrate(&store, &fs, false);
        assert!(fs.file(&legacy_path()).is_some());
        drop(store);
        std::thread::sleep(Duration::from_millis(100));

        // Next start: writes work again, the settings file was never created.
        fs.fail_next_writes(0);
        assert_eq!(fs.file(&test_path()), None);
        let store = open_fast(&fs);
        migrate(&store, &fs, false);
        assert!(store.settings().sources.anti_cheat);
        assert_eq!(fs.file(&legacy_path()), None);
        let saved = stored_json(&fs);
        assert_eq!(saved["sources"]["antiCheat"], true);
        assert_eq!(saved["migrations"]["serviceV1"], true);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn read_only_store_never_touches_legacy() {
        let fs = with_legacy(true).with_file(&test_path(), br#"{"version":99}"#);
        let store = open_fast(&fs);
        migrate(&store, &fs, true);
        assert!(fs.file(&legacy_path()).is_some());
        assert_eq!(store.state().revision, 0);
        assert!(!store.settings().migrations.service_v1);
        assert_eq!(fs.write_attempts(), 0);
    }

    #[test]
    fn errored_store_never_touches_legacy() {
        let fs = with_legacy(true).with_file(&test_path(), b"garbage");
        fs.set_fail_preserve(true);
        let store = open_fast(&fs);
        assert!(matches!(
            store.state().persistence,
            Persistence::Error { .. }
        ));
        migrate(&store, &fs, true);
        assert!(fs.file(&legacy_path()).is_some());
        assert_eq!(store.state().revision, 0);
        assert!(!fs.ops().contains(&"remove".to_string()));
    }

    #[test]
    fn leftover_legacy_is_deleted_once_migrated() {
        let fs = with_legacy(true).with_file(
            &test_path(),
            br#"{"version":1,"migrations":{"serviceV1":true}}"#,
        );
        let store = open_fast(&fs);
        migrate(&store, &fs, true);
        assert_eq!(fs.file(&legacy_path()), None);
        // Nothing to change or save.
        assert_eq!(store.state().revision, 0);
        assert_eq!(fs.write_attempts(), 0);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn no_legacy_file_still_sets_the_marker() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        migrate(&store, &fs, false);
        assert!(!store.settings().sources.anti_cheat);
        assert!(store.settings().migrations.service_v1);
        assert_eq!(stored_json(&fs)["migrations"]["serviceV1"], true);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn corrupt_legacy_reads_as_off_like_m4() {
        let fs = FakeFs::new().with_file(&legacy_path(), b"not json");
        let store = open_fast(&fs);
        migrate(&store, &fs, false);
        assert!(!store.settings().sources.anti_cheat);
        assert!(store.settings().migrations.service_v1);
        assert_eq!(fs.file(&legacy_path()), None);
        store.shutdown(LONG).unwrap();
    }

    fn legacy_state() -> LegacyWebviewState {
        LegacyWebviewState {
            section: Some("gpu/x".into()),
            window: Some(3600),
            series: BTreeMap::from([("gpu/x".to_string(), vec!["a".to_string()])]),
            view: Some("advanced".into()),
        }
    }

    #[test]
    fn webview_import_fills_only_absent_fields() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        store
            .update(&json!({"advanced": {"window": 60, "series": {"kept": ["k"]}}}))
            .unwrap();
        let mut legacy = legacy_state();
        legacy.series.insert("kept".into(), vec!["other".into()]);
        let state = import_webview(&store, legacy).unwrap();
        let settings = store.settings();
        assert_eq!(settings.advanced.window, Some(60));
        assert_eq!(settings.advanced.section.as_deref(), Some("gpu/x"));
        assert_eq!(settings.view.last, Some(ViewKind::Advanced));
        assert_eq!(settings.advanced.series["kept"], vec!["k".to_string()]);
        assert_eq!(settings.advanced.series["gpu/x"], vec!["a".to_string()]);
        assert!(settings.migrations.webview_v1);
        assert_eq!(state.persisted_revision, state.revision);
        assert_eq!(state.persistence, Persistence::Ok);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn webview_import_ignores_invalid_values() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        let legacy = LegacyWebviewState {
            window: Some(100),
            view: Some("weird".into()),
            ..LegacyWebviewState::default()
        };
        import_webview(&store, legacy).unwrap();
        let settings = store.settings();
        assert_eq!(settings.advanced.window, None);
        assert_eq!(settings.view.last, None);
        assert!(settings.migrations.webview_v1);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn webview_import_is_idempotent() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        let first = import_webview(&store, legacy_state()).unwrap();
        let other = LegacyWebviewState {
            section: Some("cpu".into()),
            window: Some(60),
            view: Some("simple".into()),
            ..LegacyWebviewState::default()
        };
        let second = import_webview(&store, other).unwrap();
        assert_eq!(second.revision, first.revision);
        assert_eq!(second.settings, first.settings);
        assert_eq!(store.settings().advanced.section.as_deref(), Some("gpu/x"));
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn webview_import_reports_persist_failure() {
        let fs = FakeFs::new();
        fs.fail_next_writes(1);
        let store = open_fast(&fs);
        let err = import_webview(&store, legacy_state()).unwrap_err();
        assert_eq!(err, ERR_PERSIST_FAILED);
        // The UI retries: the marker is in memory but not on disk, so the
        // retry must flush again instead of answering Ok at once.
        let state = import_webview(&store, legacy_state()).unwrap();
        assert_eq!(state.persisted_revision, state.revision);
        assert_eq!(stored_json(&fs)["migrations"]["webviewV1"], true);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn webview_import_with_nothing_legacy_sets_the_marker() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        import_webview(&store, LegacyWebviewState::default()).unwrap();
        assert!(store.settings().migrations.webview_v1);
        let saved = stored_json(&fs);
        assert_eq!(saved["migrations"]["webviewV1"], true);
        // Absent stays absent: nothing invented for the untouched fields.
        assert!(saved["advanced"].get("window").is_none());
        assert!(saved["view"].get("last").is_none());
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn webview_import_is_refused_when_read_only() {
        let fs = FakeFs::new().with_file(&test_path(), br#"{"version":99}"#);
        let store = open_fast(&fs);
        let err = import_webview(&store, legacy_state()).unwrap_err();
        assert_eq!(err, ERR_READ_ONLY);
        assert_eq!(store.state().revision, 0);
        assert!(!store.settings().migrations.webview_v1);
    }

    #[test]
    fn legacy_json_is_camel_case_and_tolerates_missing_keys() {
        let parsed: LegacyWebviewState =
            serde_json::from_str(r#"{"section":"cpu","series":{"cpu":["a"]}}"#).unwrap();
        assert_eq!(parsed.section.as_deref(), Some("cpu"));
        assert_eq!(parsed.window, None);
        assert_eq!(parsed.series["cpu"], vec!["a".to_string()]);
        let empty: LegacyWebviewState = serde_json::from_str("{}").unwrap();
        assert!(empty.series.is_empty());
    }
}
