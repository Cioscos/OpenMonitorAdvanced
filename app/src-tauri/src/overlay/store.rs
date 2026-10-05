//! The overlay editor's profile files: `<lowercase uuid>.json` in the profile
//! folder. Ids come from the UI and imported files are untrusted: an id is
//! accepted only as a known built-in or a lowercase UUID, the path is always
//! `<dir>\<id>.json`, imports are read bounded and nothing is written unless
//! the profile is valid.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use oma_core::model::Schema;
use oma_core::overlay::{
    builtin_profile, parse_profile, profile_to_json, unique_name, BuiltinId, Profile,
};
use oma_core::settings::overlay::is_profile_id;
use serde::Serialize;

use super::profiles::{check_profile, load_catalog, read_profile, ReadError};
use crate::i18n::{t, Lang};

/// A profile opened in the editor. `json` is the whole profile, defaults
/// included, so the UI reads it without knowing the defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableProfile {
    pub id: String,
    pub builtin: bool,
    pub json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// Neither a known built-in id nor a lowercase UUID.
    InvalidId,
    /// A built-in profile cannot be written or removed.
    ReadOnly,
    NotFound,
    /// The text of the `ProfileError` that rejected the profile.
    Invalid(String),
    Io(String),
}

impl StoreError {
    /// The i18n key of the message.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidId => "editor.error.invalidId",
            Self::ReadOnly => "editor.error.readOnly",
            Self::NotFound => "editor.error.notFound",
            Self::Invalid(_) => "editor.error.invalid",
            Self::Io(_) => "editor.error.io",
        }
    }

    /// The `{detail}` of the message.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Invalid(detail) | Self::Io(detail) => Some(detail.clone()),
            _ => None,
        }
    }
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::NotFound {
            Self::NotFound
        } else {
            Self::Io(e.to_string())
        }
    }
}

impl From<ReadError> for StoreError {
    fn from(e: ReadError) -> Self {
        match e {
            ReadError::Io(e) => e.into(),
            ReadError::Invalid(reason) => Self::Invalid(reason),
        }
    }
}

pub struct ProfileStore {
    dir: PathBuf,
}

