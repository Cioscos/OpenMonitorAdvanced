//! Opens a folder, a file or a `ms-settings:` page the way Explorer would.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::sync::mpsc;
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Why a target was not opened.
#[derive(Debug)]
pub enum OpenError {
    /// The shell did not answer within the timeout; its thread is left running.
    TimedOut,
    /// The shell refused: a Win32 error or an `SE_ERR_*` code.
    Os(io::Error),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut => f.write_str("the shell did not respond in time"),
            Self::Os(err) => err.fmt(f),
        }
    }
}

/// `target` as a NUL-terminated UTF-16 string.
fn wide(target: &OsStr) -> Vec<u16> {
    target.encode_wide().chain(Some(0)).collect()
}

/// The legacy `hInstApp` / `ShellExecute` result: a value above 32 on success;
/// below it, the Win32 error codes (2, 3, 5, 8, 11) or one of the `SE_ERR_*`
/// codes (26-32, e.g. 31 when no application is associated with the file type).
fn outcome(code: isize) -> Result<(), OpenError> {
    match code {
        c if c > 32 => Ok(()),
        c @ (2 | 3 | 5 | 8 | 11) => Err(OpenError::Os(io::Error::from_raw_os_error(c as i32))),
        c => Err(OpenError::Os(io::Error::other(format!(
            "ShellExecuteExW failed with {c}"
        )))),
    }
}

/// The error of a `ShellExecuteExW` that returned FALSE: the `hInstApp` code
/// when it carries one (it also names the `SE_ERR_*` cases), else the HRESULT
/// windows-rs reported, with a Win32-facility one turned back into its Win32
/// code. A zero code names no cause and becomes a plain failure.
fn failure(hinst_app: isize, hresult: i32) -> Result<(), OpenError> {
    if (1..=32).contains(&hinst_app) {
        return outcome(hinst_app);
    }
    let bits = hresult as u32;
    let code = if bits & 0xFFFF_0000 == 0x8007_0000 {
        (bits & 0xFFFF) as i32
    } else {
        hresult
    };
    Err(OpenError::Os(if code == 0 {
        io::Error::other("ShellExecuteExW failed")
    } else {
        io::Error::from_raw_os_error(code)
    }))
}

/// The caller's side of the hand-off: the shell thread's answer, or `TimedOut`.
fn wait_with_timeout(
    rx: &mpsc::Receiver<Result<(), OpenError>>,
    timeout: Duration,
) -> Result<(), OpenError> {
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(OpenError::TimedOut),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(OpenError::Os(io::Error::other(
            "the shell thread ended without an answer",
        ))),
    }
}

/// Runs `ShellExecuteExW` with the `open` verb. Must run on a thread that may
/// block: the shell can load COM extensions and hang.
fn shell_execute(file: &[u16]) -> Result<(), OpenError> {
    // SAFETY: called once on this thread, balanced by `CoUninitialize` below
    // only when it succeeded.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is fully initialised with its true size; `lpFile` points
    // into `file`, a NUL-terminated UTF-16 string that outlives the call, and
    // `lpVerb` is static. No other pointer in it is set.
    let result = unsafe { ShellExecuteExW(&mut info) };
    let outcome = match result {
        Ok(()) => Ok(()),
        Err(err) => failure(info.hInstApp.0 as isize, err.code().0),
    };
    if com.is_ok() {
        // SAFETY: balances the successful `CoInitializeEx` above.
        unsafe { CoUninitialize() };
    }
    outcome
}

/// Opens `target` (a folder, a file or a URI such as `ms-settings:startupapps`)
/// with the `open` verb, on a thread of its own, and waits at most `timeout`
/// for the shell to take the request. On a timeout the thread is left to
/// finish (or hang) on its own and the result is [`OpenError::TimedOut`].
pub fn open(target: &OsStr, timeout: Duration) -> Result<(), OpenError> {
    let file = wide(target);
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("oma-shell-open".to_owned())
        .spawn(move || {
            // The receiver is gone after a timeout: nobody wants the answer.
            let _ = tx.send(shell_execute(&file));
        })
        .map_err(OpenError::Os)?;
    wait_with_timeout(&rx, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_is_nul_terminated() {
        assert_eq!(wide(OsStr::new(r"C:\a")), vec![67, 58, 92, 97, 0]);
    }

    #[test]
    fn shell_execute_results_above_32_are_success() {
        assert!(outcome(33).is_ok());
        assert!(outcome(42).is_ok());
        assert!(outcome(0).is_err());
    }

    #[test]
    fn code_2_is_a_missing_file() {
        let Err(OpenError::Os(err)) = outcome(2) else {
            panic!("code 2 must be an OS error");
        };
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(err.raw_os_error(), Some(2));
        let Err(OpenError::Os(err)) = outcome(3) else {
            panic!("code 3 must be an OS error");
        };
        assert_eq!(err.raw_os_error(), Some(3));
    }

    #[test]
    fn shell_error_codes_without_an_os_equivalent_keep_their_number() {
        let Err(OpenError::Os(err)) = outcome(31) else {
            panic!("code 31 must be an OS error");
        };
        assert_eq!(err.raw_os_error(), None);
        assert!(err.to_string().contains("31"));
    }

    #[test]
    fn a_failure_prefers_the_legacy_code_and_falls_back_to_the_error_code() {
        // HRESULT_FROM_WIN32(ERROR_ACCESS_DENIED), as windows-rs reports it.
        let access_denied = 0x8007_0005_u32 as i32;
        let Err(OpenError::Os(err)) = failure(2, access_denied) else {
            panic!("expected an OS error");
        };
        assert_eq!(err.raw_os_error(), Some(2));
        let Err(OpenError::Os(err)) = failure(0, access_denied) else {
            panic!("expected an OS error");
        };
        assert_eq!(err.raw_os_error(), Some(5), "Win32 facility: the low word");
    }

    #[test]
    fn a_failure_keeps_other_hresults_and_never_reports_success_as_the_cause() {
        let e_fail = 0x8000_4005_u32 as i32;
        let Err(OpenError::Os(err)) = failure(0, e_fail) else {
            panic!("expected an OS error");
        };
        assert_eq!(err.raw_os_error(), Some(e_fail));
        let Err(OpenError::Os(err)) = failure(0, 0) else {
            panic!("expected an OS error");
        };
        assert_eq!(err.raw_os_error(), None);
        assert_eq!(err.to_string(), "ShellExecuteExW failed");
    }

    #[test]
    fn a_silent_shell_thread_times_out() {
        let (_tx, rx) = mpsc::channel::<Result<(), OpenError>>();
        let started = std::time::Instant::now();
        let result = wait_with_timeout(&rx, Duration::from_millis(50));
        assert!(matches!(result, Err(OpenError::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_reply_is_passed_on_and_a_dead_thread_is_an_error() {
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(())).unwrap();
        assert!(wait_with_timeout(&rx, Duration::from_secs(1)).is_ok());
        let (tx, rx) = mpsc::channel::<Result<(), OpenError>>();
        drop(tx);
        assert!(matches!(
            wait_with_timeout(&rx, Duration::from_secs(1)),
            Err(OpenError::Os(_))
        ));
    }
}
