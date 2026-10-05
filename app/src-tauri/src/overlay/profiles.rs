//! The overlay profile catalog: the four built-in templates, then the user's
//! profile files (`<lowercase uuid>.json` in [`profiles_dir`]). Reading is
//! bounded and read-only: a file that is too large or invalid becomes a
//! diagnostic and is never written, renamed or removed (spec §6.5).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use oma_core::model::Schema;
use oma_core::overlay::profile::MAX_PROFILE_BYTES;
use oma_core::overlay::{builtin_profile, parse_profile, BuiltinId, Profile, ProfileError};
use oma_core::settings::overlay::is_profile_id;
use serde::Serialize;

/// The id used when a requested profile does not exist.
pub const FALLBACK_PROFILE: &str = "builtin-gaming";

/// `<APPDATA>\OpenMonitorAdvanced\overlay\profiles`, from the roaming
/// application data folder.
pub fn profiles_dir(app_data: &Path) -> PathBuf {
    app_data
        .join("OpenMonitorAdvanced")
        .join("overlay")
        .join("profiles")
}

/// One profile the user can pick. A built-in's `name` is the catalog key
/// `overlay.template.<id>`, translated by the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileEntry {
    pub id: String,
    pub name: String,
    pub builtin: bool,
}

/// A profile file that could not be used: its file name and the text of the
/// `ProfileError` (or of the I/O error) that rejected it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDiagnostic {
    pub file: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProfileCatalog {
    pub entries: Vec<ProfileEntry>,
    pub diagnostics: Vec<ProfileDiagnostic>,
    /// The valid user profiles, by id.
    user: BTreeMap<String, Profile>,
}

/// Reads the profile folder `dir`. A missing folder is not an error.
pub fn load_catalog(dir: &Path) -> ProfileCatalog {
    let mut catalog = ProfileCatalog {
        entries: BuiltinId::ALL
            .iter()
            .map(|b| ProfileEntry {
                id: b.as_str().to_owned(),
                name: format!("overlay.template.{}", b.as_str()),
                builtin: true,
            })
            .collect(),
        ..ProfileCatalog::default()
    };
    let listing = match fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return catalog,
        Err(e) => {
            catalog.diagnostics.push(ProfileDiagnostic {
                file: dir.display().to_string(),
                reason: e.to_string(),
            });
            return catalog;
        }
    };
    let mut files: Vec<(String, PathBuf)> = listing
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            profile_file_id(&name)?;
            Some((name, entry.path()))
        })
        .collect();
    files.sort();
    for (file, path) in files {
        match read_profile(&path) {
            Ok(None) => {}
            Ok(Some(profile)) => {
                let id = profile_file_id(&file).unwrap_or_default().to_owned();
                catalog.entries.push(ProfileEntry {
                    id: id.clone(),
                    name: profile.name.clone(),
                    builtin: false,
                });
                catalog.user.insert(id, profile);
            }
            Err(reason) => catalog.diagnostics.push(ProfileDiagnostic { file, reason }),
        }
    }
    catalog
}

/// The profile id of a file name `<lowercase uuid>.json`; `None` for any
/// other name, built-in ids included.
fn profile_file_id(name: &str) -> Option<&str> {
    let id = name.strip_suffix(".json")?;
    (is_profile_id(id) && BuiltinId::parse(id).is_none()).then_some(id)
}

