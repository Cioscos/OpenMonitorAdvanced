//! The test files of the disk tests (DC4): names, the sidecar that lets the app clean up
//! orphans, and (Windows) the unbuffered overlapped data files. Both files are opened with
//! `FILE_FLAG_DELETE_ON_CLOSE`, so the system removes them when the handle goes away, even
//! if the process crashes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskError {
    Full,
    AccessDenied,
    Io(u32),
}

const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_HANDLE_DISK_FULL: u32 = 39;
const ERROR_DISK_FULL: u32 = 112;
#[cfg(windows)]
const ERROR_INVALID_NAME: u32 = 123;

pub fn classify_win32(code: u32) -> DiskError {
    match code {
        ERROR_DISK_FULL | ERROR_HANDLE_DISK_FULL => DiskError::Full,
        ERROR_ACCESS_DENIED => DiskError::AccessDenied,
        other => DiskError::Io(other),
    }
}

/// `oma-test-<seed as 16 hex digits>`: the name every file of one run starts with.
pub fn file_prefix(seed: u64) -> String {
    format!("oma-test-{seed:016x}")
}

/// The content of the file next to the data: who owns the files and since when.
pub fn sidecar_json(pid: u32, started_at: u64, prefix: &str) -> String {
    format!(r#"{{"format":1,"pid":{pid},"startedAt":{started_at},"prefix":"{prefix}"}}"#)
}

/// `<prefix>[suffix].bin`; the suffix, if any, is `-` plus ASCII letters and digits
/// (`-3`, `-sync`). Anything else gives `None`.
pub fn data_name(prefix: &str, suffix: Option<&str>) -> Option<String> {
    let s = match suffix {
        None => "",
        Some(s) => {
            let tail = s.strip_prefix('-')?;
            if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return None;
            }
            s
        }
    };
    Some(format!("{prefix}{s}.bin"))
}

#[cfg(windows)]
pub use win::{DataFile, TestFiles};

