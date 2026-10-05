//! Which game to follow: the foreground process, when the service sees it
//! present at least [`MIN_GAME_FPS`] frames per second and it is neither one
//! of ours nor a system process. A target that stops qualifying (alt-tab, a
//! loading screen) is kept for [`TARGET_GRACE_MS`] before it drops. Pure: the
//! caller supplies a monotonic clock in milliseconds.

/// Fewest displayed frames per second for a process to count as a game.
pub const MIN_GAME_FPS: f64 = 10.0;

/// How long a target is kept after it last qualified.
pub const TARGET_GRACE_MS: u64 = 3_000;

/// Processes that are never a target, compared without regard to case.
pub const SYSTEM_EXCLUDED: &[&str] = &[
    "dwm.exe",
    "explorer.exe",
    "applicationframehost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "textinputhost.exe",
    "lockapp.exe",
    "csrss.exe",
    "msedgewebview2.exe",
    "oma-service.exe",
    "presentmon-2.6.0-x64.exe",
];

/// Our own executables, never a target whatever the excluded names: the
/// overlay window presents frames like any other (SD11, DP15) and its PID
/// changes at every restart.
pub const OWN_PROCESS_NAMES: &[&str] = &["oma-app.exe", "oma-overlay.exe"];

/// A process that presents frames, as the service reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub displayed_fps: f64,
}

pub struct TargetPicker {
    own_pids: Vec<u32>,
    /// Lowercase.
    excluded: Vec<String>,
    foreground: Option<u32>,
    processes: Vec<ProcessInfo>,
    current: Option<ProcessInfo>,
    /// When `current` last qualified as the candidate.
    last_candidate_ms: u64,
    /// The target the last [`Self::tick`] reported.
    reported: Option<u32>,
}

impl TargetPicker {
    pub fn new(own_pids: Vec<u32>, excluded_names: Vec<String>) -> Self {
        Self {
            own_pids,
            excluded: excluded_names
                .into_iter()
                .map(|name| name.to_lowercase())
                .collect(),
            foreground: None,
            processes: Vec::new(),
            current: None,
            last_candidate_ms: 0,
            reported: None,
        }
    }

    /// The foreground window now belongs to `pid`.
    pub fn on_foreground(&mut self, pid: u32, now_ms: u64) {
        // The state up to now counts before the change.
        self.evaluate(now_ms);
        self.foreground = Some(pid);
        self.evaluate(now_ms);
    }

    /// The latest list of presenting processes.
    pub fn on_processes(&mut self, procs: &[ProcessInfo], now_ms: u64) {
        self.evaluate(now_ms);
        self.processes.clear();
        self.processes.extend_from_slice(procs);
        self.evaluate(now_ms);
    }

    pub fn current(&self) -> Option<&ProcessInfo> {
        self.current.as_ref()
    }

    /// The new target (`Some(None)` when it dropped) when it changed since
    /// the previous tick, `None` otherwise.
    pub fn tick(&mut self, now_ms: u64) -> Option<Option<u32>> {
        self.evaluate(now_ms);
        let pid = self.current.as_ref().map(|p| p.pid);
        if pid == self.reported {
            None
        } else {
            self.reported = pid;
            Some(pid)
        }
    }

    fn candidate(&self) -> Option<&ProcessInfo> {
        let pid = self.foreground?;
        if self.own_pids.contains(&pid) {
            return None;
        }
        self.processes.iter().find(|p| {
            p.pid == pid && p.displayed_fps >= MIN_GAME_FPS && !is_excluded(&self.excluded, &p.name)
        })
    }

    fn evaluate(&mut self, now_ms: u64) {
        if let Some(candidate) = self.candidate().cloned() {
            self.current = Some(candidate);
            self.last_candidate_ms = now_ms;
        } else if self.current.is_some()
            && now_ms.saturating_sub(self.last_candidate_ms) >= TARGET_GRACE_MS
        {
            self.current = None;
        }
    }
}