impl ProfileStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// A built-in bound to `schema` with its translated name, or a user file.
    pub fn load(
        &self,
        id: &str,
        schema: &Schema,
        lang: Lang,
    ) -> Result<EditableProfile, StoreError> {
        let profile = self.profile(id, schema, lang)?;
        let json = serde_json::to_string(&profile).map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(EditableProfile {
            id: id.to_owned(),
            builtin: BuiltinId::parse(id).is_some(),
            json,
        })
    }

    /// Validates `json` and writes it as `<id>.json`; without `id`, under a
    /// new id with a unique name (DD11). Returns the id.
    pub fn save(&self, id: Option<&str>, json: &str) -> Result<String, StoreError> {
        let path = id.map(|id| self.user_path(id)).transpose()?;
        let profile = parse_profile(json).map_err(|e| StoreError::Invalid(e.to_string()))?;
        match (id, path) {
            (Some(id), Some(path)) => {
                write(&path, &profile)?;
                Ok(id.to_owned())
            }
            _ => self.create(profile),
        }
    }

    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        Ok(fs::remove_file(self.user_path(id)?)?)
    }

    /// A copy of `id` as a new user profile; a built-in is bound to `schema`
    /// by role and named in `lang` (DD10). Returns the new id.
    pub fn duplicate(&self, id: &str, schema: &Schema, lang: Lang) -> Result<String, StoreError> {
        let profile = self.profile(id, schema, lang)?;
        self.create(profile)
    }

    /// Reads at most `MAX_PROFILE_BYTES + 1` bytes of `path`, validates them
    /// and saves the profile under a new id with a unique name.
    pub fn import(&self, path: &Path) -> Result<String, StoreError> {
        let profile = read_profile(path)?.ok_or(StoreError::NotFound)?;
        self.create(profile)
    }

    /// Writes `id` as minimal JSON to `path`.
    pub fn export(
        &self,
        id: &str,
        schema: &Schema,
        lang: Lang,
        path: &Path,
    ) -> Result<(), StoreError> {
        let profile = self.profile(id, schema, lang)?;
        Ok(write_file(path, profile_to_json(&profile).as_bytes())?)
    }

    /// The profile `id`: a built-in bound to `schema` and named in `lang`, or
    /// a user file.
    pub fn profile(&self, id: &str, schema: &Schema, lang: Lang) -> Result<Profile, StoreError> {
        if let Some(builtin) = BuiltinId::parse(id) {
            let mut profile = builtin_profile(builtin, schema);
            profile.name = builtin_name(lang, builtin);
            return Ok(profile);
        }
        read_profile(&self.user_path(id)?)?.ok_or(StoreError::NotFound)
    }

    /// `<dir>\<id>.json` for a user profile id.
    fn user_path(&self, id: &str) -> Result<PathBuf, StoreError> {
        if BuiltinId::parse(id).is_some() {
            return Err(StoreError::ReadOnly);
        }
        if !is_profile_id(id) {
            return Err(StoreError::InvalidId);
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    /// Saves `profile` under a new id, renamed to be unique among the user
    /// profiles and the built-ins' names in both languages.
    fn create(&self, mut profile: Profile) -> Result<String, StoreError> {
        let catalog = load_catalog(&self.dir);
        let mut taken: Vec<String> = catalog
            .entries
            .iter()
            .filter(|e| !e.builtin)
            .map(|e| e.name.clone())
            .collect();
        for builtin in BuiltinId::ALL {
            taken.extend([Lang::En, Lang::It].map(|lang| builtin_name(lang, builtin)));
        }
        let taken: Vec<&str> = taken.iter().map(String::as_str).collect();
        profile.name = unique_name(&taken, &profile.name);
        let id = new_id()?;
        write(&self.user_path(&id)?, &profile)?;
        Ok(id)
    }
}

fn builtin_name(lang: Lang, builtin: BuiltinId) -> String {
    t(lang, &format!("overlay.template.{}", builtin.as_str()), &[])
}

/// Writes `profile` as minimal JSON, atomically, after checking that the
/// catalog and the overlay will accept the file.
fn write(path: &Path, profile: &Profile) -> Result<(), StoreError> {
    let json = profile_to_json(profile);
    check_profile(&json).map_err(StoreError::Invalid)?;
    Ok(write_file(path, json.as_bytes())?)
}

/// Writes `bytes` to a temporary file of its own next to `path`, syncs it and
/// replaces `path` with it. Concurrent writes to one target never share a
/// temporary file, and it is removed on every failure (a profile has no
/// leftover recovery, unlike `settings.json`).
fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let mut name = path.as_os_str().to_owned();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(name);
    // `create_new`: never truncate a file someone else is writing.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| replace(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(windows)]
fn replace(tmp: &Path, path: &Path) -> io::Result<()> {
    oma_win::fsutil::replace_file(tmp, path)
}

#[cfg(not(windows))]
fn replace(tmp: &Path, path: &Path) -> io::Result<()> {
    fs::rename(tmp, path)
}

#[cfg(windows)]
fn new_id() -> Result<String, StoreError> {
    Ok(oma_win::overlay_pipe::random_uuid_v4()?)
}

#[cfg(not(windows))]
fn new_id() -> Result<String, StoreError> {
    Err(StoreError::Io("no random source off Windows".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::overlay::profile::MAX_PROFILE_BYTES;
    use std::collections::BTreeMap;

    const UUID_A: &str = "00000000-0000-4000-8000-00000000000a";

    /// A fresh, empty folder under the temp dir, unique to `name`.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "oma-store-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// File name to content (empty for a folder), for "the folder stays as it was".
    fn listing(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().into_string().unwrap(),
                    fs::read(e.path()).unwrap_or_default(),
                )
            })
            .collect()
    }

    fn profile_json(name: &str) -> String {
        serde_json::json!({
            "format": 1,
            "name": name,
            "scale": 1.0,
            "blocks": [{
                "id": "a",
                "rect": {"x": 0, "y": 0, "w": 10, "h": 2},
                "source": {"text": "hi"},
                "kind": "text"
            }]
        })
        .to_string()
    }

    fn name_of(store_dir: &Path, id: &str) -> String {
        read_profile(&store_dir.join(format!("{id}.json")))
            .unwrap()
            .unwrap()
            .name
    }

    fn this_machine() -> Schema {
        serde_json::from_str(include_str!(
            "../../../../crates/oma-core/tests/fixtures/this-machine-schema.json"
        ))
        .unwrap()
    }

    #[test]
    fn save_writes_minimal_json_atomically_and_reloads() {
        let dir = temp_dir("save");
        let store = ProfileStore::new(dir.clone());
        let json = profile_json("Mine");
        assert_eq!(store.save(Some(UUID_A), &json), Ok(UUID_A.to_owned()));
        let want = profile_to_json(&parse_profile(&json).unwrap());
        let files = listing(&dir);
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            [&format!("{UUID_A}.json")]
        );
        assert_eq!(files[&format!("{UUID_A}.json")], want.as_bytes());
        assert!(!want.contains("scale"), "defaults are left out: {want}");
        let loaded = store.load(UUID_A, &Schema::default(), Lang::En).unwrap();
        assert_eq!(loaded.id, UUID_A);
        assert!(!loaded.builtin);
        assert_eq!(
            parse_profile(&loaded.json).unwrap(),
            parse_profile(&json).unwrap()
        );
        // The whole profile, defaults included.
        assert!(loaded.json.contains("scale"), "{}", loaded.json);
        // Saving again replaces the file.
        store.save(Some(UUID_A), &profile_json("Renamed")).unwrap();
        assert_eq!(name_of(&dir, UUID_A), "Renamed");
        assert_eq!(listing(&dir).len(), 1);
        // An invalid profile writes nothing.
        let before = listing(&dir);
        assert!(matches!(
            store.save(Some(UUID_A), "{\"format\":1}"),
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(listing(&dir), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_without_id_assigns_a_uuid_and_a_unique_name() {
        let dir = temp_dir("save-new");
        let store = ProfileStore::new(dir.clone());
        store.save(Some(UUID_A), &profile_json("Mine")).unwrap();
        let id = store.save(None, &profile_json("mine")).unwrap();
        assert!(
            is_profile_id(&id) && BuiltinId::parse(&id).is_none(),
            "{id}"
        );
        assert_ne!(id, UUID_A);
        assert_eq!(name_of(&dir, &id), "mine (2)");
        // The built-ins' names count too, in both languages.
        let id = store.save(None, &profile_json("Completo")).unwrap();
        assert_eq!(name_of(&dir, &id), "Completo (2)");
        assert_eq!(listing(&dir).len(), 3);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn builtins_are_read_only() {
        let dir = temp_dir("builtins");
        let store = ProfileStore::new(dir.clone());
        let schema = this_machine();
        assert_eq!(
            store.save(Some("builtin-gaming"), &profile_json("x")),
            Err(StoreError::ReadOnly)
        );
        assert_eq!(store.delete("builtin-gaming"), Err(StoreError::ReadOnly));
        let loaded = store.load("builtin-full", &schema, Lang::It).unwrap();
        assert!(loaded.builtin);
        let profile = parse_profile(&loaded.json).unwrap();
        assert_eq!(profile.name, "Completo");
        assert_eq!(
            profile.blocks,
            builtin_profile(BuiltinId::Full, &schema).blocks
        );
        assert!(listing(&dir).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn profile_ids_outside_the_uuid_form_are_rejected() {
        let root = temp_dir("ids");
        let dir = root.join("profiles");
        fs::create_dir_all(&dir).unwrap();
        // A file the bad ids could reach.
        fs::write(root.join("x.json"), profile_json("outside")).unwrap();
        let store = ProfileStore::new(dir.clone());
        let schema = Schema::default();
        let out = root.join("export.json");
        for id in [
            r"..\x",
            r"..\..\x",
            "../x",
            r"C:\x",
            "a/b",
            "a:b",
            "builtin-gaming.json",
            "",
            "00000000-0000-4000-8000-00000000000A",
        ] {
            assert_eq!(
                store.load(id, &schema, Lang::En),
                Err(StoreError::InvalidId),
                "{id}"
            );
            assert_eq!(
                store.save(Some(id), &profile_json("x")),
                Err(StoreError::InvalidId),
                "{id}"
            );
            assert_eq!(store.delete(id), Err(StoreError::InvalidId), "{id}");
            assert_eq!(
                store.duplicate(id, &schema, Lang::En),
                Err(StoreError::InvalidId),
                "{id}"
            );
            assert_eq!(
                store.export(id, &schema, Lang::En, &out),
                Err(StoreError::InvalidId),
                "{id}"
            );
        }
        assert!(listing(&dir).is_empty());
        assert_eq!(listing(&root).len(), 2, "only x.json and profiles/");
        assert_eq!(
            fs::read_to_string(root.join("x.json")).unwrap(),
            profile_json("outside")
        );
        // A well-formed id that has no file.
        assert_eq!(
            store.load(UUID_A, &schema, Lang::En),
            Err(StoreError::NotFound)
        );
        assert_eq!(store.delete(UUID_A), Err(StoreError::NotFound));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn import_rejects_oversized_and_invalid_files_without_writing() {
        let root = temp_dir("import-bad");
        let dir = root.join("profiles");
        let store = ProfileStore::new(dir.clone());
        store.save(Some(UUID_A), &profile_json("Mine")).unwrap();
        let before = listing(&dir);

        let mut oversized = profile_json("big");
        oversized.push_str(&" ".repeat(MAX_PROFILE_BYTES + 1 - oversized.len()));
        assert_eq!(oversized.len(), MAX_PROFILE_BYTES + 1);
        let mut unknown: serde_json::Value = serde_json::from_str(&profile_json("u")).unwrap();
        unknown["id"] = serde_json::json!(UUID_A);
        let overflow = profile_json("o").replace("1.0", "1e999");
        assert!(overflow.contains("1e999"));
        let blocks: Vec<serde_json::Value> = (0..257)
            .map(|i| {
                serde_json::json!({
                    "id": format!("b{i}"),
                    "rect": {"x": 0, "y": 0, "w": 1, "h": 1},
                    "source": {"text": "x"},
                    "kind": "text"
                })
            })
            .collect();
        let many = serde_json::json!({"format": 1, "name": "many", "blocks": blocks}).to_string();
        for (file, text) in [
            ("oversized.omaoverlay.json", oversized),
            ("unknown.omaoverlay.json", unknown.to_string()),
            ("overflow.omaoverlay.json", overflow),
            ("many.omaoverlay.json", many),
            ("utf16.omaoverlay.json", "\u{feff}{}".to_owned()),
        ] {
            let path = root.join(file);
            fs::write(&path, text).unwrap();
            let got = store.import(&path);
            assert!(
                matches!(got, Err(StoreError::Invalid(_))),
                "{file}: {got:?}"
            );
        }
        assert_eq!(
            store.import(&root.join("missing.omaoverlay.json")),
            Err(StoreError::NotFound)
        );
        assert_eq!(store.import(&dir), Err(StoreError::NotFound), "a folder");
        assert_eq!(listing(&dir), before);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn import_assigns_a_new_id_and_a_unique_name() {
        let root = temp_dir("import");
        let dir = root.join("profiles");
        let store = ProfileStore::new(dir.clone());
        store.save(Some(UUID_A), &profile_json("Mine")).unwrap();
        let mine = fs::read(dir.join(format!("{UUID_A}.json"))).unwrap();
        // The file name is never used as an id.
        let path = root.join("builtin-gaming.omaoverlay.json");
        fs::write(&path, profile_json("Mine")).unwrap();
        let id = store.import(&path).unwrap();
        assert!(
            is_profile_id(&id) && BuiltinId::parse(&id).is_none(),
            "{id}"
        );
        assert_ne!(id, UUID_A);
        assert_eq!(name_of(&dir, &id), "Mine (2)");
        assert_eq!(fs::read(dir.join(format!("{UUID_A}.json"))).unwrap(), mine);
        assert_eq!(listing(&dir).len(), 2);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn duplicate_binds_a_builtin_by_role() {
        let dir = temp_dir("duplicate");
        let store = ProfileStore::new(dir.clone());
        let schema = this_machine();
        let id = store
            .duplicate("builtin-gaming", &schema, Lang::En)
            .unwrap();
        let copy = read_profile(&dir.join(format!("{id}.json")))
            .unwrap()
            .unwrap();
        assert_eq!(copy.name, "Gaming (2)");
        assert_eq!(
            copy.blocks,
            builtin_profile(BuiltinId::Gaming, &schema).blocks
        );
        // A user profile duplicates as itself under a new name.
        let again = store.duplicate(&id, &schema, Lang::En).unwrap();
        assert_ne!(again, id);
        assert_eq!(name_of(&dir, &again), "Gaming (3)");
        assert_eq!(listing(&dir).len(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_removes_only_that_file() {
        let dir = temp_dir("delete");
        let store = ProfileStore::new(dir.clone());
        let uuid_b = "00000000-0000-4000-8000-00000000000b";
        store.save(Some(UUID_A), &profile_json("A")).unwrap();
        store.save(Some(uuid_b), &profile_json("B")).unwrap();
        assert_eq!(store.delete(UUID_A), Ok(()));
        assert_eq!(
            listing(&dir).keys().collect::<Vec<_>>(),
            [&format!("{uuid_b}.json")]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn export_writes_minimal_json() {
        let root = temp_dir("export");
        let store = ProfileStore::new(root.join("profiles"));
        let schema = this_machine();
        store.save(Some(UUID_A), &profile_json("Mine")).unwrap();
        let out = root.join("Mine.omaoverlay.json");
        store.export(UUID_A, &schema, Lang::En, &out).unwrap();
        assert_eq!(
            fs::read_to_string(&out).unwrap(),
            profile_to_json(&parse_profile(&profile_json("Mine")).unwrap())
        );
        // A built-in exports bound and named.
        store
            .export("builtin-full", &schema, Lang::It, &out)
            .unwrap();
        let mut want = builtin_profile(BuiltinId::Full, &schema);
        want.name = "Completo".to_owned();
        assert_eq!(fs::read_to_string(&out).unwrap(), profile_to_json(&want));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_no_temp_file() {
        let dir = temp_dir("failed-write");
        let store = ProfileStore::new(dir.clone());
        // A folder where the file should go: the replace fails.
        fs::create_dir(dir.join(format!("{UUID_A}.json"))).unwrap();
        let got = store.save(Some(UUID_A), &profile_json("Mine"));
        assert!(matches!(got, Err(StoreError::Io(_))), "{got:?}");
        assert_eq!(
            listing(&dir).keys().collect::<Vec<_>>(),
            [&format!("{UUID_A}.json")]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn each_write_uses_its_own_temp_file() {
        let dir = temp_dir("own-temp");
        let store = ProfileStore::new(dir.clone());
        // A fixed `<id>.json.tmp` (a leftover, or another writer's) is not in the way.
        fs::create_dir(dir.join(format!("{UUID_A}.json.tmp"))).unwrap();
        assert_eq!(
            store.save(Some(UUID_A), &profile_json("Mine")),
            Ok(UUID_A.to_owned())
        );
        assert_eq!(name_of(&dir, UUID_A), "Mine");
        assert_eq!(listing(&dir).len(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_error_key_is_in_both_catalogs() {
        let catalogs: [serde_json::Value; 2] = [
            serde_json::from_str(include_str!("../../../src/lib/i18n/en.json")).unwrap(),
            serde_json::from_str(include_str!("../../../src/lib/i18n/it.json")).unwrap(),
        ];
        for e in [
            StoreError::InvalidId,
            StoreError::ReadOnly,
            StoreError::NotFound,
            StoreError::Invalid(String::new()),
            StoreError::Io(String::new()),
        ] {
            for catalog in &catalogs {
                assert!(catalog.get(e.key()).is_some(), "{} missing", e.key());
            }
        }
    }
}
