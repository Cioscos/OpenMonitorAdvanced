//! Prints the DACL of the sensor pipe in SDDL (diagnostic for the M4 checks).
//!
//! Usage: `cargo run -p oma-win --example pipe_sddl [pipe name]`, default
//! `OpenMonitorAdvanced.Sensors.v1`. Opening the pipe takes one instance
//! until this exits; the service session drops it at its `Subscribe` timeout.

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| oma_ipc::PIPE_NAME.to_owned());
    match win::pipe_dacl(&name) {
        Ok(sddl) => {
            println!("{sddl}");
            std::process::ExitCode::SUCCESS
        }
        Err((step, code)) => {
            eprintln!(r"\\.\pipe\{name}: {step} failed with error {code}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("pipe_sddl only runs on Windows");
}

#[cfg(windows)]
mod win {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL, WIN32_ERROR};
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
        SE_KERNEL_OBJECT,
    };
    use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_MODE, OPEN_EXISTING, READ_CONTROL, SECURITY_IDENTIFICATION,
        SECURITY_SQOS_PRESENT,
    };

    fn code(e: &windows::core::Error) -> u32 {
        WIN32_ERROR::from_error(e).map_or(e.code().0 as u32, |w| w.0)
    }

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: this value is the sole owner of the handle, never used after drop.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Memory the security APIs allocated with `LocalAlloc`.
    struct Local(*mut core::ffi::c_void);

    impl Drop for Local {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the pointer came from LocalAlloc inside the API that returned it and
                // is freed once.
                unsafe {
                    let _ = LocalFree(Some(HLOCAL(self.0)));
                }
            }
        }
    }

    /// The pipe's DACL as SDDL; on failure, the failing step and its Win32 code.
    pub fn pipe_dacl(name: &str) -> Result<String, (&'static str, u32)> {
        let path = HSTRING::from(format!(r"\\.\pipe\{name}"));
        // SAFETY: `path` is a valid NUL-terminated wide string for the call; READ_CONTROL only,
        // enough to read the security descriptor. The handle is owned right away.
        let pipe = unsafe {
            CreateFileW(
                &path,
                READ_CONTROL.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        }
        .map(Handle)
        .map_err(|e| ("CreateFileW", code(&e)))?;

        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: live handle opened with READ_CONTROL; `sd` receives a LocalAlloc'd descriptor
        // that the `Local` guard below frees.
        let err = unsafe {
            GetSecurityInfo(
                pipe.0,
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                None,
                None,
                Some(&mut sd),
            )
        };
        let sd = Local(sd.0);
        if err.0 != 0 {
            return Err(("GetSecurityInfo", err.0));
        }

        let mut text = PWSTR::null();
        // SAFETY: `sd` is a valid self-relative descriptor; `text` receives a LocalAlloc'd
        // NUL-terminated string that the `Local` guard below frees.
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                PSECURITY_DESCRIPTOR(sd.0),
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                None,
            )
        }
        .map_err(|e| {
            (
                "ConvertSecurityDescriptorToStringSecurityDescriptorW",
                code(&e),
            )
        })?;
        let text = Local(text.0.cast());
        // SAFETY: `text` is a NUL-terminated wide string, alive until the guard drops.
        unsafe { PWSTR(text.0.cast()).to_string() }.map_err(|_| ("UTF-16 decoding", 0))
    }
}
