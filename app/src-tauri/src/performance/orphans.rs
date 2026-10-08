//! Test files a disk run left behind (DC11): `oma-load` opens them delete-on-close, so
//! they stay only after a system crash or a power cut. A file goes only when its sidecar
//! `oma-test-<16 hex>.oma-test.json` is valid (at most 4 KiB, `format` 1, `prefix` equal to
//! its name) and names a process that is gone, or a process with another start time.
//! Then only `^<prefix>(-\d{1,6}|-sync)?\.bin$` and the sidecar go: no other name, no
//! subfolder, and a reparse point is never followed nor deleted.

use std::fs;
use std::io::Read;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const NAME_START: &str = "oma-test-";
const SIDECAR_SUFFIX: &str = ".oma-test.json";
const SIDECAR_MAX: u64 = 4096;
const SIDECAR_FORMAT: u32 = 1;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// A plain file of the folder (never a folder, a link or a reparse point).
#[derive(Debug, Clone)]
pub struct DirFile {
    pub name: String,
    pub len: u64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sidecar {
    format: u32,
    pid: u32,
    started_at: u64,
    prefix: String,
}

/// `oma-test-<16 lowercase hex>`, the prefix of a sidecar name.
fn sidecar_prefix(name: &str) -> Option<&str> {
    let prefix = name.strip_suffix(SIDECAR_SUFFIX)?;
    let hex = prefix.strip_prefix(NAME_START)?;
    (hex.len() == 16 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then_some(prefix)
}

/// `^<prefix>(-\d{1,6}|-sync)?\.bin$`.
fn is_data_file(name: &str, prefix: &str) -> bool {
    let Some(rest) = name
        .strip_prefix(prefix)
        .and_then(|r| r.strip_suffix(".bin"))
    else {
        return false;
    };
    match rest.strip_prefix('-') {
        None => rest.is_empty(),
        Some("sync") => true,
        Some(n) => (1..=6).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()),
    }
}

/// Whether the run of a sidecar is over. `startedAt` 0 means `oma-load` could not read its
/// own start time: then only a pid that is gone counts (R12).
fn is_orphan(sidecar: &Sidecar, started_at: &impl Fn(u32) -> Option<u64>) -> bool {
    let live = started_at(sidecar.pid);
    if sidecar.started_at == 0 {
        live.is_none()
    } else {
        live != Some(sidecar.started_at)
    }
}

/// The files of `dir` to delete, data files before their sidecar. `read_sidecar` reads a
/// file by name (capped); `started_at` is `process_started_at`.
pub fn orphans(
    dir: &Path,
    files: &[DirFile],
    read_sidecar: impl Fn(&str) -> Option<Vec<u8>>,
    started_at: impl Fn(u32) -> Option<u64>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for f in files {
        let Some(prefix) = sidecar_prefix(&f.name) else {
            continue;
        };
        if f.len > SIDECAR_MAX {
            continue;
        }
        let Some(bytes) = read_sidecar(&f.name) else {
            continue;
        };
        let Ok(sidecar) = serde_json::from_slice::<Sidecar>(&bytes) else {
            continue;
        };
        if sidecar.format != SIDECAR_FORMAT
            || sidecar.prefix != prefix
            || !is_orphan(&sidecar, &started_at)
        {
            continue;
        }
        out.extend(
            files
                .iter()
                .filter(|d| is_data_file(&d.name, prefix))
                .map(|d| dir.join(&d.name)),
        );
        out.push(dir.join(&f.name));
    }
    out
}

/// The plain files of `dir` named `oma-test-*`: folders, links and reparse points are left out.
fn list(dir: &Path) -> Vec<DirFile> {
    let Ok(entries) = fs::read_dir(dir) else {
        return vec![];
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !name.starts_with(NAME_START) {
                return None;
            }
            // On Windows the entry's metadata is the directory listing's: links are not followed.
            let m = e.metadata().ok()?;
            (m.is_file() && m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0)
                .then_some(DirFile { name, len: m.len() })
        })
        .collect()
}

/// At most [`SIDECAR_MAX`] bytes, never through a reparse point.
fn read_capped(path: &Path) -> Option<Vec<u8>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let mut bytes = Vec::new();
    file.take(SIDECAR_MAX + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= SIDECAR_MAX).then_some(bytes)
}

