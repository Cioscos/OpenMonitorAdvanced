//! Keep-awake and sleep detection for the stress test (M8a1).

use std::marker::PhantomData;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::System::Power::{
    GetSystemPowerStatus, SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
    EXECUTION_STATE, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemInformation::GetTickCount64;

/// Keeps the system awake (no idle sleep) while alive. The state belongs to
/// the thread that calls it, so the guard is `!Send`: drop it on that thread.
pub struct KeepAwake {
    /// The state `SetThreadExecutionState` returned in `new`, restored on drop.
    previous: EXECUTION_STATE,
    _not_send: PhantomData<*const ()>,
}

impl KeepAwake {
    pub fn new() -> Self {
        // SAFETY: plain flags; the call only changes this thread's execution state.
        let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
        if previous.0 == 0 {
            tracing::warn!("SetThreadExecutionState failed: the PC may sleep during the test");
        }
        Self {
            previous,
            _not_send: PhantomData,
        }
    }
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        // Back to what the thread had before (correct with nested guards); a failed `new`
        // (previous 0) falls back to plain ES_CONTINUOUS, which clears the requirement.
        let restore = EXECUTION_STATE(self.previous.0 | ES_CONTINUOUS.0);
        // SAFETY: as in `new`.
        unsafe { SetThreadExecutionState(restore) };
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

/// `ACLineStatus`: 0 offline (on battery) is `Some(true)`, 1 online is
/// `Some(false)`, 255 unknown or anything else is `None`.
pub fn battery_from_line_status(status: u8) -> Option<bool> {
    match status {
        0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

/// Whether the PC runs on battery, or `None` when Windows cannot tell.
pub fn on_battery() -> Option<bool> {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: `status` is a live, writable SYSTEM_POWER_STATUS for the call.
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    battery_from_line_status(status.ACLineStatus)
}

#[cfg(test)]
mod tests {
    #[test]
    fn battery_from_line_status_values() {
        assert_eq!(super::battery_from_line_status(0), Some(true));
        assert_eq!(super::battery_from_line_status(1), Some(false));
        assert_eq!(super::battery_from_line_status(255), None);
        assert_eq!(super::battery_from_line_status(2), None);
    }

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
