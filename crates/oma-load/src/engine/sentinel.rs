//! The sentinel: a thread that looks at the workers' beats once a second and reports a
//! worker whose beat has not moved for [`HUNG_AFTER_MS`]. A suspend (DA15) resets the base
//! of every beat, so a PC that slept is never taken for a hung worker.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// A worker whose beat stands still this long is hung.
#[cfg(not(test))]
pub(crate) const HUNG_AFTER_MS: u64 = 10_000;
#[cfg(test)]
pub(crate) const HUNG_AFTER_MS: u64 = 1_000;

#[cfg(not(test))]
const TICK: Duration = Duration::from_secs(1);
#[cfg(test)]
const TICK: Duration = Duration::from_millis(50);

/// Growth of `asleep_ms` between two ticks that counts as a suspend: the value jitters by
/// a tick of `GetTickCount64` (about 16 ms) while the PC is awake.
const SUSPEND_GAP_MS: u64 = 1_000;

/// The pure part of the sentinel.
pub(crate) struct Watch {
    generation: u64,
    beats: Vec<u64>,
    since_ms: Vec<u64>,
    asleep_ms: u64,
}

impl Watch {
    pub(crate) fn new(asleep_ms: u64) -> Self {
        Self {
            generation: 0,
            beats: Vec::new(),
            since_ms: Vec::new(),
            asleep_ms,
        }
    }

    /// One look at the `beats` of the workers of `generation` (a new set of workers has a
    /// new generation) at `now_ms`, with the system's `asleep_ms`. Returns the first hung
    /// worker.
    pub(crate) fn tick(
        &mut self,
        now_ms: u64,
        asleep_ms: u64,
        generation: u64,
        beats: &[u64],
    ) -> Option<usize> {
        let slept = asleep_ms.saturating_sub(self.asleep_ms) > SUSPEND_GAP_MS;
        self.asleep_ms = asleep_ms;
        if slept || generation != self.generation || beats.len() != self.beats.len() {
            self.generation = generation;
            self.beats = beats.to_vec();
            self.since_ms = vec![now_ms; beats.len()];
            return None;
        }
        let mut hung = None;
        for (i, &beat) in beats.iter().enumerate() {
            if beat != self.beats[i] {
                self.beats[i] = beat;
                self.since_ms[i] = now_ms;
            } else if hung.is_none() && now_ms.saturating_sub(self.since_ms[i]) >= HUNG_AFTER_MS {
                hung = Some(i);
            }
        }
        hung
    }
}

/// Runs the sentinel until `done`: `beats` gives the generation and the beats of the
/// current workers, `asleep_ms` the time the system slept, and `on_stall` is called once
/// with the first hung worker, after which the sentinel ends.
pub(crate) fn watch_loop(
    done: &AtomicBool,
    beats: impl Fn() -> (u64, Vec<u64>),
    asleep_ms: impl Fn() -> u64,
    on_stall: impl FnOnce(usize),
) {
    let start = Instant::now();
    let mut watch = Watch::new(asleep_ms());
    loop {
        // The engine unparks the sentinel when it is done.
        thread::park_timeout(TICK);
        if done.load(Ordering::Acquire) {
            return;
        }
        let (generation, b) = beats();
        let now_ms = start.elapsed().as_millis() as u64;
        if let Some(i) = watch.tick(now_ms, asleep_ms(), generation, &b) {
            on_stall(i);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_ignores_a_suspend_gap() {
        let mut w = Watch::new(0);
        assert_eq!(w.tick(0, 0, 1, &[5, 9]), None, "a new set of workers");
        assert_eq!(w.tick(500, 0, 1, &[5, 10]), None);
        // The PC slept for 60 s: the monotonic clock jumped, and so did `asleep_ms`.
        assert_eq!(w.tick(60_500, 60_000, 1, &[5, 10]), None);
        assert_eq!(
            w.tick(60_500 + HUNG_AFTER_MS - 1, 60_010, 1, &[5, 10]),
            None,
            "the beats count again from the resume"
        );
        assert_eq!(w.tick(60_500 + HUNG_AFTER_MS, 60_010, 1, &[5, 10]), Some(0));

        // The same gap without a suspend is a hung worker.
        let mut awake = Watch::new(0);
        assert_eq!(awake.tick(0, 0, 1, &[5]), None);
        assert_eq!(awake.tick(60_000, 0, 1, &[5]), Some(0));
    }

    #[test]
    fn a_moving_beat_or_a_new_generation_is_not_hung() {
        let mut w = Watch::new(0);
        assert_eq!(w.tick(0, 0, 1, &[0]), None);
        assert_eq!(w.tick(HUNG_AFTER_MS, 0, 1, &[1]), None);
        assert_eq!(w.tick(2 * HUNG_AFTER_MS, 0, 2, &[1]), None, "new workers");
        assert_eq!(w.tick(3 * HUNG_AFTER_MS, 0, 2, &[1]), Some(0));
    }
}
