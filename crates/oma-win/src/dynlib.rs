//! System32-only dynamic loading of GPU vendor libraries (NVML, NVAPI, ADL, IGCL).
// Remove this attribute when Task 5 adds the optional DXCore loader:
// the `expect` turns into a warning as soon as
// the code is no longer dead.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the GPU vendor layers once they are wired in"
    )
)]

use std::ffi::CStr;

use windows::core::{HSTRING, PCSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

/// A DLL loaded from `%SystemRoot%\System32` only: never from the application
/// directory, the current directory or `PATH`, so a planted DLL is never picked up.
///
/// There is deliberately no `Drop` (decision D1): vendor libraries stay loaded
/// for the whole process lifetime. ADL does not return its memory, NVAPI
/// function pointers dangle after unloading (access violation on the next
/// call) and NVML touches its whole data section again on shutdown.
pub(crate) struct Library(HMODULE);

// SAFETY: an HMODULE is the base address of a mapped image; it is valid on any
// thread and stays valid because the library is never freed.
unsafe impl Send for Library {}

impl Library {
    /// Loads `name` from System32 only (LoadLibraryExW + LOAD_LIBRARY_SEARCH_SYSTEM32).
    pub(crate) fn system32(name: &str) -> windows::core::Result<Self> {
        let name = HSTRING::from(name);
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call;
        // no file handle is passed (it must be None).
        let module = unsafe { LoadLibraryExW(&name, None, LOAD_LIBRARY_SEARCH_SYSTEM32) }?;
        Ok(Self(module))
    }

    /// Resolves the export `symbol` as the function-pointer type `F`.
    ///
    /// # Safety
    /// `F` must be the exact `unsafe extern "C"`/`"system"` fn-pointer type of `symbol`.
    pub(crate) unsafe fn symbol<F: Copy>(&self, symbol: &CStr) -> Option<F> {
        const { assert!(size_of::<F>() == size_of::<usize>()) };
        // SAFETY: `symbol` is NUL-terminated and `self.0` is a loaded module.
        let address = unsafe { GetProcAddress(self.0, PCSTR(symbol.as_ptr().cast())) }?;
        // SAFETY: the caller guarantees that `F` is the export's fn-pointer type,
        // and the size check above rules out non-pointer types.
        Some(unsafe { std::mem::transmute_copy::<_, F>(&address) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type GetTickCount64Fn = unsafe extern "system" fn() -> u64;

    #[test]
    fn loads_a_system32_library_and_resolves_a_symbol() {
        let kernel32 = Library::system32("kernel32.dll").expect("kernel32 from System32");
        // SAFETY: GetTickCount64 takes no arguments and returns a ULONGLONG.
        let tick: GetTickCount64Fn =
            unsafe { kernel32.symbol(c"GetTickCount64") }.expect("GetTickCount64");
        // SAFETY: `tick` is the real GetTickCount64 export.
        assert!(unsafe { tick() } > 0);
    }

    #[test]
    fn missing_symbol_is_none() {
        let kernel32 = Library::system32("kernel32.dll").expect("kernel32 from System32");
        // SAFETY: the symbol does not exist, so no pointer is ever produced.
        let missing: Option<GetTickCount64Fn> = unsafe { kernel32.symbol(c"OmaDoesNotExist") };
        assert!(missing.is_none());
    }

    #[test]
    fn library_absent_from_system32_is_an_error() {
        assert!(Library::system32("oma-does-not-exist.dll").is_err());
    }
}
