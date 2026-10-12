//! Tauri commands of the «Themes» page (spec M10 §4.2, §5.3). The file I/O
//! and the modal dialogs run on the blocking pool, like the editor's.
//! The active theme changes through the settings patch `appearance.theme`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use oma_core::theme::{
    builtin, builtins, contrast_warnings, parse_theme, resolve, scheme_of, to_css, variables,
    ParsedTheme, Scheme, ThemeError, ThemeWarning, DEFAULT_THEME,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use super::store::{app_themes_dir, read_bounded, StoredTheme, ThemeStore, ThemeStoreError};
use crate::i18n::{t, Lang};
use crate::overlay::editor::{blocking, file_name, lang, Answer, CommandError};
use crate::overlay::store::write_file;
use crate::settings::SettingsStore;

/// Name of the file «Download template» proposes.
const TEMPLATE_FILE: &str = "oma-theme-template.css";

impl From<ThemeStoreError> for CommandError {
    fn from(e: ThemeStoreError) -> Self {
        Self {
            key: e.key().to_owned(),
            detail: e.detail(),
        }
    }
}

impl From<ThemeError> for CommandError {
    fn from(e: ThemeError) -> Self {
        Self {
            key: e.key().to_owned(),
            detail: None,
        }
    }
}

/// A tile of the gallery. `name` is `None` for a built-in (the UI shows
/// `themes.builtin.<id>`); `vars` are merged with the base theme.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeSummary {
    pub id: String,
    pub name: Option<String>,
    pub author: Option<String>,
    pub scheme: Scheme,
    pub builtin: bool,
    pub vars: BTreeMap<String, String>,
}

/// The summary of a file to import, before «Add».
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub token: u64,
    pub name: String,
    pub author: Option<String>,
    pub scheme: Scheme,
    pub vars: BTreeMap<String, String>,
    /// Variables the file sets.
    pub applied: usize,
    /// The parser's warnings, then the contrast ones.
    pub warnings: Vec<ThemeWarning>,
}

/// The variables to apply at start-up; `fallback` when the saved user theme
/// was missing or corrupt and Synthwave took its place.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTheme {
    pub id: String,
    pub vars: BTreeMap<String, String>,
    pub fallback: bool,
}

/// The last preview: token, theme and the file name without extension.
static PENDING: Mutex<Option<(u64, ParsedTheme, String)>> = Mutex::new(None);
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn store() -> Result<ThemeStore, ThemeStoreError> {
    app_themes_dir()
        .map(ThemeStore::new)
        .ok_or_else(|| ThemeStoreError::Io("APPDATA is not set".to_owned()))
}

fn active_theme(app: &AppHandle) -> String {
    app.state::<Arc<SettingsStore>>()
        .snapshot()
        .appearance
        .theme
        .clone()
}

fn summary(id: &str, theme: &ParsedTheme, is_builtin: bool) -> ThemeSummary {
    ThemeSummary {
        id: id.to_owned(),
        name: if is_builtin {
            None
        } else {
            theme.meta.name.clone()
        },
        author: theme.meta.author.clone(),
        scheme: scheme_of(theme),
        builtin: is_builtin,
        vars: resolve(theme),
    }
}

fn user_summary(s: &StoredTheme) -> ThemeSummary {
    summary(&s.id, &s.theme, false)
}

/// The built-ins, then the user themes by name.
pub fn list_summaries(store: &ThemeStore) -> Vec<ThemeSummary> {
    builtins()
        .iter()
        .map(|b| summary(b.id, &b.theme, true))
        .chain(store.list().iter().map(user_summary))
        .collect()
}

/// The theme `id` resolves to; Synthwave when it is unknown (with
/// `fallback` only for a user theme whose file is missing or invalid).
pub fn resolved_for(id: &str, store: Option<&ThemeStore>) -> ResolvedTheme {
    if let Some(b) = builtin(id) {
        return ResolvedTheme {
            id: id.to_owned(),
            vars: resolve(&b.theme),
            fallback: false,
        };
    }
    let user = oma_core::theme::is_valid_theme_id(id);
    if let Some(s) = store.filter(|_| user).and_then(|s| s.get(id).ok()) {
        return ResolvedTheme {
            id: id.to_owned(),
            vars: resolve(&s.theme),
            fallback: false,
        };
    }
    ResolvedTheme {
        id: DEFAULT_THEME.to_owned(),
        vars: resolve(&builtin(DEFAULT_THEME).expect("default theme").theme),
        fallback: user,
    }
}

