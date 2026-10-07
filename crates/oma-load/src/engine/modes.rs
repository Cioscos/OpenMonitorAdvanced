//! The load modes (§4.3): `steady` runs the kernel without pauses; `variable` alternates
//! random bursts and pauses of 10–500 ms from the phase seed, switching between `kernel`
//! and `alt_kernel` at every burst; `light` (one thread) bursts for 200–2000 ms and pauses
//! for 50–500 ms. All the workers of a phase share the seed, so they start and stop
//! together: the load steps are as steep as they can be.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use oma_ipc::load::LoadMode;

use crate::kernel::Kernel;
use crate::rng::Xoshiro256ss;

/// The longest sleep of a pause, so `quit` is seen quickly and the beat moves.
const PAUSE_CHUNK: Duration = Duration::from_millis(50);
/// The end of a pause is waited for actively, for a precise load step.
const PAUSE_SPIN: Duration = Duration::from_millis(1);

/// The next burst and pause of `mode`.
pub(crate) fn durations(mode: LoadMode, rng: &mut Xoshiro256ss) -> (Duration, Duration) {
    let (busy, pause) = match mode {
        LoadMode::Light => ((200, 2000), (50, 500)),
        LoadMode::Steady | LoadMode::Variable => ((10, 500), (10, 500)),
    };
    let mut pick =
        |(lo, hi): (u64, u64)| Duration::from_millis(lo + rng.next_u64() % (hi - lo + 1));
    (pick(busy), pick(pause))
}

/// Waits until `deadline` or `quit`: sleeps until 1 ms before, then spins, bumping `beat`.
pub(crate) fn pause_until(deadline: Instant, beat: &AtomicU64, quit: &AtomicBool) {
    while !quit.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let left = deadline - now;
        if left > PAUSE_SPIN {
            thread::sleep((left - PAUSE_SPIN).min(PAUSE_CHUNK));
        } else {
            std::hint::spin_loop();
        }
        beat.fetch_add(1, Ordering::Relaxed);
    }
}

/// Runs `kernels` in `mode` until `quit`; `step(k, kernel)` runs and checks one iteration
/// of `kernels[k]`. In `variable` the bursts take the kernels in turn. A fixed-work phase
/// runs until `quit` too: `step` tells its counted steps from the filler ones.
pub(crate) fn work(
    mode: LoadMode,
    seed: u64,
    kernels: &mut [Box<dyn Kernel>],
    beat: &AtomicU64,
    quit: &AtomicBool,
    mut step: impl FnMut(usize, &mut dyn Kernel),
) {
    let running = || !quit.load(Ordering::Relaxed);
    if mode == LoadMode::Steady {
        while running() {
            step(0, kernels[0].as_mut());
        }
        return;
    }
    let mut rng = Xoshiro256ss::new(seed);
    let mut burst = 0usize;
    while running() {
        let (busy, pause) = durations(mode, &mut rng);
        let k = burst % kernels.len();
        let end = Instant::now() + busy;
        while running() && Instant::now() < end {
            step(k, kernels[k].as_mut());
        }
        pause_until(Instant::now() + pause, beat, quit);
        burst += 1;
    }
}
