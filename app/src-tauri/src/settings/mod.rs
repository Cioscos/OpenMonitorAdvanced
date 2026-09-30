//! Persisted settings of the shell: the file, revisions, the coalescing writer
//! thread and the Tauri commands. The model itself (types, tolerant decoding,
//! strict patches) lives in `oma_core::settings`.

pub mod commands;
#[cfg(test)]
pub(crate) mod fake_fs;
pub mod migrate;
mod store;
mod writer;

use std::io;
use std::path::{Path, PathBuf};

use oma_core::settings::{Diagnostic, Settings};
use serde::Serialize;

pub use store::SettingsStore;

/// Event carrying a [`SettingsState`] after every applied change.
pub const EVENT_SETTINGS: &str = "oma:settings";

/// `%APPDATA%\OpenMonitorAdvanced\settings.json`; `None` without APPDATA.
pub fn settings_path() -> Option<PathBuf> {
    let app_data = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(app_data)
            .join("OpenMonitorAdvanced")
            .join("settings.json"),
    )
}

/// File access of the store, replaceable in tests.
pub trait SettingsFs: Send + Sync {
    /// `Ok(None)` when the file does not exist.
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>>;
    /// Writes a temporary file next to `path`, syncs it and replaces `path`.
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Renames `path` to `to`; fails with `AlreadyExists` instead of overwriting `to`.
    fn preserve(&self, path: &Path, to: &Path) -> io::Result<()>;
    /// Copies `path` to `to`, leaving `path` in place; fails with `AlreadyExists`
    /// instead of overwriting `to`.
    fn copy_exclusive(&self, path: &Path, to: &Path) -> io::Result<()>;
    fn remove(&self, path: &Path) -> io::Result<()>;
}

/// The temporary file `write_atomic` uses next to `path`.
pub(crate) fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

/// The real file system.
pub struct RealFs;

impl SettingsFs for RealFs {
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write as _;

        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = tmp_path(path);
        {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        #[cfg(windows)]
        let replaced = oma_win::fsutil::replace_file(&tmp, path);
        #[cfg(not(windows))]
        let replaced = std::fs::rename(&tmp, path);
        if replaced.is_err() && remove_tmp_after_failed_replace(path.exists()) {
            let _ = std::fs::remove_file(&tmp);
        }
        replaced
    }

    fn preserve(&self, path: &Path, to: &Path) -> io::Result<()> {
        // `rename` overwrites an existing target on Windows, so check first;
        // the name carries a timestamp and the pid, which makes a race moot.
        if to.exists() {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        std::fs::rename(path, to)
    }

    fn copy_exclusive(&self, path: &Path, to: &Path) -> io::Result<()> {
        use std::io::Write as _;

        let bytes = std::fs::read(path)?;
        // `create_new` fails with `AlreadyExists` atomically.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to)?;
        let written = file.write_all(&bytes).and_then(|()| file.sync_all());
        if written.is_err() {
            drop(file);
            let _ = std::fs::remove_file(to);
        }
        written
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
}

/// After a failed replace, the temporary file is garbage only while the target
/// still exists. A failed `ReplaceFileW` can leave the target missing with the
/// data only in the temporary file (e.g. an antivirus holding the new file), so
/// it is kept then, and the next save overwrites it.
pub(crate) fn remove_tmp_after_failed_replace(target_exists: bool) -> bool {
    target_exists
}

/// Where the settings stand on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Persistence {
    /// In sync with the file (or nothing to save yet).
    Ok,
    /// The current revision is not written yet.
    Pending,
    /// The file was corrupt: it was kept at `path` and defaults were saved.
    Recovered { path: String },
    /// Changes stay in memory and are never written.
    ReadOnly { reason: String },
    /// The last save failed (retried), or the file could not be read or preserved.
    Error { reason: String },
}

/// State of an effect outside the settings file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EffectStatus {
    #[default]
    Idle,
    Pending,
    Applied,
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyStatus {
    pub service: EffectStatus,
    pub autostart: EffectStatus,
    pub vendor_libraries: EffectStatus,
}

/// An external effect tracked in [`ApplyStatus`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Service,
    Autostart,
    VendorLibraries,
}

/// Snapshot handed to the UI and to listeners.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsState {
    pub settings: serde_json::Value,
    pub revision: u64,
    pub persisted_revision: u64,
    /// Increases on every emission, so consumers can drop stale events.
    pub seq: u64,
    pub persistence: Persistence,
    pub apply_status: ApplyStatus,
    /// What opening the file had to fix or leave out (rules included), for the
    /// whole session.
    pub diagnostics: Vec<Diagnostic>,
}

