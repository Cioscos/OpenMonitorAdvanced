//! Total dedicated VRAM of discrete GPUs, from the DXGI adapter description
//! captured at enumeration (static: no API call per tick).

use std::collections::BTreeSet;

use oma_core::model::Source;
use oma_core::provider::ProviderError;

use super::adapter::Adapter;
use super::field::GpuField;
use super::layer::{GpuLayer, Readings};

/// Dedicated VRAM in bytes. Integrated GPUs only have a firmware carve-out of
/// system RAM (485 MiB on the Radeon iGPU here), which is not VRAM.
pub(crate) fn dedicated_total(adapter: &Adapter) -> Option<f64> {
    (!adapter.integrated && adapter.dedicated_bytes > 0).then_some(adapter.dedicated_bytes as f64)
}

#[derive(Default)]
pub(crate) struct DxgiLayer {
    totals: Vec<Option<f64>>,
}

impl GpuLayer for DxgiLayer {
    fn source(&self) -> Source {
        Source::Dxgi
    }

    fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>> {
        self.totals = adapters.iter().map(dedicated_total).collect();
        self.totals
            .iter()
            .map(|total| match total {
                Some(_) => BTreeSet::from([GpuField::MemoryDedicatedTotal]),
                None => BTreeSet::new(),
            })
            .collect()
    }

    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
        Ok(self
            .totals
            .iter()
            .map(|total| {
                total
                    .map(|bytes| Readings::from([(GpuField::MemoryDedicatedTotal, bytes)]))
                    .unwrap_or_default()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(integrated: bool, dedicated_bytes: u64) -> Adapter {
        Adapter {
            luid: 0x17DB6,
            name: "Test GPU".to_owned(),
            vendor_id: 0x10DE,
            device_id: 0x2704,
            subsys_id: 0x5111_1462,
            pci: None,
            integrated,
            dedicated_bytes,
        }
    }

    #[test]
    fn only_discrete_adapters_report_a_total() {
        assert_eq!(
            dedicated_total(&adapter(false, 16_048 << 20)),
            Some((16_048u64 << 20) as f64)
        );
        assert_eq!(dedicated_total(&adapter(true, 485 << 20)), None);
        assert_eq!(dedicated_total(&adapter(false, 0)), None);
    }

    #[test]
    fn attach_and_sample_follow_adapter_order() {
        let mut layer = DxgiLayer::default();
        assert_eq!(layer.source(), Source::Dxgi);
        let adapters = [adapter(false, 16 << 30), adapter(true, 512 << 20)];
        assert_eq!(
            layer.attach(&adapters),
            vec![
                BTreeSet::from([GpuField::MemoryDedicatedTotal]),
                BTreeSet::new()
            ]
        );
        for _ in 0..2 {
            let values = layer.sample().expect("sample");
            assert_eq!(
                values,
                vec![
                    Readings::from([(GpuField::MemoryDedicatedTotal, (16u64 << 30) as f64)]),
                    Readings::new()
                ]
            );
        }
    }

    #[test]
    fn reattach_replaces_the_previous_adapters() {
        let mut layer = DxgiLayer::default();
        layer.attach(&[adapter(false, 8 << 30)]);
        assert!(layer.attach(&[]).is_empty());
        assert_eq!(layer.sample(), Ok(vec![]));
    }
}
