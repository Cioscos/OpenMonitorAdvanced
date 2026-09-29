//! Start with Windows through the user's `HKCU\...\Run` key (spec M5 §2.6).
//!
//! The app owns one `Run` value. Whether Windows actually starts it is decided
//! by `StartupApproved\Run` (Task Manager and Settings write there): its binary
//! format is undocumented, so it is only ever read, never written.

use std::io;
use std::path::Path;

use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE,
    REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
};

pub const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const APPROVED_SUBKEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
pub const VALUE_NAME: &str = "OpenMonitor Advanced";

/// The command Windows runs at sign-in: the quoted path, then `--minimized`.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" --minimized", exe.display())
}

/// Whether Windows will start the entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Effective {
    /// The app has no `Run` value.
    NotConfigured,
    Enabled,
    /// The user switched the entry off in Task Manager or Settings.
    DisabledByWindows,
    /// A `StartupApproved` value in a format this build does not recognise.
    Unknown,
}

/// Reads a `StartupApproved\Run` value for an existing `Run` entry. An absent
/// value means nobody has toggled the entry, so Windows runs it; the first byte
/// of a present one is the state (`02`/`06` on, `03`/`07` off).
pub fn approved_state(bytes: Option<&[u8]>) -> Effective {
    match bytes {
        None => Effective::Enabled,
        Some(bytes) => match bytes.first() {
            Some(0x02 | 0x06) => Effective::Enabled,
            Some(0x03 | 0x07) => Effective::DisabledByWindows,
            _ => Effective::Unknown,
        },
    }
}

/// Where the entry lives; a plain value so tests can point it at a scratch key.
#[derive(Clone, Debug)]
pub struct RunKey {
    pub run_subkey: String,
    pub approved_subkey: String,
    pub value_name: String,
}

impl RunKey {
    pub fn production() -> Self {
        Self {
            run_subkey: RUN_SUBKEY.to_owned(),
            approved_subkey: APPROVED_SUBKEY.to_owned(),
            value_name: VALUE_NAME.to_owned(),
        }
    }

    /// The command stored in the `Run` value, `None` when there is none.
    pub fn read(&self) -> io::Result<Option<String>> {
        Ok(get_raw(&self.run_subkey, &self.value_name)?.map(|bytes| {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .take_while(|&unit| unit != 0)
                .collect();
            String::from_utf16_lossy(&units)
        }))
    }

    /// Stores the command for `exe`, creating the key when needed.
    pub fn write(&self, exe: &Path) -> io::Result<()> {
        let text: Vec<u8> = command_line(exe)
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        set_raw(&self.run_subkey, &self.value_name, REG_SZ, &text)
    }

    /// Deletes the value; an absent value or key is `Ok`.
    pub fn remove(&self) -> io::Result<()> {
        let Some(key) = OpenKey::open(&self.run_subkey, KEY_SET_VALUE)? else {
            return Ok(());
        };
        let name = wide(&self.value_name);
        // SAFETY: `key` is an open handle and `name` is NUL-terminated.
        let code = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
        match code {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
            other => Err(os_error(other)),
        }
    }

    /// `NotConfigured` without a `Run` value, `Unknown` when a key cannot be
    /// read, otherwise what `StartupApproved` says about it (read only).
    pub fn effective(&self) -> Effective {
        match self.read() {
            Ok(None) => return Effective::NotConfigured,
            Ok(Some(_)) => {}
            Err(_) => return Effective::Unknown,
        }
        match get_raw(&self.approved_subkey, &self.value_name) {
            Ok(None) => approved_state(None),
            Ok(Some(bytes)) => approved_state(Some(&bytes)),
            Err(_) => Effective::Unknown,
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn os_error(code: WIN32_ERROR) -> io::Error {
    io::Error::from_raw_os_error(code.0 as i32)
}

/// An open `HKCU` subkey, closed on drop.
struct OpenKey(HKEY);

impl OpenKey {
    /// `None` when the subkey does not exist.
    fn open(subkey: &str, access: REG_SAM_FLAGS) -> io::Result<Option<Self>> {
        let subkey = wide(subkey);
        let mut key = HKEY::default();
        // SAFETY: `subkey` is NUL-terminated and `key` is a valid out pointer.
        let code = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                None,
                access,
                &mut key,
            )
        };
        match code {
            ERROR_SUCCESS => Ok(Some(Self(key))),
            ERROR_FILE_NOT_FOUND => Ok(None),
            other => Err(os_error(other)),
        }
    }

    /// Opens the subkey, creating it (and its parents) when needed.
    fn create(subkey: &str, access: REG_SAM_FLAGS) -> io::Result<Self> {
        let subkey = wide(subkey);
        let mut key = HKEY::default();
        // SAFETY: `subkey` is NUL-terminated, the optional pointers are absent
        // and `key` is a valid out pointer.
        let code = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                access,
                None,
                &mut key,
                None,
            )
        };
        if code == ERROR_SUCCESS {
            Ok(Self(key))
        } else {
            Err(os_error(code))
        }
    }
}

