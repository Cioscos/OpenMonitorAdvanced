//! Keep-awake and sleep detection for the stress test (M8a1).

use std::marker::PhantomData;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
use windows::Win32::System::SystemInformation::GetTickCount64;

/// Keeps the system awake (no idle sleep) while alive. The state belongs to
/// the thread that calls it, so the guard is `!Send`: drop it on that thread.
pub struct KeepAwake(PhantomData<*const ()>);

impl KeepAwake {
    pub fn new() -> Self {
        // SAFETY: plain flags; the call only changes this thread's execution state.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
        Self(PhantomData)
    }
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        // SAFETY: as in `new`; ES_CONTINUOUS alone clears the requirement.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

#[link(name = "kernel32")]
extern "system" {
    /// `BOOL QueryUnbiasedInterruptTime(PULONGLONG)`, declared here to avoid
    /// one more `windows` feature for a single call.
    fn QueryUnbiasedInterruptTime(unbiased_time: *mut u64) -> i32;
}

/// When the system booted, in milliseconds since the Unix epoch: now minus
/// the uptime of `GetTickCount64`.
pub fn boot_time_unix_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    // SAFETY: no arguments, returns the milliseconds since boot.
    now - unsafe { GetTickCount64() } as i64
}

/// Milliseconds the system has slept since boot: `GetTickCount64` counts
/// sleep, the unbiased interrupt time (100 ns units) does not. It only grows
/// while the PC sleeps and ignores clock changes.
pub fn asleep_ms() -> u64 {
    let mut unbiased = 0u64;
    // SAFETY: `unbiased` is a live, writable u64 for the call.
    let ok = unsafe { QueryUnbiasedInterruptTime(&mut unbiased) };
    if ok == 0 {
        return 0;
    }
    // SAFETY: no arguments.
    let ticks = unsafe { GetTickCount64() };
    ticks.saturating_sub(unbiased / 10_000)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn asleep_ms_does_not_grow_while_awake() {
        let a = asleep_ms();
        std::thread::sleep(Duration::from_millis(100));
        let b = asleep_ms();
        assert!(b.abs_diff(a) < 50, "asleep_ms moved from {a} to {b}");
    }

    #[test]
    fn boot_time_is_in_the_past() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!(boot_time_unix_ms() < now);
    }

    #[test]
    fn keep_awake_can_be_created_and_dropped() {
        drop(KeepAwake::new());
    }
}
