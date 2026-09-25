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

use oma_core::provider::Provider;

/// Every unprivileged Windows provider, in display order. `vendor` is the
/// safe-mode switch for the GPU vendor libraries (spec §8).
pub fn default_providers(vendor: gpu::VendorSwitch) -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(cpu::CpuProvider::new()),
        Box::new(gpu::GpuProvider::new(vendor)),
        Box::new(memory::MemoryProvider),
        Box::new(storage::StorageProvider::default()),
        Box::new(network::NetworkProvider::default()),
    ]
}
