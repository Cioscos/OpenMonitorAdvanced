//! The user's theme files: `<lowercase uuid v4>.css` in the theme folder, in
//! the normalized form of `to_css`. Every read validates the file again;
//! unreadable, invalid or foreign files are skipped (spec M10 §5.3).

use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use oma_core::overlay::unique_name;
use oma_core::theme::{builtins, is_valid_theme_id, parse_theme, to_css, ParsedTheme};

use crate::i18n::{t, Lang};
use crate::overlay::store::write_file;

/// At most this many user themes.
pub const MAX_THEMES: usize = 100;
/// Longest theme name, in characters (the parser's cap).
const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct StoredTheme {
    pub id: String,
    pub theme: ParsedTheme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeStoreError {
    TooMany,
    Io(String),
    NotFound,
}

impl ThemeStoreError {
    /// The i18n key of the message.
    pub fn key(&self) -> &'static str {
        match self {
            Self::TooMany => "themes.error.tooMany",
            Self::Io(_) => "themes.error.io",
            Self::NotFound => "themes.error.notFound",
        }
    }

    /// The `{detail}` of the message.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Io(detail) => Some(detail.clone()),
            _ => None,
        }
    }
}

impl From<io::Error> for ThemeStoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

/// Reading or deleting a theme file: a missing file is `NotFound`. Writes,
/// exports and template saves keep a vanished folder as `Io`.
fn missing_as_not_found(e: io::Error) -> ThemeStoreError {
    if e.kind() == io::ErrorKind::NotFound {
        ThemeStoreError::NotFound
    } else {
        e.into()
    }
}

/// `%APPDATA%\OpenMonitorAdvanced\themes`.
pub fn app_themes_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|app_data| {
        Path::new(&app_data)
            .join("OpenMonitorAdvanced")
            .join("themes")
    })
}

/// The built-in theme's visible name in `lang` (`themes.builtin.<id>`).
pub fn builtin_name(lang: Lang, id: &str) -> String {
    t(lang, &format!("themes.builtin.{id}"), &[])
}

/// Reads at most `MAX_THEME_BYTES + 1` bytes of `path`, so the parser can
/// tell a file that is too large.
pub fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(oma_core::theme::MAX_THEME_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub struct ThemeStore {
    dir: PathBuf,
}

impl ThemeStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The valid user themes, by name (case-insensitive).
    pub fn list(&self) -> Vec<StoredTheme> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut themes: Vec<StoredTheme> = entries
            .flatten()
            .filter_map(|e| {
                let id = e.file_name().to_str()?.strip_suffix(".css")?.to_owned();
                self.get(&id).ok()
            })
            .collect();
        themes.sort_by_key(|s| (name_of(s).to_lowercase(), s.id.clone()));
        themes
    }

    /// A user theme; `NotFound` for a built-in id, a malformed id, a missing
    /// or an invalid file.
    pub fn get(&self, id: &str) -> Result<StoredTheme, ThemeStoreError> {
        let path = self.path(id)?;
        let theme = parse_theme(&read_bounded(&path).map_err(missing_as_not_found)?)
            .map_err(|_| ThemeStoreError::NotFound)?;
        Ok(StoredTheme {
            id: id.to_owned(),
            theme,
        })
    }

    /// Saves `parsed` under a new id. Its name (or `fallback_name`, the file
    /// name without extension, when it has none) is made unique among the
    /// user themes and the built-ins' names in both languages.
    pub fn add(
        &self,
        parsed: &ParsedTheme,
        fallback_name: &str,
    ) -> Result<StoredTheme, ThemeStoreError> {
        let existing = self.list();
        if existing.len() >= MAX_THEMES {
            return Err(ThemeStoreError::TooMany);
        }
        let mut taken: Vec<String> = existing.iter().map(|s| name_of(s).to_owned()).collect();
        for b in builtins() {
            taken.extend([Lang::En, Lang::It].map(|lang| builtin_name(lang, b.id)));
        }
        let taken: Vec<&str> = taken.iter().map(String::as_str).collect();
        let wanted = parsed
            .meta
            .name
            .clone()
            .unwrap_or_else(|| clean_name(fallback_name));
        let mut name = unique_name(&taken, &wanted);
        if name.chars().count() > MAX_NAME_CHARS {
            // Room for the « (n)» suffix within the parser's cap.
            let short: String = wanted.chars().take(MAX_NAME_CHARS - 6).collect();
            name = unique_name(&taken, short.trim_end());
        }
        let mut theme = parsed.clone();
        theme.meta.name = Some(name);
        theme.warnings.clear();
        let css = to_css(&theme);
        let id = new_id()?;
        write_file(&self.path(&id)?, css.as_bytes())?;
        // What `get` will read back (the header escapes `*/`).
        let theme = parse_theme(css.as_bytes()).map_err(|_| ThemeStoreError::NotFound)?;
        Ok(StoredTheme { id, theme })
    }

    pub fn delete(&self, id: &str) -> Result<(), ThemeStoreError> {
        fs::remove_file(self.path(id)?).map_err(missing_as_not_found)
    }

    /// `<dir>\<id>.css` for a user theme id.
    fn path(&self, id: &str) -> Result<PathBuf, ThemeStoreError> {
        if !is_valid_theme_id(id) || id.starts_with("builtin-") {
            return Err(ThemeStoreError::NotFound);
        }
        Ok(self.dir.join(format!("{id}.css")))
    }
}

fn name_of(s: &StoredTheme) -> &str {
    s.theme.meta.name.as_deref().unwrap_or_default()
}