/// The downloadable template: every variable with a comment on its use (in
/// `lang`) and Synthwave's value.
pub fn template_text(lang: Lang) -> String {
    let synthwave = &builtin(DEFAULT_THEME).expect("default theme").theme.vars;
    let mut s = format!(
        "/* @oma-theme 1\n   @name {}\n   @author */\n:root {{\n",
        t(lang, "themes.template.name", &[])
    );
    for spec in variables() {
        let key = format!("themes.var.{}", spec.name.trim_start_matches("--"));
        // The text goes in a comment: keep `*/` out of it.
        let text = t(lang, &key, &[]).replace("*/", "* /");
        s.push_str(&format!(
            "  /* {text} */\n  {}: {};\n",
            spec.name, synthwave[&spec.name]
        ));
    }
    s.push_str("}\n");
    s
}

/// The preview of `bytes` read from a file named `stem`, kept as the pending import.
pub fn preview(bytes: &[u8], stem: &str) -> Result<ImportPreview, ThemeError> {
    let parsed = parse_theme(bytes)?;
    let vars = resolve(&parsed);
    let mut warnings = parsed.warnings.clone();
    warnings.extend(contrast_warnings(&vars));
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    let out = ImportPreview {
        token,
        name: parsed.meta.name.clone().unwrap_or_else(|| stem.to_owned()),
        author: parsed.meta.author.clone(),
        scheme: scheme_of(&parsed),
        vars,
        applied: parsed.vars.len(),
        warnings,
    };
    *PENDING.lock().unwrap_or_else(PoisonError::into_inner) =
        Some((token, parsed, stem.to_owned()));
    Ok(out)
}

/// Adds the pending import if `token` is the one held.
pub fn confirm(token: u64, store: &ThemeStore) -> Result<ThemeSummary, ThemeStoreError> {
    let mut pending = PENDING.lock().unwrap_or_else(PoisonError::into_inner);
    let Some((_, parsed, stem)) = pending.as_ref().filter(|(held, ..)| *held == token) else {
        return Err(ThemeStoreError::NotFound);
    };
    let added = store.add(parsed, stem)?;
    *pending = None;
    Ok(user_summary(&added))
}

fn into_path(path: tauri_plugin_dialog::FilePath) -> Result<PathBuf, ThemeStoreError> {
    path.into_path()
        .map_err(|e| ThemeStoreError::Io(e.to_string()))
}

/// «Save» dialog over `window` proposing `name`; `None` when cancelled.
fn save_dialog(
    app: &AppHandle,
    window: &WebviewWindow,
    name: String,
) -> Result<Option<PathBuf>, ThemeStoreError> {
    let lang = lang(app);
    app.dialog()
        .file()
        .set_parent(window)
        .set_file_name(name)
        .add_filter(t(lang, "themes.dialog.filter", &[]), &["css"])
        .blocking_save_file()
        .map(into_path)
        .transpose()
}

#[tauri::command]
pub async fn themes_list() -> Answer<Vec<ThemeSummary>> {
    blocking(|| Ok(list_summaries(&store()?))).await
}

#[tauri::command]
pub async fn theme_import_preview(
    app: AppHandle,
    window: WebviewWindow,
) -> Answer<Option<ImportPreview>> {
    blocking(move || {
        let lang = lang(&app);
        let picked = app
            .dialog()
            .file()
            .set_parent(&window)
            .add_filter(t(lang, "themes.dialog.filter", &[]), &["css"])
            .blocking_pick_file();
        let Some(path) = picked else { return Ok(None) };
        let path = into_path(path)?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = read_bounded(&path).map_err(ThemeStoreError::from)?;
        Ok(Some(preview(&bytes, &stem)?))
    })
    .await
}

#[tauri::command]
pub async fn theme_import_confirm(token: u64) -> Answer<ThemeSummary> {
    blocking(move || Ok(confirm(token, &store()?)?)).await
}

#[tauri::command]
pub async fn theme_export(app: AppHandle, window: WebviewWindow, id: String) -> Answer<bool> {
    blocking(move || {
        let stored = store()?.get(&id)?;
        let name = stored.theme.meta.name.clone().unwrap_or_default();
        let Some(path) = save_dialog(&app, &window, file_name(&name, "css"))? else {
            return Ok(false);
        };
        write(&path, &to_css(&stored.theme))?;
        Ok(true)
    })
    .await
}

