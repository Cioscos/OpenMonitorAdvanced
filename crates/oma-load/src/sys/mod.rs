//! Thread placement and CPU identification.

#[cfg(windows)]
pub mod affinity;
pub mod cpuid;
pub mod memory;

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

/// The physical memory available now, in bytes (DA10); `None` when it cannot be read.
pub fn available_memory() -> Option<u64> {
    #[cfg(windows)]
    {
        oma_win::memory::memory_status()
            .map_err(|e| tracing::warn!(error = %e, "cannot read the available memory"))
            .ok()
            .map(|(_, available)| available)
    }
    #[cfg(not(windows))]
    {
        None
    }
}
