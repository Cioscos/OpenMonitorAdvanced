//! Descendants of a process from a Toolhelp snapshot (used to leave our own processes out of
//! the "other programs on the GPU" warning).

use std::collections::HashSet;
use std::io;
use std::mem::size_of;

use windows::core::HRESULT;
use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

/// `root` and every descendant, from `(pid, parent pid)` pairs. Always contains `root`; the
/// visited set makes a reused pid that is the parent of its own ancestor harmless.
pub fn descendants_of(pairs: &[(u32, u32)], root: u32) -> HashSet<u32> {
    // ponytail: a reused pid whose parent exited can be wrongly included; comparing creation
    // times would fix it, acceptable for a warning filter.
    let mut found = HashSet::from([root]);
    // Fixed point: repeat until a pass adds nothing (pairs are in no particular order).
    loop {
        let before = found.len();
        for &(pid, parent) in pairs {
            if found.contains(&parent) {
                found.insert(pid);
            }
        }
        if found.len() == before {
            return found;
        }
    }
}

/// Closes the snapshot handle on drop.
struct Snapshot(HANDLE);

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: the guard is the sole owner of a handle returned by
        // `CreateToolhelp32Snapshot` (an error return never builds the guard), and it is not
        // used after this point.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// True when `code` is the normal end of a Toolhelp enumeration.
fn is_end_of_list(code: HRESULT) -> bool {
    code == ERROR_NO_MORE_FILES.to_hresult()
}

fn to_io(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
}

/// `root` and its descendants among the running processes (one Toolhelp snapshot).
pub fn descendants(root: u32) -> io::Result<HashSet<u32>> {
    // SAFETY: plain flags; the handle is owned (and closed) by the guard.
    let snapshot =
        Snapshot(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.map_err(to_io)?);
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut pairs = Vec::new();
    // SAFETY: `snapshot.0` is a live TH32CS_SNAPPROCESS snapshot; `entry` is a live `&mut`
    // with `dwSize` = size_of::<PROCESSENTRY32W>().
    let mut next = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    loop {
        match next {
            Ok(()) => pairs.push((entry.th32ProcessID, entry.th32ParentProcessID)),
            Err(e) if is_end_of_list(e.code()) => break,
            Err(e) => return Err(to_io(e)),
        }
        // SAFETY: same invariants as above: live snapshot handle, live `&mut entry` with
        // `dwSize` still set.
        next = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    Ok(descendants_of(&pairs, root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descendants_include_grandchildren() {
        let pairs = [(10, 1), (11, 10), (12, 11)];
        assert_eq!(descendants_of(&pairs, 10), HashSet::from([10, 11, 12]));
    }

    #[test]
    fn descendants_survive_a_parent_cycle() {
        // 10 -> 11 -> 12, and a reused pid makes 10 look like a child of 12.
        let pairs = [(10, 12), (11, 10), (12, 11)];
        assert_eq!(descendants_of(&pairs, 10), HashSet::from([10, 11, 12]));
        // A cycle not reachable from the root stays out.
        let apart = [(12, 11), (11, 12)];
        assert_eq!(descendants_of(&apart, 10), HashSet::from([10]));
    }

    #[test]
    fn only_no_more_files_ends_the_list() {
        assert!(is_end_of_list(ERROR_NO_MORE_FILES.to_hresult()));
        assert!(!is_end_of_list(windows::Win32::Foundation::E_FAIL));
    }

    #[test]
    fn unrelated_processes_are_left_out() {
        let pairs = [(11, 10), (20, 1), (21, 20)];
        assert_eq!(descendants_of(&pairs, 10), HashSet::from([10, 11]));
        assert_eq!(descendants_of(&pairs, 99), HashSet::from([99]));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn this_process_is_its_own_root() {
        let me = std::process::id();
        assert!(descendants(me).unwrap().contains(&me));
    }
}