fn is_excluded(excluded: &[String], name: &str) -> bool {
    let name = name.to_lowercase();
    OWN_PROCESS_NAMES.contains(&name.as_str()) || excluded.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: u32 = 7;

    fn picker() -> TargetPicker {
        TargetPicker::new(
            vec![OWN],
            SYSTEM_EXCLUDED.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    fn proc(pid: u32, name: &str, fps: f64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_owned(),
            displayed_fps: fps,
        }
    }

    fn game() -> ProcessInfo {
        proc(100, "game.exe", 144.0)
    }

    fn other_game() -> ProcessInfo {
        proc(200, "other.exe", 60.0)
    }

    /// A picker following `game()` from time 0.
    fn following_game() -> TargetPicker {
        let mut p = picker();
        p.on_processes(&[game(), other_game()], 0);
        p.on_foreground(100, 0);
        assert_eq!(p.tick(0), Some(Some(100)));
        p
    }

    #[test]
    fn foreground_game_becomes_target() {
        let mut p = picker();
        p.on_processes(&[game()], 0);
        assert_eq!(p.current(), None);
        p.on_foreground(100, 10);
        assert_eq!(p.current(), Some(&game()));
        assert_eq!(p.tick(10), Some(Some(100)));
    }

    #[test]
    fn slow_presenter_is_not_a_game() {
        let mut p = picker();
        p.on_processes(&[proc(100, "slow.exe", 9.0)], 0);
        p.on_foreground(100, 0);
        assert_eq!(p.current(), None);
        assert_eq!(p.tick(0), None);
    }

    #[test]
    fn excluded_names_are_never_targets() {
        let mut p = picker();
        p.on_processes(&[proc(50, "DWM.EXE", 240.0)], 0);
        p.on_foreground(50, 0);
        assert_eq!(p.current(), None);
        assert_eq!(p.tick(0), None);
    }

    #[test]
    fn own_processes_are_never_targets() {
        // Built without any excluded name: our own executables stay excluded.
        let mut p = TargetPicker::new(vec![], vec![]);
        p.on_processes(&[proc(60, "OMA-Overlay.exe", 60.0)], 0);
        p.on_foreground(60, 0);
        assert_eq!(p.current(), None);
        assert_eq!(p.tick(0), None);
        p.on_processes(&[proc(61, "oma-app.exe", 60.0)], 0);
        p.on_foreground(61, 0);
        assert_eq!(p.current(), None);
        assert!(OWN_PROCESS_NAMES.contains(&"oma-overlay.exe"));
    }

    #[test]
    fn own_pids_are_never_targets() {
        let mut p = picker();
        p.on_processes(&[proc(OWN, "renamed.exe", 60.0)], 0);
        p.on_foreground(OWN, 0);
        assert_eq!(p.current(), None);
        assert_eq!(p.tick(0), None);
    }

    #[test]
    fn target_survives_a_short_alt_tab() {
        let mut p = following_game();
        p.on_foreground(300, 1_000);
        p.tick(1_000);
        p.tick(3_000);
        assert_eq!(p.current(), Some(&game()));
        // Back in the game before the grace ran out: no change at all.
        p.on_foreground(100, 3_000);
        assert_eq!(p.tick(3_000), None);
        assert_eq!(p.current(), Some(&game()));
    }

    #[test]
    fn target_drops_after_three_seconds() {
        let mut p = following_game();
        p.on_foreground(300, 1_000);
        assert_eq!(p.tick(3_999), None);
        assert_eq!(p.current(), Some(&game()));
        assert_eq!(p.tick(4_000), Some(None));
        assert_eq!(p.current(), None);
    }

    #[test]
    fn switching_to_another_game_is_immediate() {
        let mut p = following_game();
        p.on_foreground(200, 500);
        assert_eq!(p.current(), Some(&other_game()));
        assert_eq!(p.tick(500), Some(Some(200)));
    }

    #[test]
    fn tick_reports_only_changes() {
        let mut p = picker();
        assert_eq!(p.tick(0), None);
        p.on_processes(&[game()], 0);
        p.on_foreground(100, 0);
        assert_eq!(p.tick(0), Some(Some(100)));
        assert_eq!(p.tick(250), None);
        p.on_processes(&[game()], 500);
        assert_eq!(p.tick(500), None);
        // The game stops presenting: it drops 3 s after it last qualified.
        p.on_processes(&[], 1_000);
        assert_eq!(p.tick(3_500), None);
        assert_eq!(p.tick(4_000), Some(None));
        assert_eq!(p.tick(4_250), None);
    }
}
