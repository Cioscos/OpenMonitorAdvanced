//! Unprivileged Windows data providers for OpenMonitor Advanced.
#![cfg(windows)]

pub mod cpu;
pub mod crash;
pub(crate) mod dynlib;
pub mod gpu;
pub mod memory;
pub mod network;
mod pdh;
pub mod storage;
mod storage_identity;
mod storage_ioctl;
mod storage_temperature;
pub mod svc;

use oma_core::provider::Provider;

/// Every unprivileged Windows provider, in display order. `vendor` is the
/// safe-mode switch for the GPU vendor libraries (spec §8); `processes`
/// receives the per-process GPU usage (read by the shell's `get_gpu_processes`).
pub fn default_providers(
    vendor: gpu::VendorSwitch,
    processes: gpu::GpuProcessTable,
) -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(cpu::CpuProvider::new()),
        Box::new(gpu::GpuProvider::new(vendor, processes)),
        Box::new(memory::MemoryProvider),
        Box::new(storage::StorageProvider::default()),
        Box::new(network::NetworkProvider::default()),
    ]
}
