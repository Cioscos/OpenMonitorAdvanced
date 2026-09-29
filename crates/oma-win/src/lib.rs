//! Unprivileged Windows data providers for OpenMonitor Advanced.
#![cfg(windows)]

pub mod cpu;
pub mod crash;
pub(crate) mod dynlib;
pub mod fsutil;
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

/// Handles shared with the sensor service link (spec §M4), owned by the
/// caller and cloned into the providers that need them: the `svc` provider
/// reads `feed`, and both it and the storage provider share `drives` so
/// service devices can bind onto the disks the storage provider discovers.
#[derive(Clone, Default)]
pub struct ServiceHandles {
    pub feed: svc::SvcFeed,
    pub drives: storage::DriveIdTable,
}

/// Every unprivileged Windows provider, in display order. `vendor` holds the
/// switches for the GPU vendor libraries: safe mode (spec §8) and one per
/// library. `processes` receives the per-process GPU usage (read by the
/// shell's `get_gpu_processes`).
/// `service` is last: it binds its devices onto the ids the other providers
/// (CPU, memory, storage) have already published for this discovery.
pub fn default_providers(
    vendor: gpu::VendorSwitch,
    processes: gpu::GpuProcessTable,
    service: ServiceHandles,
) -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(cpu::CpuProvider::new()),
        Box::new(gpu::GpuProvider::new(vendor, processes)),
        Box::new(memory::MemoryProvider),
        Box::new(storage::StorageProvider::new(service.drives.clone())),
        Box::new(network::NetworkProvider::default()),
        Box::new(svc::SvcProvider::new(service.feed, service.drives)),
    ]
}