/// A file name as a theme name: no control characters, at most 64 characters.
fn clean_name(s: &str) -> String {
    let kept: String = s.chars().filter(|c| !c.is_control()).collect();
    let capped: String = kept.trim().chars().take(MAX_NAME_CHARS).collect();
    match capped.trim_end() {
        "" => "Theme".to_owned(),
        name => name.to_owned(),
    }
}

#[cfg(windows)]
fn new_id() -> Result<String, ThemeStoreError> {
    Ok(oma_win::overlay_pipe::random_uuid_v4()?)
}

#[cfg(not(windows))]
fn new_id() -> Result<String, ThemeStoreError> {
    Err(ThemeStoreError::Io(
        "no random source off Windows".to_owned(),
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const UUID_A: &str = "00000000-0000-4000-8000-00000000000a";

    /// A fresh, empty folder under the temp dir, unique to `name`.
    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "oma-themes-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(crate) fn theme(name: Option<&str>) -> ParsedTheme {
        let header = name.map_or(String::new(), |n| format!("/* @name {n} */\n"));
        parse_theme(format!("{header}:root {{ --bg: #102030; }}").as_bytes()).unwrap()
    }

    fn names(store: &ThemeStore) -> Vec<String> {
        store.list().iter().map(|s| name_of(s).to_owned()).collect()
    }

    #[test]
    fn add_writes_normalized_css_with_unique_name() {
        let dir = temp_dir("add");
        let store = ThemeStore::new(dir.clone());
        let a = store.add(&theme(Some("Nord")), "file").unwrap();
        let b = store.add(&theme(Some("nord")), "file").unwrap();
        assert!(is_valid_theme_id(&a.id) && a.id != b.id);
        assert_eq!(a.theme.meta.name.as_deref(), Some("Nord"));
        assert_eq!(b.theme.meta.name.as_deref(), Some("nord (2)"));
        // The file is the normalized form, read back the same.
        let css = fs::read_to_string(dir.join(format!("{}.css", a.id))).unwrap();
        assert_eq!(css, to_css(&a.theme));
        assert_eq!(store.get(&a.id).unwrap(), a);
        // Built-ins' visible names count (both languages): «Synthwave», «Mezzanotte».
        let c = store.add(&theme(Some("synthwave")), "file").unwrap();
        assert_eq!(c.theme.meta.name.as_deref(), Some("synthwave (2)"));
        let d = store.add(&theme(Some("Mezzanotte")), "file").unwrap();
        assert_eq!(d.theme.meta.name.as_deref(), Some("Mezzanotte (2)"));
        // No @name: the file name.
        let e = store.add(&theme(None), "My file").unwrap();
        assert_eq!(e.theme.meta.name.as_deref(), Some("My file"));
        // A taken 64-character name is shortened to stay unique within the cap.
        let long = "x".repeat(64);
        store.add(&theme(Some(&long)), "f").unwrap();
        let f = store.add(&theme(Some(&long)), "f").unwrap();
        assert_eq!(f.theme.meta.name, Some("x".repeat(58)));
        assert_eq!(names(&store).len(), 7);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_skips_invalid_and_foreign_files() {
        let dir = temp_dir("list");
        let store = ThemeStore::new(dir.clone());
        let ok = store.add(&theme(Some("Ok")), "f").unwrap();
        fs::write(dir.join("notes.txt"), ":root { --bg: #000; }").unwrap();
        fs::write(dir.join("x.css"), ":root { --bg: #000; }").unwrap();
        fs::write(dir.join(format!("{UUID_A}.css")), "").unwrap();
        fs::write(
            dir.join("00000000-0000-4000-8000-00000000000b.CSS.txt"),
            ":root { --bg: #000; }",
        )
        .unwrap();
        assert_eq!(store.list(), [ok]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn add_refuses_past_100() {
        let dir = temp_dir("max");
        let store = ThemeStore::new(dir.clone());
        let css = to_css(&theme(Some("T")));
        for i in 0..MAX_THEMES {
            fs::write(
                dir.join(format!("00000000-0000-4000-8000-{i:012}.css")),
                &css,
            )
            .unwrap();
        }
        assert_eq!(store.list().len(), MAX_THEMES);
        assert_eq!(
            store.add(&theme(Some("One more")), "f"),
            Err(ThemeStoreError::TooMany)
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_is_not_found_only_for_read_and_delete() {
        let e = || io::Error::from(io::ErrorKind::NotFound);
        assert!(matches!(ThemeStoreError::from(e()), ThemeStoreError::Io(_)));
        assert_eq!(missing_as_not_found(e()), ThemeStoreError::NotFound);
    }

    #[test]
    fn delete_removes_the_file() {
        let dir = temp_dir("delete");
        let store = ThemeStore::new(dir.clone());
        let a = store.add(&theme(Some("A")), "f").unwrap();
        store.delete(&a.id).unwrap();
        assert!(store.list().is_empty());
        assert_eq!(store.delete(&a.id), Err(ThemeStoreError::NotFound));
        assert_eq!(
            store.delete("builtin-synthwave"),
            Err(ThemeStoreError::NotFound)
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn get_unknown_is_not_found() {
        let dir = temp_dir("get");
        let store = ThemeStore::new(dir.clone());
        for id in [UUID_A, "builtin-synthwave", r"..\x", "x"] {
            assert_eq!(store.get(id), Err(ThemeStoreError::NotFound), "{id}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_map_to_their_keys() {
        assert_eq!(ThemeStoreError::TooMany.key(), "themes.error.tooMany");
        assert_eq!(ThemeStoreError::Io("x".into()).key(), "themes.error.io");
        assert_eq!(ThemeStoreError::NotFound.key(), "themes.error.notFound");
    }
}
