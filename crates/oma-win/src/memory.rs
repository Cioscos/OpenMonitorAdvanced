//! Physical memory usage from GlobalMemoryStatusEx.

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use windows::Win32::System::SystemInformation::{
    GetPhysicallyInstalledSystemMemory, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};

const DEVICE_ID: &str = "memory/0";

pub(crate) fn used_bytes(total: u64, available: u64) -> u64 {
    total.saturating_sub(available)
}

pub(crate) fn used_pct(total: u64, available: u64) -> Option<f64> {
    (total > 0).then(|| used_bytes(total, available) as f64 * 100.0 / total as f64)
}

fn status_ex() -> Result<MEMORYSTATUSEX, ProviderError> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `dwLength` is set as the API requires.
    unsafe { GlobalMemoryStatusEx(&mut status) }
        .map_err(|e| ProviderError::Failed(format!("GlobalMemoryStatusEx: {e}")))?;
    Ok(status)
}

/// Total and available physical memory, in bytes.
pub fn memory_status() -> std::io::Result<(u64, u64)> {
    let s = status_ex().map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok((s.ullTotalPhys, s.ullAvailPhys))
}

/// Installed RAM in whole GiB from the firmware's KiB count, between 1 and 4096.
pub fn ram_gb_from_kib(kib: u64) -> u32 {
    let gib = kib.saturating_add(512 * 1024) / (1024 * 1024);
    gib.clamp(1, 4096) as u32
}

/// RAM in whole GiB from a byte count, rounded up, between 1 and 4096.
pub fn ram_gb_from_bytes(bytes: u64) -> u32 {
    const GIB: u64 = 1 << 30;
    (bytes / GIB + u64::from(bytes % GIB != 0)).clamp(1, 4096) as u32
}

/// Installed RAM in GiB: `GetPhysicallyInstalledSystemMemory`, or the usable
/// `ullTotalPhys` rounded up (a little under the installed size) if that fails.
pub fn installed_ram_gb() -> Option<u32> {
    let mut kib = 0u64;
    // SAFETY: `kib` is a valid, writable u64 alive for the call.
    if unsafe { GetPhysicallyInstalledSystemMemory(&mut kib) }.is_ok() && kib > 0 {
        return Some(ram_gb_from_kib(kib));
    }
    memory_status()
        .ok()
        .map(|(total, _)| ram_gb_from_bytes(total))
}

#[derive(Default)]
pub struct MemoryProvider;

impl Provider for MemoryProvider {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        status_ex()?;
        Ok(Inventory {
            devices: vec![Device {
                id: DEVICE_ID.to_owned(),
                kind: DeviceKind::Memory,
                name: "RAM".to_owned(),
                vendor: None,
                properties: Default::default(),
            }],
            sensors: vec![
                Sensor::new(
                    DEVICE_ID,
                    SensorKind::Load,
                    "used",
                    Unit::Percent,
                    Label::new("memory.load"),
                    Source::Win32,
                ),
                Sensor::new(
                    DEVICE_ID,
                    SensorKind::Data,
                    "used",
                    Unit::Bytes,
                    Label::new("memory.used"),
                    Source::Win32,
                ),
                Sensor::new(
                    DEVICE_ID,
                    SensorKind::Data,
                    "total",
                    Unit::Bytes,
                    Label::new("memory.total"),
                    Source::Win32,
                ),
            ],
        })
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let s = status_ex()?;
        Ok(vec![
            used_pct(s.ullTotalPhys, s.ullAvailPhys),
            Some(used_bytes(s.ullTotalPhys, s.ullAvailPhys) as f64),
            Some(s.ullTotalPhys as f64),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn used_is_total_minus_available() {
        assert_eq!(used_bytes(32, 8), 24);
        assert_eq!(used_bytes(8, 32), 0);
    }

    #[test]
    fn used_percentage() {
        assert_eq!(used_pct(32, 8), Some(75.0));
        assert_eq!(used_pct(200, 50), Some(75.0));
        assert_eq!(used_pct(0, 0), None);
    }

    #[test]
    fn ram_gb_rounds_to_the_installed_size() {
        assert_eq!(ram_gb_from_kib(33_554_432), 32);
        assert_eq!(ram_gb_from_kib(16_777_216), 16);
        assert_eq!(ram_gb_from_kib(33_520_000), 32);
        assert_eq!(ram_gb_from_kib(0), 1);
    }

    #[test]
    fn ram_gb_from_bytes_rounds_up() {
        let gib = 1u64 << 30;
        assert_eq!(ram_gb_from_bytes(gib * 312 / 10), 32);
        assert_eq!(ram_gb_from_bytes(0), 1);
        assert_eq!(ram_gb_from_bytes(gib * 5000), 4096);
    }
}