#[tauri::command]
pub async fn theme_delete(app: AppHandle, id: String) -> Answer<()> {
    blocking(move || {
        store()?.delete(&id)?;
        if active_theme(&app) == id {
            let patch = serde_json::json!({ "appearance": { "theme": DEFAULT_THEME } });
            if let Err(e) = app.state::<Arc<SettingsStore>>().update(&patch) {
                tracing::warn!(?e, "cannot reset the theme after deleting it");
            }
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn theme_resolved(app: AppHandle) -> Answer<ResolvedTheme> {
    blocking(move || Ok(resolved_for(&active_theme(&app), store().ok().as_ref()))).await
}

#[tauri::command]
pub async fn theme_template_save(app: AppHandle, window: WebviewWindow) -> Answer<bool> {
    blocking(move || {
        let Some(path) = save_dialog(&app, &window, TEMPLATE_FILE.to_owned())? else {
            return Ok(false);
        };
        write(&path, &template_text(lang(&app)))?;
        Ok(true)
    })
    .await
}

fn write(path: &Path, text: &str) -> Result<(), ThemeStoreError> {
    Ok(write_file(path, text.as_bytes())?)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::super::store::tests::{temp_dir, theme, UUID_A};
    use super::*;

    fn synthwave() -> BTreeMap<String, String> {
        resolve(&builtin(DEFAULT_THEME).unwrap().theme)
    }

    #[test]
    fn resolved_uses_builtin() {
        let r = resolved_for("builtin-daylight", None);
        assert_eq!(r.id, "builtin-daylight");
        assert!(!r.fallback);
        assert_eq!(r.vars, resolve(&builtin("builtin-daylight").unwrap().theme));
    }

    #[test]
    fn resolved_uses_the_user_file() {
        let dir = temp_dir("resolved");
        let store = ThemeStore::new(dir.clone());
        let a = store.add(&theme(Some("A")), "f").unwrap();
        let r = resolved_for(&a.id, Some(&store));
        assert_eq!((r.id.as_str(), r.fallback), (a.id.as_str(), false));
        assert_eq!(r.vars["--bg"], "#102030");
        assert_eq!(r.vars.len(), variables().len());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolved_falls_back_when_active_file_is_corrupt() {
        let dir = temp_dir("corrupt");
        let store = ThemeStore::new(dir.clone());
        fs::write(dir.join(format!("{UUID_A}.css")), ":root { --bg: #000").unwrap();
        let r = resolved_for(UUID_A, Some(&store));
        assert_eq!(r.id, DEFAULT_THEME);
        assert!(r.fallback);
        assert_eq!(r.vars, synthwave());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolved_unknown_uuid_falls_back() {
        let dir = temp_dir("unknown");
        let store = ThemeStore::new(dir.clone());
        let r = resolved_for(UUID_A, Some(&store));
        assert_eq!((r.id.as_str(), r.fallback), (DEFAULT_THEME, true));
        // No folder at all: the same.
        assert!(resolved_for(UUID_A, None).fallback);
        // A malformed id is Synthwave without the notice.
        let r = resolved_for("../x", Some(&store));
        assert_eq!((r.id.as_str(), r.fallback), (DEFAULT_THEME, false));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn template_lists_every_variable() {
        for lang in [Lang::En, Lang::It] {
            let text = template_text(lang);
            let parsed = parse_theme(text.as_bytes()).unwrap();
            assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
            assert_eq!(parsed.vars, builtin(DEFAULT_THEME).unwrap().theme.vars);
            assert_eq!(parsed.meta.name, Some(t(lang, "themes.template.name", &[])));
            for spec in variables() {
                let key = format!("themes.var.{}", spec.name.trim_start_matches("--"));
                let desc = t(lang, &key, &[]);
                assert_ne!(desc, key, "{key} missing in {lang:?}");
                assert!(text.contains(&format!("/* {desc} */\n  {}:", spec.name)));
            }
        }
        assert_ne!(template_text(Lang::En), template_text(Lang::It));
    }

    #[test]
    fn list_has_builtins_first_without_names() {
        let dir = temp_dir("summaries");
        let store = ThemeStore::new(dir.clone());
        store.add(&theme(Some("Mine")), "f").unwrap();
        let list = list_summaries(&store);
        let ids: Vec<&str> = list.iter().take(4).map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "builtin-synthwave",
                "builtin-midnight",
                "builtin-daylight",
                "builtin-contrast"
            ]
        );
        assert!(list[..4].iter().all(|s| s.builtin && s.name.is_none()));
        assert_eq!(list[2].scheme, Scheme::Light);
        assert_eq!(list[4].name.as_deref(), Some("Mine"));
        assert!(!list[4].builtin);
        assert_eq!(list[4].vars.len(), variables().len());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn preview_then_confirm_adds_once() {
        let dir = temp_dir("import");
        let store = ThemeStore::new(dir.clone());
        // Dark text on the dark base: a contrast warning, and one discarded line.
        let css = b":root { --text: #000000; --nope: 1px; }";
        let p = preview(css, "dark file").unwrap();
        assert_eq!((p.name.as_str(), p.applied), ("dark file", 1));
        assert!(p.warnings.len() >= 2, "{:?}", p.warnings);
        assert_eq!(confirm(p.token + 1, &store), Err(ThemeStoreError::NotFound));
        let added = confirm(p.token, &store).unwrap();
        assert_eq!(added.name.as_deref(), Some("dark file"));
        assert_eq!(confirm(p.token, &store), Err(ThemeStoreError::NotFound));
        assert_eq!(store.list().len(), 1);
        assert_eq!(
            CommandError::from(preview(b"", "x").unwrap_err()).key,
            "themes.error.empty"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