#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Path, PathBuf};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_IO_PENDING, FILETIME, GENERIC_READ, GENERIC_WRITE, HANDLE,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FileEndOfFileInfo, FileStorageInfo, FlushFileBuffers, GetDiskFreeSpaceExW,
        GetFileInformationByHandleEx, SetFileInformationByHandle, WriteFile, CREATE_NEW, DELETE,
        FILE_END_OF_FILE_INFO, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_DELETE_ON_CLOSE,
        FILE_FLAG_NO_BUFFERING, FILE_FLAG_OVERLAPPED, FILE_FLAG_WRITE_THROUGH, FILE_SHARE_DELETE,
        FILE_SHARE_MODE, FILE_SHARE_NONE, FILE_SHARE_READ, FILE_STORAGE_INFO,
    };
    use windows::Win32::System::Ioctl::FSCTL_SET_COMPRESSION;
    use windows::Win32::System::Threading::{
        CreateEventW, GetCurrentProcess, GetCurrentProcessId, GetProcessTimes,
    };
    use windows::Win32::System::IO::{DeviceIoControl, GetOverlappedResult, OVERLAPPED};

    // Compile-time size checks of the FFI structs passed by pointer.
    const _: () = assert!(std::mem::size_of::<FILE_END_OF_FILE_INFO>() == 8);
    const _: () = assert!(std::mem::size_of::<FILE_STORAGE_INFO>() == 28);

    /// A handle closed on drop.
    struct Owned(HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the handle is valid and owned by this wrapper; closed exactly once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    // SAFETY: a kernel handle can be used and closed from any thread.
    unsafe impl Send for Owned {}

    pub struct DataFile {
        owned: Owned,
        pub bytes: u64,
        pub sector: u32,
    }

    impl DataFile {
        /// The handle, for the IOCP association and the I/O calls; valid while the
        /// `DataFile` lives.
        pub fn handle(&self) -> HANDLE {
            self.owned.0
        }

        pub fn flush(&self) -> Result<(), DiskError> {
            // SAFETY: the handle is a valid open file with write access.
            unsafe { FlushFileBuffers(self.owned.0) }.map_err(|e| err(&e))
        }
    }

    pub struct TestFiles {
        dir: PathBuf,
        prefix: String,
        reserve: u64,
        // Keeps the sidecar open (and so alive) until the run ends.
        _sidecar: Owned,
    }

    fn err(e: &windows::core::Error) -> DiskError {
        let hr = e.code().0 as u32;
        // HRESULT_FROM_WIN32 keeps the code in the low 16 bits.
        classify_win32(if hr >> 16 == 0x8007 { hr & 0xFFFF } else { hr })
    }

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn free_bytes(dir: &Path) -> Result<u64, DiskError> {
        let w = wide(dir);
        let mut avail = 0u64;
        // SAFETY: `w` is NUL-terminated and `avail` outlives the call.
        unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut avail), None, None) }
            .map_err(|e| err(&e))?;
        Ok(avail)
    }

    /// `free - bytes >= reserve`, with no overflow whatever the inputs.
    fn fits(free: u64, bytes: u64, reserve: u64) -> bool {
        free.checked_sub(bytes).is_some_and(|rest| rest >= reserve)
    }

    fn started_at() -> u64 {
        let zero = FILETIME::default();
        let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
        // SAFETY: the pseudo handle is always valid; the four outputs live through the call.
        let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
        match ok {
            Ok(()) => (u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime),
            Err(_) => 0,
        }
    }

    fn create_new(
        path: &Path,
        access: u32,
        share: FILE_SHARE_MODE,
        flags: FILE_FLAGS_AND_ATTRIBUTES,
    ) -> Result<Owned, DiskError> {
        let w = wide(path);
        // SAFETY: `w` is NUL-terminated; no security attributes or template file are passed.
        let h = unsafe {
            CreateFileW(
                PCWSTR(w.as_ptr()),
                access,
                share,
                None,
                CREATE_NEW,
                flags,
                None,
            )
        }
        .map_err(|e| err(&e))?;
        Ok(Owned(h))
    }

    impl TestFiles {
        /// Checks the free space against `reserve` and writes the sidecar. `dir` must be an
        /// absolute path without `..`; files are only ever created with the fixed names.
        pub fn create(dir: &Path, seed: u64, reserve: u64) -> Result<TestFiles, DiskError> {
            if !dir.is_absolute() || dir.components().any(|c| matches!(c, Component::ParentDir)) {
                return Err(DiskError::Io(ERROR_INVALID_NAME));
            }
            if !fits(free_bytes(dir)?, 0, reserve) {
                return Err(DiskError::Full);
            }
            let prefix = file_prefix(seed);
            let side = create_new(
                &dir.join(format!("{prefix}.oma-test.json")),
                GENERIC_WRITE.0 | DELETE.0,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                FILE_FLAG_DELETE_ON_CLOSE,
            )?;
            // SAFETY: plain process id query.
            let pid = unsafe { GetCurrentProcessId() };
            let json = sidecar_json(pid, started_at(), &prefix);
            let mut written = 0u32;
            // SAFETY: the handle was opened without FILE_FLAG_OVERLAPPED, so the call is
            // synchronous; the buffer and `written` outlive it.
            unsafe { WriteFile(side.0, Some(json.as_bytes()), Some(&mut written), None) }
                .map_err(|e| err(&e))?;
            Ok(TestFiles {
                dir: dir.to_path_buf(),
                prefix,
                reserve,
                _sidecar: side,
            })
        }

        /// Creates `<prefix>[-suffix].bin` with `CREATE_NEW`, `bytes` long and uncompressed.
        /// `suffix` is `None` or `-` plus ASCII letters and digits (`-3`, `-sync`).
        pub fn open_data(
            &self,
            suffix: Option<&str>,
            bytes: u64,
            write_through: bool,
        ) -> Result<DataFile, DiskError> {
            let name = data_name(&self.prefix, suffix).ok_or(DiskError::Io(ERROR_INVALID_NAME))?;
            if !fits(free_bytes(&self.dir)?, bytes, self.reserve) {
                return Err(DiskError::Full);
            }
            let eof = i64::try_from(bytes).map_err(|_| DiskError::Full)?;
            let mut flags =
                FILE_FLAG_NO_BUFFERING | FILE_FLAG_OVERLAPPED | FILE_FLAG_DELETE_ON_CLOSE;
            if write_through {
                flags |= FILE_FLAG_WRITE_THROUGH;
            }
            let owned = create_new(
                &self.dir.join(name),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                flags,
            )?;
            let mut file = DataFile {
                owned,
                bytes: 0,
                sector: 4096,
            };
            set_uncompressed(file.owned.0);
            file.sector = physical_sector(file.owned.0);
            let info = FILE_END_OF_FILE_INFO { EndOfFile: eof };
            // SAFETY: `info` is a live FILE_END_OF_FILE_INFO and its size is passed.
            unsafe {
                SetFileInformationByHandle(
                    file.owned.0,
                    FileEndOfFileInfo,
                    std::ptr::from_ref(&info).cast(),
                    std::mem::size_of::<FILE_END_OF_FILE_INFO>() as u32,
                )
            }
            .map_err(|e| err(&e))?;
            file.bytes = bytes;
            Ok(file)
        }
    }

    /// Best effort: FAT and exFAT have no compression, so any failure is ignored. The handle
    /// is overlapped, so the call waits on an event instead of passing a null OVERLAPPED.
    fn set_uncompressed(h: HANDLE) {
        // SAFETY: a manual-reset event with no name or security attributes.
        let Ok(ev) = (unsafe { CreateEventW(None, true, false, None) }) else {
            return;
        };
        let ev = Owned(ev);
        let mut ov = OVERLAPPED {
            hEvent: ev.0,
            ..Default::default()
        };
        // COMPRESSION_FORMAT_NONE.
        let format: u16 = 0;
        let mut returned = 0u32;
        // SAFETY: `format`, `returned` and `ov` live on this frame until the operation is
        // complete (we wait below before returning).
        let r = unsafe {
            DeviceIoControl(
                h,
                FSCTL_SET_COMPRESSION,
                Some(std::ptr::from_ref(&format).cast()),
                2,
                None,
                0,
                Some(&mut returned),
                Some(&mut ov),
            )
        };
        if let Err(e) = r {
            if e.code() == ERROR_IO_PENDING.to_hresult() {
                // SAFETY: same handle and OVERLAPPED as the call above; waits for completion.
                let _ = unsafe { GetOverlappedResult(h, &ov, &mut returned, true) };
            }
        }
    }

    /// `PhysicalBytesPerSectorForPerformance`; 4096 if the query fails.
    fn physical_sector(h: HANDLE) -> u32 {
        let mut info = FILE_STORAGE_INFO::default();
        // SAFETY: `info` is a live FILE_STORAGE_INFO and its size is passed.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                h,
                FileStorageInfo,
                std::ptr::from_mut(&mut info).cast(),
                std::mem::size_of::<FILE_STORAGE_INFO>() as u32,
            )
        };
        match ok {
            Ok(()) if info.PhysicalBytesPerSectorForPerformance >= 512 => {
                info.PhysicalBytesPerSectorForPerformance
            }
            _ => 4096,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn fits_never_overflows() {
            assert!(fits(10, 5, 5));
            assert!(!fits(10, 5, 6));
            assert!(!fits(5, 10, 0));
            assert!(!fits(1 << 40, 0, u64::MAX / 2));
            assert!(!fits(u64::MAX, u64::MAX, 1));
        }

        fn temp_dir(tag: &str) -> PathBuf {
            let d = std::env::temp_dir().join(format!("oma-c3-{tag}-{}", std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            d
        }

        #[test]
        #[ignore = "requires real Windows hardware"]
        fn test_file_is_deleted_on_close() {
            let dir = temp_dir("del");
            {
                let tf = TestFiles::create(&dir, 0x1234, 1 << 20).unwrap();
                let df = tf.open_data(None, 64 << 20, false).unwrap();
                assert_eq!(df.bytes, 64 << 20);
                assert!(df.sector >= 512);
                df.flush().unwrap();
                let side =
                    std::fs::read_to_string(dir.join("oma-test-0000000000001234.oma-test.json"))
                        .unwrap();
                assert!(
                    side.contains(r#""prefix":"oma-test-0000000000001234""#),
                    "{side}"
                );
                assert!(dir.join("oma-test-0000000000001234.bin").exists());
            }
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
            std::fs::remove_dir(&dir).unwrap();
        }

        #[test]
        #[ignore = "requires real Windows hardware"]
        fn create_new_refuses_an_existing_name() {
            let dir = temp_dir("dup");
            let tf = TestFiles::create(&dir, 0x77, 1 << 20).unwrap();
            assert!(TestFiles::create(&dir, 0x77, 1 << 20).is_err());
            let a = tf.open_data(Some("-sync"), 1 << 20, true).unwrap();
            assert!(tf.open_data(Some("-sync"), 1 << 20, true).is_err());
            assert!(tf.open_data(Some("../x"), 1 << 20, false).is_err());
            assert!(tf.open_data(Some("sync"), 1 << 20, false).is_err());
            let b = tf.open_data(Some("-3"), 1 << 20, false).unwrap();
            assert!(dir.join("oma-test-0000000000000077-3.bin").exists());
            drop(b);
            drop(a);
            drop(tf);
            std::fs::remove_dir(&dir).unwrap();
        }

        #[test]
        #[ignore = "requires real Windows hardware"]
        fn reserve_larger_than_the_free_space_is_full() {
            let dir = temp_dir("full");
            assert_eq!(
                TestFiles::create(&dir, 1, u64::MAX / 2).err(),
                Some(DiskError::Full)
            );
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
            std::fs::remove_dir(&dir).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_json_has_the_four_fields() {
        let s = sidecar_json(42, 133_000_000_000_000_000, &file_prefix(0xAB));
        assert_eq!(
            s,
            r#"{"format":1,"pid":42,"startedAt":133000000000000000,"prefix":"oma-test-00000000000000ab"}"#
        );
    }

    #[test]
    fn data_name_takes_a_dashed_suffix() {
        let p = file_prefix(1);
        assert_eq!(data_name(&p, None).unwrap(), format!("{p}.bin"));
        assert_eq!(data_name(&p, Some("-3")).unwrap(), format!("{p}-3.bin"));
        assert_eq!(
            data_name(&p, Some("-sync")).unwrap(),
            format!("{p}-sync.bin")
        );
        for bad in ["3", "-", "", "-a/b", "-..", "--3", "-a.b", r"-a\b"] {
            assert!(data_name(&p, Some(bad)).is_none(), "{bad}");
        }
    }

    #[test]
    fn win32_codes_classify() {
        assert_eq!(classify_win32(112), DiskError::Full);
        assert_eq!(classify_win32(39), DiskError::Full);
        assert_eq!(classify_win32(5), DiskError::AccessDenied);
        assert_eq!(classify_win32(87), DiskError::Io(87));
    }
}