/// Deletes the orphaned test files of `dir`, logging each one; how many went.
pub fn sweep(dir: &Path, started_at: impl Fn(u32) -> Option<u64>) -> usize {
    let files = list(dir);
    if files.is_empty() {
        return 0;
    }
    let mut deleted = 0;
    for path in orphans(dir, &files, |name| read_capped(&dir.join(name)), started_at) {
        match fs::remove_file(&path) {
            Ok(()) => {
                deleted += 1;
                tracing::info!(path = %path.display(), "deleted an orphaned disk test file");
            }
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "cannot delete an orphaned disk test file");
            }
        }
    }
    deleted
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "oma-test-0123456789abcdef";
    const PID: u32 = 4242;
    const STARTED: u64 = 133_000_000_000_000_000;

    fn sidecar_json(prefix: &str, pid: u32, started: u64) -> Vec<u8> {
        format!(r#"{{"format":1,"pid":{pid},"startedAt":{started},"prefix":"{prefix}"}}"#)
            .into_bytes()
    }

    fn file(name: &str) -> DirFile {
        DirFile {
            name: name.into(),
            len: 10,
        }
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    fn gone(_: u32) -> Option<u64> {
        None
    }

    #[test]
    fn orphans_ignore_foreign_names() {
        let sidecar = format!("{P}.oma-test.json");
        let files: Vec<DirFile> = [
            sidecar.as_str(),
            &format!("{P}.bin"),
            &format!("{P}-12.bin"),
            &format!("{P}-sync.bin"),
            // Not ours: other suffixes, too many digits, other spellings.
            &format!("{P}.bin.txt"),
            &format!("{P}-1234567.bin"),
            &format!("{P}-.bin"),
            &format!("{P}-syncx.bin"),
            &format!("{P}x.bin"),
            "oma-test-0123456789ABCDEF.oma-test.json",
            "oma-test-0123456789ABCDEF.bin",
            "oma-test-123.oma-test.json",
            "oma-test-123.bin",
            "photo.jpg",
            "OMA-TEST-0123456789abcdef.bin",
        ]
        .iter()
        .map(|n| file(n))
        .collect();
        let read = |name: &str| {
            let prefix = name.strip_suffix(SIDECAR_SUFFIX)?;
            Some(sidecar_json(prefix, PID, STARTED))
        };
        let out = orphans(Path::new(r"D:\t"), &files, read, gone);
        assert_eq!(
            names(&out),
            [
                format!("{P}.bin"),
                format!("{P}-12.bin"),
                format!("{P}-sync.bin"),
                sidecar,
            ]
        );
        assert!(out.iter().all(|p| p.parent() == Some(Path::new(r"D:\t"))));
    }

    #[test]
    fn orphans_keep_a_live_process() {
        let files = [
            file(&format!("{P}.oma-test.json")),
            file(&format!("{P}.bin")),
        ];
        let run = |started: u64, live: Option<u64>| {
            let read = |_: &str| Some(sidecar_json(P, PID, started));
            orphans(Path::new(r"D:\"), &files, read, |pid| {
                assert_eq!(pid, PID);
                live
            })
            .len()
        };
        // The same process, still running: nothing goes.
        assert_eq!(run(STARTED, Some(STARTED)), 0);
        // The pid is another process now, or nobody's.
        assert_eq!(run(STARTED, Some(STARTED + 1)), 2);
        assert_eq!(run(STARTED, None), 2);
        // `oma-load` could not read its start time: only a pid that is gone counts (R12).
        assert_eq!(run(0, Some(STARTED)), 0);
        assert_eq!(run(0, None), 2);
    }

    #[test]
    fn orphans_skip_a_bad_sidecar() {
        let sidecar = format!("{P}.oma-test.json");
        let data = file(&format!("{P}.bin"));
        let check = |len: u64, bytes: Option<Vec<u8>>| {
            let files = [
                DirFile {
                    name: sidecar.clone(),
                    len,
                },
                data.clone(),
            ];
            orphans(Path::new(r"D:\"), &files, |_| bytes.clone(), gone).len()
        };
        assert_eq!(
            check(10, Some(sidecar_json(P, PID, STARTED))),
            2,
            "a good one"
        );
        // Too large, unreadable, not JSON, another format, another prefix.
        assert_eq!(check(4097, Some(sidecar_json(P, PID, STARTED))), 0);
        assert_eq!(check(10, None), 0);
        assert_eq!(check(10, Some(b"{not json".to_vec())), 0);
        let future = format!(r#"{{"format":2,"pid":1,"startedAt":5,"prefix":"{P}"}}"#);
        assert_eq!(check(10, Some(future.into_bytes())), 0);
        let other = sidecar_json("oma-test-ffffffffffffffff", PID, STARTED);
        assert_eq!(check(10, Some(other)), 0);
        assert_eq!(check(10, Some(br#"{"format":1}"#.to_vec())), 0);
    }

    /// A unique folder under the temporary folder, removed at the end.
    struct Temp(PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn orphans_delete_all_v3_parts() {
        let dir = std::env::temp_dir().join(format!(
            "oma-orphans-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let temp = Temp(dir.clone());
        let ours = [
            format!("{P}.bin"),
            format!("{P}-1.bin"),
            format!("{P}-2.bin"),
            format!("{P}-123456.bin"),
            format!("{P}-sync.bin"),
        ];
        for n in &ours {
            fs::write(dir.join(n), b"x").unwrap();
        }
        fs::write(
            dir.join(format!("{P}.oma-test.json")),
            sidecar_json(P, PID, STARTED),
        )
        .unwrap();
        // The user's file and a folder named like a part stay.
        fs::write(dir.join(format!("{P}.txt")), b"mine").unwrap();
        fs::create_dir(dir.join(format!("{P}-3.bin"))).unwrap();
        // A live run of another test in the same folder stays too.
        let live = "oma-test-fedcba9876543210";
        fs::write(dir.join(format!("{live}.bin")), b"x").unwrap();
        fs::write(
            dir.join(format!("{live}.oma-test.json")),
            sidecar_json(live, 7, 99),
        )
        .unwrap();
        let deleted = sweep(&dir, |pid| (pid == 7).then_some(99));
        assert_eq!(deleted, ours.len() + 1);
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                format!("{P}-3.bin"),
                format!("{P}.txt"),
                format!("{live}.bin"),
                format!("{live}.oma-test.json"),
            ]
        );
        drop(temp);
    }
}
