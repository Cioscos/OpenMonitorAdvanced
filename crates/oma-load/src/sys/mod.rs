//! Thread placement and CPU identification.

#[cfg(windows)]
pub mod affinity;
pub mod cpuid;

#[cfg(windows)]
pub use affinity::full_topology;

use std::io;

use oma_ipc::load::LogicalCpu;

/// Pins the calling thread to `cpu`; nothing to do off Windows.
pub fn pin(cpu: &LogicalCpu) -> io::Result<()> {
    #[cfg(windows)]
    {
        affinity::pin_current_thread(cpu)
    }
    #[cfg(not(windows))]
    {
        let _ = cpu;
        Ok(())
    }
}

/// Below-normal priority and EcoQoS off for a worker thread. A failure is not fatal: it
/// is logged once per process.
pub fn prepare_worker() {
    #[cfg(windows)]
    if let Err(e) = affinity::prepare_worker_thread() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            tracing::warn!(error = %e, "cannot prepare a worker thread");
        }
    }
}

/// Milliseconds the system has slept since boot (DA15); 0 off Windows.
pub fn asleep_ms() -> u64 {
    #[cfg(windows)]
    {
        oma_win::power::asleep_ms()
    }
    #[cfg(not(windows))]
    {
        0
    }
}
