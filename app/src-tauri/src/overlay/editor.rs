//! The overlay editor's Tauri commands: thin wrappers over [`ProfileStore`]
//! that reload the overlay's catalog after every write. The file dialogs run
//! on the blocking pool (`async`), never on the main thread, which has to
//! keep pumping their messages.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, PoisonError};

use oma_core::model::Schema;
use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use super::profiles::app_profiles_dir;
use super::store::{EditableProfile, ProfileStore, StoreError};
use crate::i18n::{t, Lang};
use crate::settings::SettingsStore;
use crate::tray::language_for;
use crate::AppState;

/// Extension of an exported profile (without the leading dot).
const EXPORT_EXTENSION: &str = "omaoverlay.json";

/// An error for the UI: an i18n key and the `{detail}` of its message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub key: String,
    pub detail: Option<String>,
}

impl From<StoreError> for CommandError {
    fn from(e: StoreError) -> Self {
        Self {
            key: e.key().to_owned(),
            detail: e.detail(),
        }
    }
}

type Answer<T> = Result<T, CommandError>;

fn store() -> Result<ProfileStore, StoreError> {
    app_profiles_dir()
        .map(ProfileStore::new)
        .ok_or_else(|| StoreError::Io("APPDATA is not set".to_owned()))
}

fn schema(app: &AppHandle) -> Schema {
    app.state::<AppState>()
        .engine
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .schema()
        .clone()
}

fn lang(app: &AppHandle) -> Lang {
    language_for(
        app.state::<Arc<SettingsStore>>()
            .snapshot()
            .general
            .language,
    )
}

/// The overlay reads the profile folder again after a write.
fn reload(app: &AppHandle) {
    #[cfg(windows)]
    if let Some(overlay) = app.try_state::<super::runner::OverlayHandle>() {
        overlay.reload_profiles();
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[tauri::command(async)]
pub fn overlay_load_profile(app: AppHandle, id: String) -> Answer<EditableProfile> {
    Ok(store()?.load(&id, &schema(&app), lang(&app))?)
}

#[tauri::command(async)]
pub fn overlay_save_profile(app: AppHandle, id: Option<String>, json: String) -> Answer<String> {
    let id = store()?.save(id.as_deref(), &json)?;
    reload(&app);
    Ok(id)
}

#[tauri::command(async)]
pub fn overlay_delete_profile(app: AppHandle, id: String) -> Answer<()> {
    store()?.delete(&id)?;
    reload(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn overlay_duplicate_profile(app: AppHandle, id: String) -> Answer<String> {
    let id = store()?.duplicate(&id, &schema(&app), lang(&app))?;
    reload(&app);
    Ok(id)
}

/// «Open» dialog over the calling window; `None` when the user cancels.
#[tauri::command(async)]
pub fn overlay_import_profile(app: AppHandle, window: WebviewWindow) -> Answer<Option<String>> {
    let lang = lang(&app);
    let picked = app
        .dialog()
        .file()
        .set_parent(&window)
        .add_filter(t(lang, "editor.dialog.filter", &[]), &[EXPORT_EXTENSION])
        .add_filter(t(lang, "editor.dialog.allFiles", &[]), &["*"])
        .blocking_pick_file();
    let Some(path) = picked else { return Ok(None) };
    let id = store()?.import(&into_path(path)?)?;
    reload(&app);
    Ok(Some(id))
}

/// «Save» dialog over the calling window, named `<name>.omaoverlay.json`;
/// `false` when the user cancels.
#[tauri::command(async)]
pub fn overlay_export_profile(app: AppHandle, window: WebviewWindow, id: String) -> Answer<bool> {
    let (schema, lang) = (schema(&app), lang(&app));
    let store = store()?;
    let name = store.profile(&id, &schema, lang)?.name;
    let picked = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_file_name(export_file_name(&name))
        .add_filter(t(lang, "editor.dialog.filter", &[]), &[EXPORT_EXTENSION])
        .blocking_save_file();
    let Some(path) = picked else { return Ok(false) };
    store.export(&id, &schema, lang, &into_path(path)?)?;
    Ok(true)
}

fn into_path(path: tauri_plugin_dialog::FilePath) -> Result<PathBuf, StoreError> {
    path.into_path().map_err(|e| StoreError::Io(e.to_string()))
}

/// `<name>.omaoverlay.json`, with the characters Windows forbids in a file
/// name replaced by `_`.
fn export_file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    format!("{}.{EXPORT_EXTENSION}", safe.trim())
}

/// The installed font families, read once per session on the blocking pool;
/// `["Segoe UI"]` when they cannot be read (not cached, so a later call retries).
#[tauri::command]
pub async fn overlay_font_families() -> Vec<String> {
    static FONTS: OnceLock<Vec<String>> = OnceLock::new();
    if let Some(fonts) = FONTS.get() {
        return fonts.clone();
    }
    match tauri::async_runtime::spawn_blocking(read_font_families).await {
        Ok(Ok(fonts)) => FONTS.get_or_init(|| fonts).clone(),
        Ok(Err(err)) => {
            tracing::warn!(%err, "cannot list the system fonts");
            vec!["Segoe UI".to_owned()]
        }
        Err(err) => {
            tracing::warn!(%err, "font listing panicked");
            vec!["Segoe UI".to_owned()]
        }
    }
}

#[cfg(windows)]
fn read_font_families() -> Result<Vec<String>, String> {
    oma_win::fonts::system_font_families().map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn read_font_families() -> Result<Vec<String>, String> {
    Err("no DirectWrite off Windows".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_file_name_replaces_forbidden_characters() {
        assert_eq!(export_file_name("Gaming"), "Gaming.omaoverlay.json");
        assert_eq!(export_file_name(r"a/b\c:d*?"), "a_b_c_d__.omaoverlay.json");
    }

    #[test]
    fn store_errors_map_to_their_keys() {
        let e = CommandError::from(StoreError::Invalid("bad".into()));
        assert_eq!(e.key, "editor.error.invalid");
        assert_eq!(e.detail.as_deref(), Some("bad"));
        assert_eq!(
            CommandError::from(StoreError::ReadOnly),
            CommandError {
                key: "editor.error.readOnly".into(),
                detail: None
            }
        );
    }
}
