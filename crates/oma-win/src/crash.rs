//! Crash marker for safe mode (spec §8): a native crash, typically inside a GPU
//! vendor DLL, leaves a small text file that the next start detects.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HMODULE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, WriteFile, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE,
    FILE_SHARE_NONE,
};
use windows::Win32::System::Diagnostics::Debug::{
    SetUnhandledExceptionFilter, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
    LPTOP_LEVEL_EXCEPTION_FILTER,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};

/// Marker path, prepared at install time so the filter never allocates.
static MARKER: OnceLock<HSTRING> = OnceLock::new();
/// Filter that was installed before ours; called after the marker is written.
static PREVIOUS: OnceLock<LPTOP_LEVEL_EXCEPTION_FILTER> = OnceLock::new();

/// Installs a process-wide unhandled-exception filter that writes "code=0x… module=<path>" to `marker`
/// when a native crash (e.g. inside a GPU vendor DLL) is about to terminate the process. Call once at startup.
///
/// Only unhandled SEH exceptions (access violations and the like) reach the
/// filter: a normal exit, a kill, a Rust panic (caught per thread) or an abort
/// (`__fastfail`) never write the marker. Later calls are ignored.
pub fn install_crash_marker(marker: PathBuf) {
    if let Some(dir) = marker.parent().filter(|d| !d.as_os_str().is_empty()) {
        if let Err(err) = std::fs::create_dir_all(dir) {
            tracing::warn!(%err, dir = %dir.display(), "cannot create the crash marker directory");
        }
    }
    if MARKER.set(HSTRING::from(marker.as_os_str())).is_err() {
        tracing::warn!("crash marker filter already installed");
        return;
    }
    // SAFETY: `write_marker` has the filter signature and only reads 'static data.
    let previous = unsafe { SetUnhandledExceptionFilter(Some(write_marker)) };
    let _ = PREVIOUS.set(previous);
}

/// What `rearm` found in place of our filter.
#[derive(Debug, PartialEq, Eq)]
enum Rearm {
    NotInstalled,
    Kept,
    Restored,
}

fn address(filter: LPTOP_LEVEL_EXCEPTION_FILTER) -> usize {
    filter.map_or(0, |f| f as usize)
}

fn rearm() -> Rearm {
    // PREVIOUS is set last by `install_crash_marker`: before that there is nothing to re-arm.
    if PREVIOUS.get().is_none() {
        return Rearm::NotInstalled;
    }
    // SAFETY: as in `install_crash_marker`. PREVIOUS keeps the filter found at
    // install time, so ours never chains to itself.
    let current = unsafe { SetUnhandledExceptionFilter(Some(write_marker)) };
    if address(current) == address(Some(write_marker)) {
        Rearm::Kept
    } else {
        Rearm::Restored
    }
}

/// Puts the crash marker filter back in place if a component loaded after
/// `install_crash_marker` (the WebView2 window, a GPU vendor DLL) replaced it.
/// The replacing filter is not chained. No-op when the marker was never installed.
pub fn rearm_crash_marker() {
    if rearm() == Rearm::Restored {
        tracing::warn!("the crash marker filter had been replaced; re-armed");
    }
}

/// Reads and deletes the marker left by a previous run that crashed; None if there is none.
pub fn take_crash_marker(marker: &Path) -> Option<String> {
    let bytes = match std::fs::read(marker) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!(%err, path = %marker.display(), "cannot read the crash marker");
            return None;
        }
    };
    if let Err(err) = std::fs::remove_file(marker) {
        tracing::warn!(%err, path = %marker.display(), "cannot delete the crash marker");
    }
    Some(String::from_utf8_lossy(&bytes).trim().to_owned())
}

