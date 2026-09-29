//! Small file-system helpers shared with the shell.

use std::path::Path;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    REPLACE_FILE_FLAGS,
};

/// Replaces `target` with `tmp` atomically. `ReplaceFileW` keeps the target's
/// identity and is atomic when it exists; when the target is absent it falls
/// back to `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`.
/// `tmp` no longer exists on success.
pub fn replace_file(tmp: &Path, target: &Path) -> std::io::Result<()> {
    let tmp = HSTRING::from(tmp.as_os_str());
    let target_w = HSTRING::from(target.as_os_str());
    if target.exists() {
        // SAFETY: both paths are NUL-terminated HSTRINGs that outlive the call;
        // there is no backup file (null) and the reserved pointers are absent.
        unsafe {
            ReplaceFileW(
                &target_w,
                &tmp,
                PCWSTR::null(),
                REPLACE_FILE_FLAGS(0),
                None,
                None,
            )
        }
    } else {
        // SAFETY: both paths are NUL-terminated HSTRINGs that outlive the call.
        unsafe {
            MoveFileExW(
                &tmp,
                &target_w,
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    }
    .map_err(std::io::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "oma-fsutil-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn replace_file_replaces_and_creates() {
        let dir = temp_dir("replace");
        let target = dir.join("settings.json");
        let tmp = dir.join("settings.json.tmp");

        // Target absent: created from the temporary file.
        std::fs::write(&tmp, b"first").unwrap();
        replace_file(&tmp, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first");
        assert!(!tmp.exists());

        // Target present: replaced.
        std::fs::write(&tmp, b"second").unwrap();
        replace_file(&tmp, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second");
        assert!(!tmp.exists());

        // Missing temporary file: an error, target untouched.
        assert!(replace_file(&tmp, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"second");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
