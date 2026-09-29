//! Opens a folder, a file or a `ms-settings:` page the way Explorer would.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::{w, PCWSTR};
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// `path` as a NUL-terminated UTF-16 string.
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// `ShellExecuteW` returns a value above 32 on success; below it, the Win32
/// error codes (2, 3, 5, 8, 11) or one of the `SE_ERR_*` codes (26-32, e.g.
/// 31 when no application is associated with the file type).
fn outcome(code: isize) -> io::Result<()> {
    match code {
        c if c > 32 => Ok(()),
        c @ (2 | 3 | 5 | 8 | 11) => Err(io::Error::from_raw_os_error(c as i32)),
        c => Err(io::Error::other(format!("ShellExecuteW failed with {c}"))),
    }
}

/// Opens `path` (a folder, a file or a URI such as `ms-settings:startupapps`)
/// with the `open` verb, on a thread of its own: the shell may run COM
/// extensions, which want an apartment the caller's thread may not have.
/// Returns once the shell has taken the request.
pub fn open(path: &Path) -> io::Result<()> {
    let file = wide(path);
    std::thread::spawn(move || {
        // SAFETY: called once on this new thread, balanced by
        // `CoUninitialize` below only when it succeeded.
        let com =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        // SAFETY: `file` is a NUL-terminated UTF-16 string that outlives the
        // call; the other string arguments are static or null.
        let code = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if com.is_ok() {
            // SAFETY: balances the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        }
        outcome(code.0 as isize)
    })
    .join()
    .map_err(|_| io::Error::other("the shell thread panicked"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_is_nul_terminated() {
        assert_eq!(wide(Path::new("C:\\a")), vec![67, 58, 92, 97, 0]);
    }

    #[test]
    fn shell_execute_results_above_32_are_success() {
        assert!(outcome(33).is_ok());
        assert!(outcome(42).is_ok());
        assert_eq!(outcome(2).unwrap_err().raw_os_error(), Some(2));
        assert_eq!(outcome(3).unwrap_err().raw_os_error(), Some(3));
        let no_association = outcome(31).unwrap_err();
        assert_eq!(no_association.raw_os_error(), None);
        assert!(no_association.to_string().contains("31"));
        assert!(outcome(0).is_err());
    }
}