/// Top-level filter. It runs on the crashing thread while the process is in an
/// unknown state (the heap may be corrupt or its lock held), so it uses only
/// stack buffers and direct Win32 calls: no allocation, no locks.
unsafe extern "system" fn write_marker(info: *const EXCEPTION_POINTERS) -> i32 {
    // SAFETY: the system passes a valid EXCEPTION_POINTERS (null is tolerated anyway).
    let record = unsafe { info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) };
    if let (Some(path), Some(record)) = (MARKER.get(), record) {
        let mut name = [0u16; 260];
        let mut len = 0;
        let mut module = HMODULE::default();
        // SAFETY: with FROM_ADDRESS the "name" argument is any address inside the
        // module; UNCHANGED_REFCOUNT means there is no handle to release.
        let found = unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                PCWSTR(record.ExceptionAddress as *const u16),
                &mut module,
            )
        };
        if found.is_ok() {
            // SAFETY: `name` is a writable buffer; the result is the length written.
            len = unsafe { GetModuleFileNameW(Some(module), &mut name) } as usize;
        }
        let mut text = [0u8; 1024];
        let written = marker_text(
            record.ExceptionCode.0 as u32,
            &name[..len.min(name.len())],
            &mut text,
        );
        // SAFETY: `path` is a NUL-terminated wide string; the handle is closed below.
        let file = unsafe {
            CreateFileW(
                path,
                FILE_GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                CREATE_ALWAYS,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        };
        if let Ok(file) = file {
            // SAFETY: `file` is an open handle and `text[..written]` is initialised.
            unsafe {
                let _ = WriteFile(file, Some(&text[..written]), None, None);
                let _ = CloseHandle(file);
            }
        }
    }
    match PREVIOUS.get().copied().flatten() {
        // SAFETY: `previous` was the process's top-level filter before ours.
        Some(previous) => unsafe { previous(info) },
        None => EXCEPTION_CONTINUE_SEARCH,
    }
}

