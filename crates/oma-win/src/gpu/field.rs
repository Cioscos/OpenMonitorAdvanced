//! Every GPU reading a layer can provide, with its sensor kind, id segment, unit and label.

use oma_core::model::{SensorKind, Unit};

/// A GPU reading. Declaration order = `ALL` order = sensor order in the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum GpuField {
    LoadCore,
    Load3d,
    LoadCompute,
    LoadCopy,
    LoadVideoDecode,
    LoadVideoEncode,
    /// NVENC utilization as NVML reports it (average of the encoder engines).
    LoadEncoder,
    /// NVDEC utilization as NVML reports it (average of the decoder engines).
    LoadDecoder,
    MemoryDedicatedUsed,
    MemoryDedicatedTotal,
    MemorySharedUsed,
    TemperatureCore,
    TemperatureHotspot,
    TemperatureMemory,
    ClockCore,
    ClockMemory,
    PowerBoard,
    PowerLimit,
    PowerLimitPercent,
    FanPercent,
    FanRpm,
    VoltageCore,
    /// 1.0 while the GPU is limited by its power budget, else 0.0.
    ThrottlePower,
    /// 1.0 while the GPU is limited by temperature, else 0.0.
    ThrottleThermal,
    /// Current PCIe link generation (live: drops to Gen 1 at idle with ASPM).
    PcieLinkGen,
    /// Current PCIe link width in lanes.
    PcieLinkWidth,
}

impl GpuField {
    pub const ALL: [GpuField; 26] = [
        GpuField::LoadCore,
        GpuField::Load3d,
        GpuField::LoadCompute,
        GpuField::LoadCopy,
        GpuField::LoadVideoDecode,
        GpuField::LoadVideoEncode,
        GpuField::LoadEncoder,
        GpuField::LoadDecoder,
        GpuField::MemoryDedicatedUsed,
        GpuField::MemoryDedicatedTotal,
        GpuField::MemorySharedUsed,
        GpuField::TemperatureCore,
        GpuField::TemperatureHotspot,
        GpuField::TemperatureMemory,
        GpuField::ClockCore,
        GpuField::ClockMemory,
        GpuField::PowerBoard,
        GpuField::PowerLimit,
        GpuField::PowerLimitPercent,
        GpuField::FanPercent,
        GpuField::FanRpm,
        GpuField::VoltageCore,
        GpuField::ThrottlePower,
        GpuField::ThrottleThermal,
        GpuField::PcieLinkGen,
        GpuField::PcieLinkWidth,
    ];

    pub fn kind(self) -> SensorKind {
        use GpuField::*;
        match self {
            LoadCore | Load3d | LoadCompute | LoadCopy | LoadVideoDecode | LoadVideoEncode
            | LoadEncoder | LoadDecoder => SensorKind::Load,
            MemoryDedicatedUsed | MemoryDedicatedTotal | MemorySharedUsed => SensorKind::Data,
            TemperatureCore | TemperatureHotspot | TemperatureMemory => SensorKind::Temperature,
            ClockCore | ClockMemory => SensorKind::Clock,
            PowerBoard | PowerLimit => SensorKind::Power,
            PowerLimitPercent => SensorKind::Percent,
            FanPercent | FanRpm => SensorKind::Fan,
            VoltageCore => SensorKind::Voltage,
            ThrottlePower | ThrottleThermal => SensorKind::Flag,
            PcieLinkGen | PcieLinkWidth => SensorKind::Link,
        }
    }