impl Drop for OpenKey {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `open` or `create` and is closed once.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// The bytes of a value; `None` when the key or the value is absent.
fn get_raw(subkey: &str, name: &str) -> io::Result<Option<Vec<u8>>> {
    let Some(key) = OpenKey::open(subkey, KEY_QUERY_VALUE)? else {
        return Ok(None);
    };
    let name = wide(name);
    // The value can grow between calls: retry on MORE_DATA with the size it reports.
    let mut buffer = vec![0u8; 512];
    for _ in 0..4 {
        let mut size = buffer.len() as u32;
        // SAFETY: `key` is open, `name` is NUL-terminated and `buffer` provides
        // `size` writable bytes; the out pointers are valid for the call.
        let code = unsafe {
            RegQueryValueExW(
                key.0,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(buffer.as_mut_ptr()),
                Some(&mut size),
            )
        };
        match code {
            ERROR_SUCCESS => {
                buffer.truncate(size as usize);
                return Ok(Some(buffer));
            }
            ERROR_FILE_NOT_FOUND => return Ok(None),
            ERROR_MORE_DATA => buffer.resize(size as usize + 2, 0),
            other => return Err(os_error(other)),
        }
    }
    Err(io::Error::other("the registry value keeps changing"))
}

/// Writes a value, creating the subkey when needed.
fn set_raw(subkey: &str, name: &str, kind: REG_VALUE_TYPE, bytes: &[u8]) -> io::Result<()> {
    let key = OpenKey::create(subkey, KEY_SET_VALUE)?;
    let name = wide(name);
    // SAFETY: `key` is open and `name` is NUL-terminated; `bytes` outlives the call.
    let code = unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, kind, Some(bytes)) };
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(os_error(code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use windows::Win32::System::Registry::{RegDeleteKeyW, RegDeleteTreeW, REG_BINARY};

    const TESTS_PARENT: &str = r"Software\OpenMonitorAdvanced\Tests";

    /// A scratch pair of keys under HKCU, deleted on drop.
    struct Scratch {
        key: RunKey,
        root: String,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = format!(r"{TESTS_PARENT}\Run-{}-{name}", std::process::id());
            Self {
                key: RunKey {
                    run_subkey: format!(r"{root}\Run"),
                    approved_subkey: format!(r"{root}\Approved"),
                    value_name: VALUE_NAME.to_owned(),
                },
                root,
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let root = wide(&self.root);
            // SAFETY: `root` and `parent` are NUL-terminated and outlive the calls.
            unsafe {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(root.as_ptr()));
                // Fails, harmlessly, while another test's key is still below.
                let parent = wide(TESTS_PARENT);
                let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(parent.as_ptr()));
            }
        }
    }

    #[test]
    fn command_line_quotes_the_path() {
        let exe = Path::new(r"C:\Program Files\OpenMonitor Advanced\oma-app.exe");
        assert_eq!(
            command_line(exe),
            r#""C:\Program Files\OpenMonitor Advanced\oma-app.exe" --minimized"#
        );
    }

    #[test]
    fn approved_state_reads_known_prefixes() {
        let state = |bytes: &[u8]| approved_state(Some(bytes));
        assert_eq!(approved_state(None), Effective::Enabled);
        assert_eq!(state(&[0x02, 0, 0, 0]), Effective::Enabled);
        assert_eq!(state(&[0x06, 0, 0, 0]), Effective::Enabled);
        assert_eq!(
            state(&[0x03, 0, 0, 0, 0xC8, 0xBD]),
            Effective::DisabledByWindows
        );
        assert_eq!(state(&[0x07]), Effective::DisabledByWindows);
        assert_eq!(state(&[]), Effective::Unknown);
        assert_eq!(state(&[0x00, 0x01]), Effective::Unknown);
        assert_eq!(state(&[0x09]), Effective::Unknown);
    }

    #[test]
    fn effective_serializes_in_camel_case() {
        let json = |e: Effective| serde_json::to_string(&e).unwrap();
        assert_eq!(json(Effective::NotConfigured), r#""notConfigured""#);
        assert_eq!(json(Effective::DisabledByWindows), r#""disabledByWindows""#);
    }

    #[test]
    fn run_value_round_trips() {
        let scratch = Scratch::new("roundtrip");
        let key = &scratch.key;
        let exe = Path::new(r"C:\Tools\oma-app.exe");

        assert_eq!(key.read().unwrap(), None, "no key yet");
        key.remove().expect("removing from a missing key is Ok");

        key.write(exe).unwrap();
        assert_eq!(
            key.read().unwrap().as_deref(),
            Some(command_line(exe).as_str())
        );

        key.write(Path::new(r"D:\Other\oma-app.exe")).unwrap();
        assert_eq!(
            key.read().unwrap().as_deref(),
            Some(r#""D:\Other\oma-app.exe" --minimized"#),
            "a second write replaces the value"
        );

        key.remove().unwrap();
        assert_eq!(key.read().unwrap(), None);
        key.remove().expect("removing an absent value is Ok");
    }

    #[test]
    fn effective_follows_the_run_entry_and_startup_approved() {
        let scratch = Scratch::new("effective");
        let key = &scratch.key;
        assert_eq!(key.effective(), Effective::NotConfigured);

        key.write(Path::new(r"C:\Tools\oma-app.exe")).unwrap();
        assert_eq!(key.effective(), Effective::Enabled, "no approved value");

        set_approved(key, &[0x03, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(key.effective(), Effective::DisabledByWindows);
        set_approved(key, &[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(key.effective(), Effective::Enabled);
        set_approved(key, &[]);
        assert_eq!(key.effective(), Effective::Unknown);

        key.remove().unwrap();
        assert_eq!(key.effective(), Effective::NotConfigured);
    }

    /// Stands in for Task Manager: writes an approved value into the scratch key.
    fn set_approved(key: &RunKey, bytes: &[u8]) {
        set_raw(&key.approved_subkey, &key.value_name, REG_BINARY, bytes).unwrap();
    }
}