/// Writes `code=0x… module=<path>` into `out` without allocating; stops before
/// a piece that does not fit, so the text stays valid UTF-8. Returns the length.
fn marker_text(code: u32, module: &[u16], out: &mut [u8]) -> usize {
    struct Cursor<'a> {
        out: &'a mut [u8],
        len: usize,
    }
    impl std::fmt::Write for Cursor<'_> {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            let end = self.len + s.len();
            if end > self.out.len() {
                return Err(std::fmt::Error);
            }
            self.out[self.len..end].copy_from_slice(s.as_bytes());
            self.len = end;
            Ok(())
        }
    }
    let mut cursor = Cursor { out, len: 0 };
    if write!(cursor, "code={code:#x} module=").is_ok() {
        for c in char::decode_utf16(module.iter().copied()) {
            let c = c.unwrap_or(char::REPLACEMENT_CHARACTER);
            if cursor.write_char(c).is_err() {
                break;
            }
        }
    }
    cursor.len
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Diagnostics::Debug::{SetErrorMode, SEM_NOGPFAULTERRORBOX};

    const CHILD_ENV: &str = "OMA_CRASH_CHILD";
    const REARM_CHILD_ENV: &str = "OMA_REARM_CHILD";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn temp_marker(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("oma-crash-{tag}-{}.txt", std::process::id()))
    }

    #[test]
    fn marker_text_has_code_and_module() {
        let mut out = [0u8; 128];
        let n = marker_text(
            0xC000_0005,
            &wide(r"C:\Windows\System32\nvml.dll"),
            &mut out,
        );
        assert_eq!(
            std::str::from_utf8(&out[..n]).unwrap(),
            r"code=0xc0000005 module=C:\Windows\System32\nvml.dll"
        );
    }

    #[test]
    fn marker_text_without_module_and_with_non_ascii_path() {
        let mut out = [0u8; 128];
        let n = marker_text(0xC000_00FD, &[], &mut out);
        assert_eq!(&out[..n], b"code=0xc00000fd module=");
        let n = marker_text(0x8000_0003, &wide(r"C:\Programmi\città.dll"), &mut out);
        assert_eq!(
            std::str::from_utf8(&out[..n]).unwrap(),
            r"code=0x80000003 module=C:\Programmi\città.dll"
        );
    }

    #[test]
    fn marker_text_truncates_on_a_character_boundary() {
        // "code=0xc0000005 module=" is 23 bytes; "a" (1) and "à" (2) fit in 26, "b" does not.
        let mut out = [0u8; 26];
        let n = marker_text(0xC000_0005, &wide("aàb"), &mut out);
        assert_eq!(
            std::str::from_utf8(&out[..n]).unwrap(),
            "code=0xc0000005 module=aà"
        );
        // "à" alone does not fit in 24 bytes and is not split.
        let mut out = [0u8; 24];
        let n = marker_text(0xC000_0005, &wide("àb"), &mut out);
        assert_eq!(&out[..n], b"code=0xc0000005 module=");
    }

    #[test]
    fn take_without_a_marker_is_none() {
        let marker = temp_marker("absent");
        let _ = std::fs::remove_file(&marker);
        assert_eq!(take_crash_marker(&marker), None);
    }

    #[test]
    fn take_reads_and_deletes_the_marker() {
        let marker = temp_marker("take");
        std::fs::write(&marker, "code=0xc0000005 module=C:\\x\\nvml.dll").unwrap();
        assert_eq!(
            take_crash_marker(&marker).as_deref(),
            Some("code=0xc0000005 module=C:\\x\\nvml.dll")
        );
        assert!(!marker.exists());
        assert_eq!(take_crash_marker(&marker), None);
    }

    /// Re-launches this test binary running only `crash_child`, which installs
    /// the filter and crashes with an access violation on a spawned thread.
    #[test]
    fn crash_marker_written_on_native_crash() {
        let marker = temp_marker("child");
        let _ = std::fs::remove_file(&marker);
        let exe = std::env::current_exe().expect("test binary path");
        let output = std::process::Command::new(&exe)
            .args([
                "--exact",
                "crash::tests::crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ENV, &marker)
            .output()
            .expect("start the crashing child");
        assert!(!output.status.success(), "the child must crash: {output:?}");
        let text = take_crash_marker(&marker).expect("marker written by the filter");
        assert!(text.contains("0xc0000005"), "{text}");
        // The faulting instruction is in the test binary itself.
        let exe_name = exe.file_name().unwrap().to_string_lossy().to_lowercase();
        assert!(text.to_lowercase().ends_with(&exe_name), "{text}");
        assert!(!marker.exists());
    }

    /// Re-launches this test binary running only `rearm_child`: the top-level
    /// filter is process-wide, so the checks run in a process of their own.
    #[test]
    fn rearm_restores_only_our_filter() {
        let marker = temp_marker("rearm");
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "crash::tests::rearm_child",
                "--ignored",
                "--nocapture",
            ])
            .env(REARM_CHILD_ENV, &marker)
            .output()
            .expect("start the re-arm child");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("rearm-child-ok"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!marker.exists(), "no crash, so no marker");
    }

    unsafe extern "system" fn other_filter(_: *const EXCEPTION_POINTERS) -> i32 {
        EXCEPTION_CONTINUE_SEARCH
    }

    unsafe extern "system" fn intruder_filter(_: *const EXCEPTION_POINTERS) -> i32 {
        EXCEPTION_CONTINUE_SEARCH
    }

    /// Address of the current top-level filter (0 for none), left in place.
    fn current_filter() -> usize {
        // SAFETY: swaps the filter out and straight back in.
        let current = unsafe { SetUnhandledExceptionFilter(None) };
        // SAFETY: as above.
        unsafe { SetUnhandledExceptionFilter(current) };
        address(current)
    }

    #[test]
    #[ignore = "child process of rearm_restores_only_our_filter"]
    fn rearm_child() {
        // Without the variable (e.g. under `--include-ignored`) this is a no-op.
        let Some(marker) = std::env::var_os(REARM_CHILD_ENV) else {
            return;
        };
        let other = address(Some(other_filter));
        // SAFETY: test filters with the right signature, in this child only.
        unsafe { SetUnhandledExceptionFilter(Some(other_filter)) };

        // Not installed: nothing changes.
        assert_eq!(rearm(), Rearm::NotInstalled);
        assert_eq!(current_filter(), other);

        install_crash_marker(PathBuf::from(marker));
        let ours = address(Some(write_marker));
        assert_eq!(current_filter(), ours);
        assert_eq!(rearm(), Rearm::Kept);
        assert_eq!(current_filter(), ours);

        // Another component (e.g. a vendor DLL) replaces the filter: re-arm puts
        // ours back and keeps chaining to the original one, not to the intruder.
        // SAFETY: as above.
        unsafe { SetUnhandledExceptionFilter(Some(intruder_filter)) };
        assert_eq!(rearm(), Rearm::Restored);
        assert_eq!(current_filter(), ours);
        assert_eq!(address(PREVIOUS.get().copied().flatten()), other);
        assert_eq!(rearm(), Rearm::Kept);
        rearm_crash_marker();
        assert_eq!(current_filter(), ours);
        println!("rearm-child-ok");
    }

    #[test]
    #[ignore = "child process of crash_marker_written_on_native_crash"]
    fn crash_child() {
        // Without the variable (e.g. under `--include-ignored`) this is a no-op.
        let Some(marker) = std::env::var_os(CHILD_ENV) else {
            return;
        };
        // SAFETY: only changes this child's error mode: no Windows Error
        // Reporting dialog for the intentional crash. The filter still runs.
        unsafe { SetErrorMode(SEM_NOGPFAULTERRORBOX) };
        install_crash_marker(PathBuf::from(marker));
        let _ = std::thread::spawn(|| {
            // SAFETY: deliberately invalid: 0x10 lies in the never-mapped first
            // page, so the read raises an access violation (0xC0000005).
            unsafe { std::ptr::read_volatile(std::ptr::without_provenance::<u32>(0x10)) }
        })
        .join();
        unreachable!("the access violation must terminate the process");
    }
}
