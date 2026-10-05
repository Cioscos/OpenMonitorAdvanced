//! The performance counter's frequency: what turns the QPC timestamps of the
//! frame data (`FrameBatch`, `PresentingProcesses`) into seconds.

use std::sync::OnceLock;

use windows::Win32::System::Performance::QueryPerformanceFrequency;

/// Ticks per second of `QueryPerformanceCounter`. Fixed at boot, so it is
/// read once and cached. 0 only if the call failed, which it never does on
/// Windows XP and later.
pub fn qpc_frequency() -> u64 {
    static FREQUENCY: OnceLock<u64> = OnceLock::new();
    *FREQUENCY.get_or_init(|| {
        let mut frequency = 0i64;
        // SAFETY: `frequency` is a live, writable i64 for the whole call, which
        // only writes the frequency through the pointer and keeps nothing.
        match unsafe { QueryPerformanceFrequency(&mut frequency) } {
            Ok(()) => u64::try_from(frequency).unwrap_or(0),
            Err(e) => {
                tracing::error!("QueryPerformanceFrequency failed: {e}");
                0
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qpc_frequency_is_positive_and_stable() {
        let first = qpc_frequency();
        assert!(first > 0);
        assert_eq!(qpc_frequency(), first);
    }
}
