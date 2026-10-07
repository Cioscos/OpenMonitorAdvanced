//! Descendants of a process from a Toolhelp snapshot (used to leave our own processes out of
//! the "other programs on the GPU" warning).

use std::collections::HashSet;
use std::io;
use std::mem::size_of;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

/// `root` and every descendant, from `(pid, parent pid)` pairs. Always contains `root`; the
/// visited set makes a reused pid that is the parent of its own ancestor harmless.
pub fn descendants_of(pairs: &[(u32, u32)], root: u32) -> HashSet<u32> {
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
struct Snapshot(windows::Win32::Foundation::HANDLE);

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: the handle is open and not used afterwards.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// `root` and its descendants among the running processes (one Toolhelp snapshot).
pub fn descendants(root: u32) -> io::Result<HashSet<u32>> {
    // SAFETY: plain flags; the handle is owned (and closed) by the guard.
    let snapshot = Snapshot(
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|e| io::Error::other(e.to_string()))?,
    );
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut pairs = Vec::new();
    // SAFETY: the snapshot handle is open and `entry` is writable with `dwSize` set.
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) }.is_ok();
    while more {
        pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot.0, &mut entry) }.is_ok();
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
