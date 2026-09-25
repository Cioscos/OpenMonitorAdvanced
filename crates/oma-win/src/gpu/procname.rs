//! pid -> executable name for the per-process GPU table (decision D6).
//!
//! A Toolhelp process snapshot needs no process handle, so it also names the processes a
//! normal user cannot open (dwm.exe, csrss.exe, services). It costs ~2.4 ms, so it is taken
//! at most once per tick and only when a pid without a cached name shows up.

use std::collections::{BTreeSet, HashMap};
use std::mem::size_of;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

const _: () = assert!(size_of::<PROCESSENTRY32W>() == 568);

/// Lists (pid, executable name) of every running process.
type Snapshot = Box<dyn FnMut() -> Vec<(u32, String)> + Send>;

/// Names that never need a snapshot.
fn fixed_name(pid: u32) -> Option<&'static str> {
    match pid {
        0 => Some("Idle"),
        4 => Some("System"),
        _ => None,
    }
}

fn fallback_name(pid: u32) -> String {
    format!("PID {pid}")
}

/// Cache of process names for the pids seen in the GPU counters.
pub(crate) struct ProcessNames {
    names: HashMap<u32, String>,
    snapshot: Snapshot,
}

impl Default for ProcessNames {
    fn default() -> Self {
        Self::with_snapshot(Box::new(toolhelp_processes))
    }
}

impl ProcessNames {
    pub(crate) fn with_snapshot(snapshot: Snapshot) -> Self {
        Self {
            names: HashMap::new(),
            snapshot,
        }
    }

    /// Keeps names for exactly `pids` (the pids in this tick's counters): forgets the others
    /// and resolves the new ones with at most one snapshot. A pid missing from the snapshot
    /// (the process just ended) is cached as "PID <n>", so it never costs a second snapshot.
    pub(crate) fn update(&mut self, pids: &BTreeSet<u32>) {
        self.names.retain(|pid, _| pids.contains(pid));
        let missing: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|&pid| fixed_name(pid).is_none() && !self.names.contains_key(&pid))
            .collect();
        if missing.is_empty() {
            return;
        }
        let listed: HashMap<u32, String> = (self.snapshot)().into_iter().collect();
        for pid in missing {
            let name = listed
                .get(&pid)
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| fallback_name(pid));
            self.names.insert(pid, name);
        }
    }

    pub(crate) fn name(&self, pid: u32) -> String {
        fixed_name(pid)
            .map(str::to_owned)
            .or_else(|| self.names.get(&pid).cloned())
            .unwrap_or_else(|| fallback_name(pid))
    }
}

/// Every running process from one Toolhelp snapshot; empty if the snapshot fails.
fn toolhelp_processes() -> Vec<(u32, String)> {
    // SAFETY: plain flags; the returned handle is closed below.
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(handle) => handle,
        Err(e) => {
            tracing::debug!(error = %e, "process snapshot failed");
            return Vec::new();
        }
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut processes = Vec::new();
    // SAFETY: the snapshot handle is open and `entry` is writable with `dwSize` set.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        processes.push((
            entry.th32ProcessID,
            crate::network::wide_to_string(&entry.szExeFile),
        ));
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: the handle is open and not used afterwards.
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    processes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Names from a fixed process list; the counter counts snapshots.
    fn names(list: &[(u32, &str)]) -> (ProcessNames, Arc<AtomicUsize>) {
        let list: Vec<(u32, String)> = list.iter().map(|&(p, n)| (p, n.to_owned())).collect();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let names = ProcessNames::with_snapshot(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            list.clone()
        }));
        (names, calls)
    }

    #[test]
    fn new_pids_are_resolved_with_one_snapshot() {
        let (mut names, calls) = names(&[(2096, "dwm.exe"), (1712, "csrss.exe"), (9, "")]);
        names.update(&BTreeSet::from([2096, 1712]));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(names.name(2096), "dwm.exe");
        assert_eq!(names.name(1712), "csrss.exe");

        // Known pids only: no snapshot.
        names.update(&BTreeSet::from([2096]));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn idle_and_system_never_need_a_snapshot() {
        let (mut names, calls) = names(&[]);
        names.update(&BTreeSet::from([0, 4]));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(names.name(0), "Idle");
        assert_eq!(names.name(4), "System");
    }

    #[test]
    fn a_pid_absent_from_the_snapshot_is_cached_with_a_fallback() {
        let (mut names, calls) = names(&[(9, "")]);
        names.update(&BTreeSet::from([777, 9]));
        assert_eq!(names.name(777), "PID 777");
        assert_eq!(names.name(9), "PID 9", "empty names are not shown");
        names.update(&BTreeSet::from([777, 9]));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no second snapshot");
    }

    #[test]
    fn pids_gone_from_the_counters_are_forgotten() {
        let (mut names, calls) = names(&[(2096, "dwm.exe")]);
        names.update(&BTreeSet::from([2096]));
        names.update(&BTreeSet::new());
        assert_eq!(names.name(2096), "PID 2096");
        // Seen again (e.g. a reused pid): resolved again.
        names.update(&BTreeSet::from([2096]));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(names.name(2096), "dwm.exe");
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn toolhelp_names_the_desktop_window_manager() {
        let processes = toolhelp_processes();
        assert!(processes.len() > 20, "{} processes", processes.len());
        let own = std::process::id();
        let (_, name) = processes
            .iter()
            .find(|(pid, _)| *pid == own)
            .expect("the test process is listed");
        assert!(name.to_ascii_lowercase().ends_with(".exe"), "{name}");
        assert!(
            processes
                .iter()
                .any(|(_, name)| name.eq_ignore_ascii_case("dwm.exe")),
            "dwm.exe cannot be opened by a normal user but must be named"
        );
    }
}
