//! Small file-system helpers shared with the shell.

use std::path::Path;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    REPLACEFILE_IGNORE_MERGE_ERRORS, REPLACE_FILE_FLAGS,
};

/// `ERROR_UNABLE_TO_MOVE_REPLACEMENT`: without a backup name, the replaced
/// file no longer exists and the replacement still has its original name.
const ERROR_UNABLE_TO_MOVE_REPLACEMENT: u32 = 1176;
/// `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`: the replacement could not be renamed
/// and the state of the replaced file is not guaranteed.
const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: u32 = 1177;
/// `REPLACEFILE_IGNORE_MERGE_ERRORS`.
const IGNORE_MERGE_ERRORS: REPLACE_FILE_FLAGS = REPLACEFILE_IGNORE_MERGE_ERRORS;

/// The Win32 error code inside an `HRESULT` (`HRESULT_FROM_WIN32` form), if any.
fn win32_code(hresult: i32) -> Option<u32> {
    let value = hresult as u32;
    (value >> 16 == 0x8007).then_some(value & 0xFFFF)
}

/// After a failed `ReplaceFileW`: is the replaced file gone while the
/// replacement is still there, so that a plain move finishes the job?
fn move_fallback_needed(code: Option<u32>, target_exists: bool, tmp_exists: bool) -> bool {
    matches!(
        code,
        Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT | ERROR_UNABLE_TO_MOVE_REPLACEMENT_2)
    ) && !target_exists
        && tmp_exists
}

fn move_over(tmp: &HSTRING, target: &HSTRING) -> windows::core::Result<()> {
    // SAFETY: both paths are NUL-terminated HSTRINGs that outlive the call.
    unsafe {
        MoveFileExW(
            tmp,
            target,
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
}

/// Replaces `target` with `tmp` atomically. `ReplaceFileW` keeps the target's
/// identity and is atomic when it exists; when the target is absent it uses
/// `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`.
/// `tmp` no longer exists on success.
///
/// If `ReplaceFileW` reports that the replacement could not be renamed after
/// the old file was already removed (errors 1176 and 1177), the data is still
/// in `tmp`, so the move is finished with `MoveFileExW` instead of giving up.
/// `REPLACEFILE_IGNORE_MERGE_ERRORS` is set: failing to copy attributes or
/// ACLs from the old file must not stop a settings save (the new file simply
/// keeps the folder's inherited ACL).
pub fn replace_file(tmp: &Path, target: &Path) -> std::io::Result<()> {
    let tmp_w = HSTRING::from(tmp.as_os_str());
    let target_w = HSTRING::from(target.as_os_str());
    let result = if target.exists() {
        // SAFETY: both paths are NUL-terminated HSTRINGs that outlive the call;
        // there is no backup file (null) and the reserved pointers are absent.
        unsafe {
            ReplaceFileW(
                &target_w,
                &tmp_w,
                PCWSTR::null(),
                IGNORE_MERGE_ERRORS,
                None,
                None,
            )
        }
    } else {
        move_over(&tmp_w, &target_w)
    };
    let Err(err) = result else {
        return Ok(());
    };
    let code = win32_code(err.code().0);
    if move_fallback_needed(code, target.exists(), tmp.exists()) {
        return move_over(&tmp_w, &target_w).map_err(to_io_error);
    }
    Err(to_io_error(err))
}

/// An `io::Error` for a Windows error: a `FACILITY_WIN32` HRESULT becomes
/// its Win32 code, so `raw_os_error` gives what callers match on; any other
/// HRESULT is not an OS error code and keeps only its message.
pub(crate) fn to_io_error(err: windows::core::Error) -> std::io::Error {
    match win32_code(err.code().0) {
        Some(code) => std::io::Error::from_raw_os_error(code as i32),
        None => std::io::Error::other(err),
    }
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
    fn win32_code_reads_only_win32_hresults() {
        assert_eq!(win32_code(0x8007_0498_u32 as i32), Some(1176));
        assert_eq!(win32_code(0x8007_0002_u32 as i32), Some(2));
        assert_eq!(win32_code(0x8000_4005_u32 as i32), None);
        assert_eq!(win32_code(0), None);
    }

    #[test]
    fn io_error_keeps_win32_codes_and_wraps_other_hresults() {
        use windows::core::{Error, HRESULT};
        let path_not_found = to_io_error(Error::from_hresult(HRESULT(0x8007_0003_u32 as i32)));
        assert_eq!(path_not_found.raw_os_error(), Some(3));
        assert_eq!(path_not_found.kind(), std::io::ErrorKind::NotFound);
        // E_FAIL is no Win32 code: no raw code, the message is kept.
        let e_fail = Error::from_hresult(HRESULT(0x8000_4005_u32 as i32));
        let message = e_fail.message();
        let other = to_io_error(e_fail);
        assert_eq!(other.raw_os_error(), None);
        assert_eq!(other.kind(), std::io::ErrorKind::Other);
        assert!(other.to_string().contains(&message), "{other}");
    }

    #[test]
    fn move_fallback_only_when_the_old_file_is_gone_and_the_new_one_remains() {
        // 1176 / 1177 with the target missing and the replacement present.
        assert!(move_fallback_needed(Some(1176), false, true));
        assert!(move_fallback_needed(Some(1177), false, true));
        // The target survived (replace did not happen): nothing to finish.
        assert!(!move_fallback_needed(Some(1176), true, true));
        // The replacement is gone too: nothing to move.
        assert!(!move_fallback_needed(Some(1176), false, false));
        // Other errors (sharing violation, access denied, ...) keep both files.
        assert!(!move_fallback_needed(Some(32), false, true));
        assert!(!move_fallback_needed(Some(1175), false, true));
        assert!(!move_fallback_needed(None, false, true));
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
