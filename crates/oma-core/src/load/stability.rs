//! Throughput stability of the GPU stress phases (plan DG7): the slowest 10 s window of a
//! phase over its fastest, after a 30 s warm-up.

use std::collections::BTreeMap;

use super::plan::Objective;

pub const WINDOW_MS: u64 = 10_000;
pub const WARMUP_MS: u64 = 30_000;
/// Below this a completed run is `low_stability` (DG8).
pub const MIN_STABILITY: f64 = 0.97;

#[derive(Debug, Default, Clone, Copy)]
struct Window {
    sum: f64,
    n: u32,
    throttled: bool,
}

#[derive(Debug)]
struct PhaseWindows {
    phase: u32,
    counts: bool,
    start_ms: u64,
    windows: BTreeMap<u64, Window>,
}

#[derive(Debug)]
pub struct StabilityMeter {
    skip_throttled: bool,
    current: Option<PhaseWindows>,
    /// Stability of each finished phase that had enough windows.
    done: Vec<f64>,
}

impl StabilityMeter {
    pub fn new(objective: Objective) -> Self {
        Self {
            skip_throttled: objective == Objective::Overclock,
            current: None,
            done: Vec::new(),
        }
    }

    /// Starts `phase` at `mono_ms`; a repeated call for the current phase does nothing.
    pub fn phase_started(&mut self, phase: u32, counts: bool, mono_ms: u64) {
        if self.current.as_ref().is_some_and(|c| c.phase == phase) {
            return;
        }
        if let Some(s) = self.current.take().and_then(|c| self.phase_stability(&c)) {
            self.done.push(s);
        }
        self.current = Some(PhaseWindows {
            phase,
            counts,
            start_ms: mono_ms,
            windows: BTreeMap::new(),
        });
    }

    /// The window of `mono_ms` in the current phase, `None` in the warm-up.
    fn window(&mut self, mono_ms: u64) -> Option<&mut Window> {
        let c = self.current.as_mut().filter(|c| c.counts)?;
        let t = mono_ms.checked_sub(c.start_ms + WARMUP_MS)?;
        Some(c.windows.entry(t / WINDOW_MS).or_default())
    }

    pub fn rate(&mut self, rate: f64, mono_ms: u64) {
        // The first `Progress` of a phase carries 0.
        if !(rate.is_finite() && rate > 0.0) {
            return;
        }
        if let Some(w) = self.window(mono_ms) {
            w.sum += rate;
            w.n += 1;
        }
    }

    pub fn throttling(&mut self, on: bool, mono_ms: u64) {
        if let (true, Some(w)) = (on, self.window(mono_ms)) {
            w.throttled = true;
        }
    }

    /// Min / max of the complete windows: a window is complete once a rate lands in a
    /// later one, so the last, cut short by the phase end, never counts.
    fn phase_stability(&self, c: &PhaseWindows) -> Option<f64> {
        let last = c.windows.iter().rev().find(|(_, w)| w.n > 0)?.0;
        let means: Vec<f64> = c
            .windows
            .range(..*last)
            .filter(|(_, w)| w.n > 0 && !(self.skip_throttled && w.throttled))
            .map(|(_, w)| w.sum / f64::from(w.n))
            .collect();
        if means.len() < 2 {
            return None;
        }
        let min = means.iter().copied().fold(f64::INFINITY, f64::min);
        let max = means.iter().copied().fold(0.0, f64::max);
        Some(min / max)
    }

    /// The worst phase so far, the current one included; `None` without a phase with at
    /// least 2 windows.
    pub fn result(&self) -> Option<f64> {
        let current = self.current.as_ref().and_then(|c| self.phase_stability(c));
        self.done.iter().copied().chain(current).reduce(f64::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1_000;

    /// One rate a second at `from_s..to_s` (seconds since the phase start at 0).
    fn feed(m: &mut StabilityMeter, from_s: u64, to_s: u64, rate: f64) {
        for t in from_s..to_s {
            m.rate(rate, t * S);
        }
    }

    fn meter(objective: Objective) -> StabilityMeter {
        let mut m = StabilityMeter::new(objective);
        m.phase_started(0, true, 0);
        m
    }

    #[test]
    fn steady_rates_give_stability_one() {
        let mut m = meter(Objective::Normal);
        feed(&mut m, 0, 71, 25.0);
        assert_eq!(m.result(), Some(1.0));
    }

    #[test]
    fn a_slow_window_lowers_stability() {
        let mut m = meter(Objective::Normal);
        feed(&mut m, 30, 50, 100.0);
        feed(&mut m, 50, 60, 95.0);
        // Closes the third window; the open one never counts.
        feed(&mut m, 60, 61, 10.0);
        assert!((m.result().unwrap() - 0.95).abs() < 1e-12);
    }

    #[test]
    fn warmup_is_excluded_from_stability() {
        let mut m = meter(Objective::Normal);
        feed(&mut m, 0, 30, 10.0);
        feed(&mut m, 30, 61, 100.0);
        assert_eq!(m.result(), Some(1.0));
    }

    fn throttled_run(objective: Objective) -> Option<f64> {
        let mut m = meter(objective);
        feed(&mut m, 30, 50, 100.0);
        feed(&mut m, 50, 60, 80.0);
        m.throttling(false, 45 * S);
        m.throttling(true, 55 * S);
        feed(&mut m, 60, 61, 100.0);
        m.result()
    }

    #[test]
    fn throttled_windows_are_excluded_in_overclock() {
        assert_eq!(throttled_run(Objective::Overclock), Some(1.0));
    }

    #[test]
    fn throttled_windows_count_in_normal() {
        assert!((throttled_run(Objective::Normal).unwrap() - 0.8).abs() < 1e-12);
    }

    #[test]
    fn too_few_windows_give_no_stability() {
        assert_eq!(StabilityMeter::new(Objective::Normal).result(), None);
        let mut m = meter(Objective::Normal);
        feed(&mut m, 30, 40, 100.0);
        feed(&mut m, 40, 41, 50.0);
        assert_eq!(m.result(), None, "one complete window");
        feed(&mut m, 41, 51, 50.0);
        assert_eq!(m.result(), Some(0.5));
    }

    #[test]
    fn phases_that_do_not_count_are_ignored() {
        let mut m = StabilityMeter::new(Objective::Normal);
        m.phase_started(0, false, 0);
        feed(&mut m, 30, 50, 100.0);
        feed(&mut m, 50, 61, 10.0);
        assert_eq!(m.result(), None);
    }

    #[test]
    fn session_stability_is_the_worst_phase() {
        let mut m = meter(Objective::Normal);
        feed(&mut m, 30, 40, 100.0);
        feed(&mut m, 40, 50, 98.0);
        // A repeated start of the same phase changes nothing.
        m.phase_started(0, true, 45 * S);
        feed(&mut m, 50, 55, 1.0);
        m.phase_started(1, true, 100 * S);
        feed(&mut m, 130, 140, 100.0);
        feed(&mut m, 140, 151, 90.0);
        assert!((m.result().unwrap() - 0.9).abs() < 1e-12);
    }
}
