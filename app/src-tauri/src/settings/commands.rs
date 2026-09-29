//! Tauri commands of the settings store.

use std::sync::Arc;

use oma_core::settings::PatchError;
use serde::Serialize;
use serde_json::Value;
use tauri::State;

use super::migrate::{import_webview, LegacyWebviewState};
use super::{SettingsState, SettingsStore};

/// A rejected patch: the failing field and an i18n key.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct PatchErrorDto {
    pub field: String,
    pub key: String,
}

impl From<PatchError> for PatchErrorDto {
    fn from(error: PatchError) -> Self {
        Self {
            field: error.field,
            key: error.key.to_string(),
        }
    }
}

#[tauri::command]
pub fn get_settings(store: State<'_, Arc<SettingsStore>>) -> SettingsState {
    store.state()
}

#[tauri::command]
pub fn update_settings(
    store: State<'_, Arc<SettingsStore>>,
    patch: Value,
) -> Result<SettingsState, PatchErrorDto> {
    store.update(&patch).map_err(PatchErrorDto::from)
}

/// Imports the web view's `localStorage` state once (spec §2.4). `Ok` means
/// the values and the marker are on disk, so the UI may delete its keys; the
/// errors are `persist_failed` (retry at the next start) and `read_only`.
#[tauri::command]
pub fn import_webview_state(
    store: State<'_, Arc<SettingsStore>>,
    legacy: LegacyWebviewState,
) -> Result<SettingsState, String> {
    import_webview(&store, legacy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_error_dto_carries_field_and_key() {
        let dto = PatchErrorDto::from(PatchError {
            field: "general.intervalMs".into(),
            key: "settings.error.range",
        });
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            serde_json::json!({"field": "general.intervalMs", "key": "settings.error.range"})
        );
    }
}
