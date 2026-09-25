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
}

impl GpuField {
    pub const ALL: [GpuField; 22] = [
        GpuField::LoadCore,
        GpuField::Load3d,
        GpuField::LoadCompute,
        GpuField::LoadCopy,
        GpuField::LoadVideoDecode,
        GpuField::LoadVideoEncode,
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
    ];

    pub fn kind(self) -> SensorKind {
        use GpuField::*;
        match self {
            LoadCore | Load3d | LoadCompute | LoadCopy | LoadVideoDecode | LoadVideoEncode => {
                SensorKind::Load
            }
            MemoryDedicatedUsed | MemoryDedicatedTotal | MemorySharedUsed => SensorKind::Data,
            TemperatureCore | TemperatureHotspot | TemperatureMemory => SensorKind::Temperature,
            ClockCore | ClockMemory => SensorKind::Clock,
            PowerBoard | PowerLimit => SensorKind::Power,
            PowerLimitPercent => SensorKind::Percent,
            FanPercent | FanRpm => SensorKind::Fan,
            VoltageCore => SensorKind::Voltage,
            ThrottlePower | ThrottleThermal => SensorKind::Flag,
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
        }
    }

    pub fn unit(self) -> Unit {
        use GpuField::*;
        match self {
            LoadCore | Load3d | LoadCompute | LoadCopy | LoadVideoDecode | LoadVideoEncode
            | PowerLimitPercent | FanPercent => Unit::Percent,
            MemoryDedicatedUsed | MemoryDedicatedTotal | MemorySharedUsed => Unit::Bytes,
            TemperatureCore | TemperatureHotspot | TemperatureMemory => Unit::Celsius,
            ClockCore | ClockMemory => Unit::Megahertz,
            PowerBoard | PowerLimit => Unit::Watt,
            FanRpm => Unit::Rpm,
            VoltageCore => Unit::Volt,
            ThrottlePower | ThrottleThermal => Unit::Boolean,
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