/// Called after every applied change and every state change, outside the
/// store lock, in `seq` order.
///
/// A listener may run on any thread that changes the store, including the
/// `oma-settings-writer` thread (which reports saves and save failures). It
/// must therefore be quick and must not call `flush_now` or `shutdown`: on the
/// writer thread they would wait for the writer itself. Calling `update`,
/// `update_with` or `set_effect` is fine; the resulting state is delivered
/// after the current one.
pub type Listener = Box<dyn Fn(&Settings, &SettingsState) + Send + Sync>;

/// `AAAAMMGG-hhmmss` (UTC) of a Unix time.
pub(crate) fn format_stamp(unix_secs: u64) -> String {
    let t = oma_core::csv::local_time(unix_secs.saturating_mul(1_000), 0);
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

/// `settings.json.bad-<stamp>-<pid>`, with `-2`, `-3`… for later attempts.
pub(crate) fn bad_file_name(base: &str, stamp: &str, pid: u32, attempt: u32) -> String {
    if attempt <= 1 {
        format!("{base}.bad-{stamp}-{pid}")
    } else {
        format!("{base}.bad-{stamp}-{pid}-{attempt}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_is_utc_and_zero_padded() {
        assert_eq!(format_stamp(0), "19700101-000000");
        assert_eq!(format_stamp(1_700_000_000), "20231114-221320");
        assert_eq!(format_stamp(951_782_400), "20000229-000000");
    }

    #[test]
    fn bad_file_names_are_unique_per_attempt() {
        assert_eq!(
            bad_file_name("settings.json", "20231114-221320", 42, 1),
            "settings.json.bad-20231114-221320-42"
        );
        assert_eq!(
            bad_file_name("settings.json", "20231114-221320", 42, 3),
            "settings.json.bad-20231114-221320-42-3"
        );
    }

    #[test]
    fn the_temporary_file_is_removed_only_when_the_target_survives() {
        // The target is intact (or already replaced): the temporary copy is garbage.
        assert!(remove_tmp_after_failed_replace(true));
        // The target is missing: the temporary file may hold the only copy.
        assert!(!remove_tmp_after_failed_replace(false));
    }

    #[test]
    fn state_serializes_in_camel_case() {
        let state = SettingsState {
            settings: serde_json::json!({}),
            revision: 2,
            persisted_revision: 1,
            seq: 5,
            persistence: Persistence::Recovered { path: "p".into() },
            apply_status: ApplyStatus {
                vendor_libraries: EffectStatus::Failed { reason: "r".into() },
                ..ApplyStatus::default()
            },
            diagnostics: Vec::new(),
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["persistedRevision"], 1);
        assert_eq!(json["persistence"]["kind"], "recovered");
        assert_eq!(json["persistence"]["path"], "p");
        assert_eq!(json["applyStatus"]["service"]["kind"], "idle");
        assert_eq!(json["applyStatus"]["vendorLibraries"]["kind"], "failed");
        assert_eq!(json["applyStatus"]["vendorLibraries"]["reason"], "r");
    }

    #[test]
    fn real_fs_round_trip_and_preserve() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "oma-realfs-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let path = dir.join("sub").join("settings.json");
        let fs = RealFs;

        assert_eq!(fs.read(&path).unwrap(), None);
        // Creates the folder, then replaces on the second write.
        fs.write_atomic(&path, b"one").unwrap();
        fs.write_atomic(&path, b"two").unwrap();
        assert_eq!(fs.read(&path).unwrap().as_deref(), Some(&b"two"[..]));
        assert!(!dir.join("sub").join("settings.json.tmp").exists());

        // Preserve moves, and refuses to overwrite an existing target.
        let kept = dir.join("sub").join("settings.json.bad-x");
        fs.preserve(&path, &kept).unwrap();
        assert_eq!(fs.read(&path).unwrap(), None);
        fs.write_atomic(&path, b"three").unwrap();
        let err = fs.preserve(&path, &kept).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs.read(&kept).unwrap().as_deref(), Some(&b"two"[..]));

        // Copy keeps the source and refuses to overwrite an existing target.
        let copy = dir.join("sub").join("settings.json.tmp.bad-y");
        fs.copy_exclusive(&path, &copy).unwrap();
        assert_eq!(fs.read(&copy).unwrap().as_deref(), Some(&b"three"[..]));
        assert_eq!(fs.read(&path).unwrap().as_deref(), Some(&b"three"[..]));
        let err = fs.copy_exclusive(&path, &copy).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        fs.remove(&copy).unwrap();

        fs.remove(&kept).unwrap();
        assert_eq!(fs.read(&kept).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
