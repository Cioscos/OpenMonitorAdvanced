//! Well-known shell folders.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

use crate::fsutil::to_io_error;

/// The current user's Documents folder, wherever it has been redirected to.
pub fn documents_dir() -> io::Result<PathBuf> {
    // SAFETY: `FOLDERID_Documents` is a valid GUID; a null token means the
    // current user.
    let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) }
        .map_err(to_io_error)?;
    // SAFETY: on success `raw` is a NUL-terminated UTF-16 string allocated by
    // the shell; it is read once here and freed right after.
    let path = unsafe { OsString::from_wide(raw.as_wide()) };
    // SAFETY: `raw` came from `SHGetKnownFolderPath`, which requires the
    // caller to release it with `CoTaskMemFree`; it is not used afterwards.
    unsafe { CoTaskMemFree(Some(raw.as_ptr() as *const _)) };
    Ok(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_dir_exists_here() {
        let dir = documents_dir().expect("documents folder");
        assert!(dir.is_absolute());
        assert!(dir.is_dir(), "{dir:?}");
    }
}