/// Reads and validates one profile file without ever reading more than
/// [`MAX_PROFILE_BYTES`] + 1 bytes. `Ok(None)` when it is not a regular file.
fn read_profile(path: &Path) -> Result<Option<Profile>, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Ok(None);
    }
    if meta.len() > MAX_PROFILE_BYTES as u64 {
        return Err(ProfileError::TooLarge.to_string());
    }
    // The file may grow between the size check and the read: bound the read too.
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    File::open(path)
        .and_then(|f| f.take(MAX_PROFILE_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    let text =
        String::from_utf8(bytes).map_err(|e| ProfileError::Json(e.to_string()).to_string())?;
    parse_profile(&text).map(Some).map_err(|e| e.to_string())
}

impl ProfileCatalog {
    /// The id and profile to use for `id`: a built-in bound to `schema`, a
    /// user profile, or «Gaming» for an unknown id.
    pub fn resolve(&self, id: &str, schema: &Schema) -> (String, Profile) {
        if let Some(builtin) = BuiltinId::parse(id) {
            return (id.to_owned(), builtin_profile(builtin, schema));
        }
        match self.user.get(id) {
            Some(profile) => (id.to_owned(), profile.clone()),
            None => (
                FALLBACK_PROFILE.to_owned(),
                builtin_profile(BuiltinId::Gaming, schema),
            ),
        }
    }

    /// The id after `id` in the catalog order, wrapping around.
    /// An unknown id counts as «Gaming», the profile it resolves to.
    pub fn next_after(&self, id: &str) -> String {
        let position = |id: &str| self.entries.iter().position(|e| e.id == id);
        let Some(at) = position(id).or_else(|| position(FALLBACK_PROFILE)) else {
            return FALLBACK_PROFILE.to_owned();
        };
        self.entries[(at + 1) % self.entries.len()].id.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::overlay::profile::MAX_PROFILE_BYTES;
    use std::fs;

    const UUID_A: &str = "00000000-0000-4000-8000-00000000000a";
    const UUID_B: &str = "00000000-0000-4000-8000-00000000000b";

    /// A fresh, empty folder under the temp dir, unique to `name`.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "oma-profiles-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn profile_json(name: &str) -> String {
        serde_json::json!({ "format": 1, "name": name, "blocks": [] }).to_string()
    }

    fn builtin_ids() -> Vec<String> {
        BuiltinId::ALL
            .iter()
            .map(|b| b.as_str().to_owned())
            .collect()
    }

    fn ids(catalog: &ProfileCatalog) -> Vec<String> {
        catalog.entries.iter().map(|e| e.id.clone()).collect()
    }

    #[test]
    fn profiles_dir_is_under_app_data() {
        assert_eq!(
            profiles_dir(Path::new(r"C:\Users\u\AppData\Roaming")),
            PathBuf::from(r"C:\Users\u\AppData\Roaming\OpenMonitorAdvanced\overlay\profiles")
        );
    }

    #[test]
    fn missing_dir_gives_only_builtins() {
        let dir = temp_dir("missing").join("nope");
        let catalog = load_catalog(&dir);
        assert_eq!(ids(&catalog), builtin_ids());
        assert!(catalog.diagnostics.is_empty());
        assert!(catalog.entries.iter().all(|e| e.builtin));
        assert_eq!(catalog.entries[1].name, "overlay.template.builtin-gaming");
    }

    #[test]
    fn user_files_follow_the_builtins_by_name() {
        let dir = temp_dir("order");
        fs::write(dir.join(format!("{UUID_B}.json")), profile_json("Bee")).unwrap();
        fs::write(dir.join(format!("{UUID_A}.json")), profile_json("Ay")).unwrap();
        let catalog = load_catalog(&dir);
        let mut want = builtin_ids();
        want.extend([UUID_A.to_owned(), UUID_B.to_owned()]);
        assert_eq!(ids(&catalog), want);
        assert_eq!(
            catalog.entries[4],
            ProfileEntry {
                id: UUID_A.into(),
                name: "Ay".into(),
                builtin: false
            }
        );
        assert!(catalog.diagnostics.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn invalid_file_gives_a_diagnostic_and_stays_untouched() {
        let dir = temp_dir("invalid");
        let path = dir.join(format!("{UUID_A}.json"));
        let bytes = b"{ \"format\": 1, \"name\": \"x\", \"bogus\": true }".to_vec();
        fs::write(&path, &bytes).unwrap();
        let catalog = load_catalog(&dir);
        assert_eq!(ids(&catalog), builtin_ids());
        assert_eq!(catalog.diagnostics.len(), 1);
        let diag = &catalog.diagnostics[0];
        assert_eq!(diag.file, format!("{UUID_A}.json"));
        assert!(diag.reason.starts_with("invalid profile JSON"), "{diag:?}");
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn files_without_uuid_names_are_ignored() {
        let dir = temp_dir("names");
        for name in [
            "builtin-gaming.json",
            "notes.json",
            "00000000-0000-4000-8000-00000000000A.json",
            "00000000-0000-4000-8000-00000000000a.txt",
            "00000000-0000-4000-8000-00000000000a.json.bak",
        ] {
            fs::write(dir.join(name), profile_json("x")).unwrap();
        }
        fs::create_dir(dir.join(format!("{UUID_B}.json"))).unwrap();
        let catalog = load_catalog(&dir);
        assert_eq!(ids(&catalog), builtin_ids());
        assert!(catalog.diagnostics.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn oversized_file_is_rejected() {
        let dir = temp_dir("oversized");
        let path = dir.join(format!("{UUID_A}.json"));
        // Valid JSON padded with whitespace past the limit.
        let mut text = profile_json("big");
        text.push_str(&" ".repeat(MAX_PROFILE_BYTES));
        fs::write(&path, &text).unwrap();
        let catalog = load_catalog(&dir);
        assert_eq!(ids(&catalog), builtin_ids());
        assert_eq!(
            catalog.diagnostics,
            vec![ProfileDiagnostic {
                file: format!("{UUID_A}.json"),
                reason: oma_core::overlay::ProfileError::TooLarge.to_string(),
            }]
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), text.len() as u64);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_id_resolves_to_gaming() {
        let schema = Schema::default();
        let catalog = load_catalog(&temp_dir("resolve").join("nope"));
        let (id, profile) = catalog.resolve(UUID_A, &schema);
        assert_eq!(id, FALLBACK_PROFILE);
        assert_eq!(profile, builtin_profile(BuiltinId::Gaming, &schema));
        let (id, profile) = catalog.resolve("builtin-full", &schema);
        assert_eq!(id, "builtin-full");
        assert_eq!(profile, builtin_profile(BuiltinId::Full, &schema));
    }

    #[test]
    fn user_profile_resolves_to_its_file() {
        let dir = temp_dir("user");
        fs::write(dir.join(format!("{UUID_A}.json")), profile_json("Mine")).unwrap();
        let catalog = load_catalog(&dir);
        let (id, profile) = catalog.resolve(UUID_A, &Schema::default());
        assert_eq!(id, UUID_A);
        assert_eq!(profile.name, "Mine");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn next_after_wraps() {
        let dir = temp_dir("next");
        fs::write(dir.join(format!("{UUID_A}.json")), profile_json("Mine")).unwrap();
        let catalog = load_catalog(&dir);
        assert_eq!(catalog.next_after("builtin-minimal-fps"), "builtin-gaming");
        assert_eq!(catalog.next_after("builtin-bar"), UUID_A);
        assert_eq!(catalog.next_after(UUID_A), "builtin-minimal-fps");
        // An unknown id stands for «Gaming», which it resolves to.
        assert_eq!(catalog.next_after(UUID_B), "builtin-full");
        fs::remove_dir_all(&dir).unwrap();
    }
}
