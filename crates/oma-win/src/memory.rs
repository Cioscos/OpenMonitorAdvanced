//! Physical memory usage from GlobalMemoryStatusEx.

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

const DEVICE_ID: &str = "memory/0";

pub(crate) fn used_bytes(total: u64, available: u64) -> u64 {
    total.saturating_sub(available)
}

pub(crate) fn used_pct(total: u64, available: u64) -> Option<f64> {
    (total > 0).then(|| used_bytes(total, available) as f64 * 100.0 / total as f64)
}

fn memory_status() -> Result<MEMORYSTATUSEX, ProviderError> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `dwLength` is set as the API requires.
    unsafe { GlobalMemoryStatusEx(&mut status) }
        .map_err(|e| ProviderError::Failed(format!("GlobalMemoryStatusEx: {e}")))?;
    Ok(status)
}

#[derive(Default)]
pub struct MemoryProvider;

impl Provider for MemoryProvider {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        memory_status()?;
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
        let s = memory_status()?;
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
}
