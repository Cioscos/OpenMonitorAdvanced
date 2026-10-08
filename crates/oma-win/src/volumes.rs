//! Volume list and target-folder probe for the disk benchmark and stress test (M8c). Metadata
//! only: nothing here reads data from a disk, so a sleeping HDD stays asleep.

use std::fs::OpenOptions;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_FLAG_DELETE_ON_CLOSE,
};

use crate::storage_ioctl::PhysicalDrive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Fixed,
    Removable,
    Remote,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    /// `X:\`.
    pub root: String,
    pub label: String,
    pub fs: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub drive: DriveKind,
    /// The physical disk; `None` for network or multi-disk volumes.
    pub disk_index: Option<u32>,
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderProbe {
    /// `X:\` of a local path with a drive letter.
    pub volume_root: Option<String>,
    pub remote: bool,
    pub exists: bool,
    pub writable: bool,
    pub sync: bool,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

const DRIVE_UNKNOWN: u32 = 0;
const DRIVE_NO_ROOT_DIR: u32 = 1;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;

/// UNC (`\\server\x`, `\\?\UNC\server\x`); `\\?\C:\x` and `\\.\` are local.
pub fn is_remote_path(path: &str) -> bool {
    let p = path.replace('/', "\\");
    if let Some(rest) = p.strip_prefix(r"\\?\").or_else(|| p.strip_prefix(r"\\.\")) {
        return rest
            .get(..4)
            .is_some_and(|s| s.eq_ignore_ascii_case("UNC\\"));
    }
    p.starts_with(r"\\")
}

/// A path component named like a sync client folder (case-insensitive).
pub fn is_sync_path(path: &str) -> bool {
    path.split(['\\', '/']).any(|c| {
        let c = c.to_lowercase();
        c == "onedrive" || c == "dropbox" || c == "google drive"
    })
}

fn drive_kind(raw: u32) -> DriveKind {
    match raw {
        DRIVE_FIXED => DriveKind::Fixed,
        DRIVE_REMOVABLE => DriveKind::Removable,
        DRIVE_REMOTE => DriveKind::Remote,
        _ => DriveKind::Other,
    }
}

/// `(free to the caller, total)` bytes of the volume holding `path`; zeros on failure.
fn free_total(path: &str) -> (u64, u64) {
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: valid NUL-terminated path; both out-pointers are live `u64`s; the third
    // (total free) is optional.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            &HSTRING::from(path),
            Some(&mut free),
            Some(&mut total),
            None,
        )
    };
    if ok.is_ok() {
        (free, total)
    } else {
        (0, 0)
    }
}

fn utf16_text(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// `(label, file system)`; `None` when the volume has no media or cannot be read.
fn volume_texts(root: &str) -> Option<(String, String)> {
    let mut label = [0u16; 261];
    let mut fs = [0u16; 261];
    // SAFETY: valid NUL-terminated root; the buffers are live slices whose lengths the
    // binding passes; the serial, max-component and flags out-pointers are absent.
    unsafe {
        GetVolumeInformationW(
            &HSTRING::from(root),
            Some(&mut label),
            None,
            None,
            None,
            Some(&mut fs),
        )
    }
    .ok()?;
    Some((utf16_text(&label), utf16_text(&fs)))
}

/// The mounted drive letters worth testing: no optical drives, no empty readers.
pub fn volumes() -> Vec<VolumeInfo> {
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let system_root = format!("{}\\", system.trim_end_matches('\\')).to_lowercase();
    // SAFETY: no arguments.
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for bit in 0..26u8 {
        if mask & (1 << bit) == 0 {
            continue;
        }
        let letter = (b'A' + bit) as char;
        let root = format!("{letter}:\\");
        // SAFETY: valid NUL-terminated root path.
        let raw = unsafe { GetDriveTypeW(&HSTRING::from(root.as_str())) };
        if matches!(raw, DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR | DRIVE_CDROM) {
            continue;
        }
        let Some((label, fs)) = volume_texts(&root) else {
            continue; // no media
        };
        let drive = drive_kind(raw);
        let disk_index = (drive != DriveKind::Remote)
            .then(|| PhysicalDrive::open_path(&format!(r"\\.\{letter}:"))?.disk_number())
            .flatten();
        let (free_bytes, total_bytes) = free_total(&root);
        out.push(VolumeInfo {
            system: root.to_lowercase() == system_root,
            root,
            label,
            fs,
            total_bytes,
            free_bytes,
            drive,
            disk_index,
        });
    }
    out
}

fn drive_letter_root(path: &str) -> Option<String> {
    let b = path.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        .then(|| format!("{}:\\", (b[0] as char).to_ascii_uppercase()))
}

/// Creates (and, by delete-on-close, removes) a probe file in `dir`.
fn can_create_file(dir: &Path) -> bool {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!(
        "oma-probe-{:x}{:x}{n:x}.tmp",
        std::process::id(),
        nanos as u64
    );
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .access_mode(0x4000_0000 | 0x0001_0000) // GENERIC_WRITE | DELETE
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE.0)
        .open(dir.join(name))
        .is_ok()
}

/// Looks at a candidate folder without touching its data (DC6).
pub fn probe_folder(path: &Path) -> FolderProbe {
    let text = path.to_string_lossy();
    let volume_root = drive_letter_root(&text);
    let remote = is_remote_path(&text)
        || volume_root.as_deref().is_some_and(|r| {
            // SAFETY: valid NUL-terminated root path.
            unsafe { GetDriveTypeW(&HSTRING::from(r)) == DRIVE_REMOTE }
        });
    let mut probe = FolderProbe {
        volume_root,
        remote,
        exists: false,
        writable: false,
        sync: is_sync_path(&text),
        free_bytes: 0,
        total_bytes: 0,
    };
    if remote {
        return probe;
    }
    probe.exists = path.is_dir();
    if !probe.exists {
        return probe;
    }
    probe.writable = can_create_file(path);
    probe.sync = probe.sync
        || path.ancestors().any(|a| {
            std::fs::symlink_metadata(a)
                .is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0 != 0)
        });
    (probe.free_bytes, probe.total_bytes) = free_total(&text);
    probe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unc_and_remote_paths_are_remote() {
        assert!(is_remote_path(r"\\server\x"));
        assert!(is_remote_path(r"\\?\UNC\server\x"));
        assert!(!is_remote_path(r"C:\x"));
        assert!(!is_remote_path(r"\\?\C:\x"));
    }

    #[test]
    fn sync_paths_are_detected() {
        assert!(is_sync_path(r"C:\Users\a\OneDrive\x"));
        assert!(is_sync_path(r"D:\Dropbox"));
        assert!(is_sync_path(r"C:\Users\a\Google Drive\y"));
        assert!(!is_sync_path(r"C:\Users\a\drive\x"));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn system_volume_is_listed() {
        let root = format!("{}\\", std::env::var("SystemDrive").unwrap());
        let v = volumes();
        let sys = v
            .iter()
            .find(|v| v.root.eq_ignore_ascii_case(&root))
            .unwrap();
        assert!(sys.system && sys.disk_index.is_some());
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn temp_folder_probes_writable() {
        let dir = std::env::temp_dir();
        let p = probe_folder(&dir);
        assert!(p.exists && p.writable && !p.remote && p.free_bytes > 0);
        let left = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("oma-probe-"));
        assert!(!left);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn missing_folder_probes_not_found() {
        let p = probe_folder(&std::env::temp_dir().join("oma-no-such-dir-xyz"));
        assert!(!p.exists && !p.writable && !p.remote);
    }
}