    /// Last segment of the sensor id (`<device>/<kind>/<name>`).
    pub fn name(self) -> &'static str {
        use GpuField::*;
        match self {
            LoadCore => "core",
            Load3d => "3d",
            LoadCompute => "compute",
            LoadCopy => "copy",
            LoadVideoDecode => "video-decode",
            LoadVideoEncode => "video-encode",
            LoadEncoder => "encoder",
            LoadDecoder => "decoder",
            MemoryDedicatedUsed => "memory-dedicated-used",
            MemoryDedicatedTotal => "memory-dedicated-total",
            MemorySharedUsed => "memory-shared-used",
            TemperatureCore => "core",
            TemperatureHotspot => "hotspot",
            TemperatureMemory => "memory",
            ClockCore => "core",
            ClockMemory => "memory",
            PowerBoard => "board",
            PowerLimit => "limit",
            PowerLimitPercent => "power-limit",
            FanPercent => "percent",
            FanRpm => "rpm",
            VoltageCore => "core",
            ThrottlePower => "throttle-power",
            ThrottleThermal => "throttle-thermal",
            PcieLinkGen => "pcie-gen",
            PcieLinkWidth => "pcie-width",
        }
    }

    pub fn unit(self) -> Unit {
        use GpuField::*;
        match self {
            LoadCore | Load3d | LoadCompute | LoadCopy | LoadVideoDecode | LoadVideoEncode
            | LoadEncoder | LoadDecoder | PowerLimitPercent | FanPercent => Unit::Percent,
            MemoryDedicatedUsed | MemoryDedicatedTotal | MemorySharedUsed => Unit::Bytes,
            TemperatureCore | TemperatureHotspot | TemperatureMemory => Unit::Celsius,
            ClockCore | ClockMemory => Unit::Megahertz,
            PowerBoard | PowerLimit => Unit::Watt,
            FanRpm => Unit::Rpm,
            VoltageCore => Unit::Volt,
            ThrottlePower | ThrottleThermal => Unit::Boolean,
            PcieLinkGen => Unit::PcieGeneration,
            PcieLinkWidth => Unit::Lanes,
        }
    }

    /// Translation key without the `sensor.` prefix (the UI adds it).
    pub fn label_key(self) -> &'static str {
        use GpuField::*;
        match self {
            LoadCore => "gpu.load.core",
            Load3d => "gpu.load.3d",
            LoadCompute => "gpu.load.compute",
            LoadCopy => "gpu.load.copy",
            LoadVideoDecode => "gpu.load.videoDecode",
            LoadVideoEncode => "gpu.load.videoEncode",
            LoadEncoder => "gpu.load.encoder",
            LoadDecoder => "gpu.load.decoder",
            MemoryDedicatedUsed => "gpu.memory.dedicatedUsed",
            MemoryDedicatedTotal => "gpu.memory.dedicatedTotal",
            MemorySharedUsed => "gpu.memory.sharedUsed",
            TemperatureCore => "gpu.temperature.core",
            TemperatureHotspot => "gpu.temperature.hotspot",
            TemperatureMemory => "gpu.temperature.memory",
            ClockCore => "gpu.clock.core",
            ClockMemory => "gpu.clock.memory",
            PowerBoard => "gpu.power.board",
            PowerLimit => "gpu.power.limit",
            PowerLimitPercent => "gpu.power.limitPercent",
            FanPercent => "gpu.fan.percent",
            FanRpm => "gpu.fan.rpm",
            VoltageCore => "gpu.voltage.core",
            ThrottlePower => "gpu.throttle.power",
            ThrottleThermal => "gpu.throttle.thermal",
            PcieLinkGen => "gpu.pcie.gen",
            PcieLinkWidth => "gpu.pcie.width",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::model::{Label, Sensor, Source};
    use std::collections::BTreeSet;

    #[test]
    fn all_follows_declaration_order_without_duplicates() {
        assert!(GpuField::ALL.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn sensor_ids_and_label_keys_are_unique() {
        let ids: BTreeSet<_> = GpuField::ALL
            .iter()
            .map(|f| format!("{}/{}", f.kind().as_str(), f.name()))
            .collect();
        assert_eq!(ids.len(), GpuField::ALL.len());
        let keys: BTreeSet<_> = GpuField::ALL.iter().map(|f| f.label_key()).collect();
        assert_eq!(keys.len(), GpuField::ALL.len());
        assert!(keys.iter().all(|k| k.starts_with("gpu.")));
    }

    #[test]
    fn table_matches_the_contract() {
        let row = |f: GpuField| (f.kind(), f.name(), f.unit(), f.label_key());
        assert_eq!(
            row(GpuField::LoadVideoDecode),
            (
                SensorKind::Load,
                "video-decode",
                Unit::Percent,
                "gpu.load.videoDecode"
            )
        );
        assert_eq!(
            row(GpuField::MemoryDedicatedTotal),
            (
                SensorKind::Data,
                "memory-dedicated-total",
                Unit::Bytes,
                "gpu.memory.dedicatedTotal"
            )
        );
        assert_eq!(
            row(GpuField::PowerLimitPercent),
            (
                SensorKind::Percent,
                "power-limit",
                Unit::Percent,
                "gpu.power.limitPercent"
            )
        );
        assert_eq!(
            row(GpuField::FanRpm),
            (SensorKind::Fan, "rpm", Unit::Rpm, "gpu.fan.rpm")
        );
        assert_eq!(
            row(GpuField::VoltageCore),
            (SensorKind::Voltage, "core", Unit::Volt, "gpu.voltage.core")
        );
        assert_eq!(
            row(GpuField::ThrottleThermal),
            (
                SensorKind::Flag,
                "throttle-thermal",
                Unit::Boolean,
                "gpu.throttle.thermal"
            )
        );
    }

    #[test]
    fn m3_fields_match_the_contract() {
        let row = |f: GpuField| (f.kind(), f.name(), f.unit(), f.label_key());
        assert_eq!(
            row(GpuField::LoadEncoder),
            (
                SensorKind::Load,
                "encoder",
                Unit::Percent,
                "gpu.load.encoder"
            )
        );
        assert_eq!(
            row(GpuField::LoadDecoder),
            (
                SensorKind::Load,
                "decoder",
                Unit::Percent,
                "gpu.load.decoder"
            )
        );
        assert_eq!(
            row(GpuField::PcieLinkGen),
            (
                SensorKind::Link,
                "pcie-gen",
                Unit::PcieGeneration,
                "gpu.pcie.gen"
            )
        );
        assert_eq!(
            row(GpuField::PcieLinkWidth),
            (
                SensorKind::Link,
                "pcie-width",
                Unit::Lanes,
                "gpu.pcie.width"
            )
        );
        // Encoder/decoder follow the PDH per-engine loads; the link comes last.
        let position = |f: GpuField| GpuField::ALL.iter().position(|&x| x == f).unwrap();
        assert_eq!(position(GpuField::LoadEncoder), 6);
        assert_eq!(position(GpuField::LoadDecoder), 7);
        assert_eq!(position(GpuField::PcieLinkGen), 24);
        assert_eq!(position(GpuField::PcieLinkWidth), 25);
    }

    #[test]
    fn sensor_id_matches_the_spec_example() {
        let f = GpuField::TemperatureHotspot;
        let s = Sensor::new(
            "gpu/pci-0000:01:00.0",
            f.kind(),
            f.name(),
            f.unit(),
            Label::new(f.label_key()),
            Source::Nvapi,
        );
        assert_eq!(s.id, "gpu/pci-0000:01:00.0/temperature/hotspot");
    }
}

/// `GpuField` is crate-private, so the integration test tests/labels.rs keeps
/// its own copy of the label keys: this check keeps that copy complete.
#[cfg(test)]
mod label_key_tests {
    use super::GpuField;

    #[test]
    fn every_label_key_is_checked_by_the_labels_test() {
        let labels_test = include_str!("../../tests/labels.rs");
        for field in GpuField::ALL {
            let quoted = format!("\"{}\",", field.label_key());
            assert!(
                labels_test.contains(&quoted),
                "tests/labels.rs KEYS lacks {}",
                field.label_key()
            );
        }
    }
}
