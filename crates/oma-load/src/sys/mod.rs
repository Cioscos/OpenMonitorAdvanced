//! Thread placement and CPU identification.

#[cfg(windows)]
pub mod affinity;
pub mod cpuid;

#[cfg(windows)]
pub use affinity::full_topology;
