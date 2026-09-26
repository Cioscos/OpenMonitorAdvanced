//! Data model shared by providers, the engine and the UI.

use serde::{Deserialize, Serialize};

/// Kind of hardware component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Cpu,
    Gpu,
    Memory,
    Storage,
    Network,
    Motherboard,
    Battery,
    FanController,
    Psu,
}

/// What a sensor measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    Temperature,
    Load,
    Clock,
    Power,
    Voltage,
    Current,
    Fan,
    Data,
    Throughput,
    Energy,
    Flag,
    Percent,
    /// A bus link, e.g. the PCIe link of a GPU.
    Link,
    /// A cumulative count, e.g. a disk's power-on cycles.
    Counter,
}

impl SensorKind {
    /// Segment used in sensor ids (`<device>/<kind>/<name>`).
    pub fn as_str(self) -> &'static str {
        match self {
            SensorKind::Temperature => "temperature",
            SensorKind::Load => "load",
            SensorKind::Clock => "clock",
            SensorKind::Power => "power",
            SensorKind::Voltage => "voltage",
            SensorKind::Current => "current",
            SensorKind::Fan => "fan",
            SensorKind::Data => "data",
            SensorKind::Throughput => "throughput",
            SensorKind::Energy => "energy",
            SensorKind::Flag => "flag",
            SensorKind::Percent => "percent",
            SensorKind::Link => "link",
            SensorKind::Counter => "counter",
        }
    }
}

/// Unit of a sensor value. Values are stored in these base units; the UI
/// converts for display (e.g. bytes/s to bit/s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Celsius,
    Percent,
    Megahertz,
    Watt,
    Volt,
    Ampere,
    Rpm,
    Bytes,
    BytesPerSecond,
    BitsPerSecond,
    Joule,
    Boolean,
    /// PCIe link generation (1 = 2.5 GT/s ... 5 = 32 GT/s).
    PcieGeneration,
    /// Number of active link lanes.
    Lanes,
    /// Power-on hours of a storage device.
    Hours,
    /// A whole-number count, e.g. power-on cycles.
    Count,
}

/// Where a reading comes from; shown as a badge in the Advanced view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Pdh,
    Win32,
    IpHelper,
    Dxgi,
    D3dkmt,
    Nvml,
    Nvapi,
    Adl,
    Igcl,
    /// Windows Plug and Play device properties (cfgmgr32).
    Pnp,
    Mock,
    /// LibreHardwareMonitor, read by the privileged service (spec §M4).
    Lhm,
}

/// Translatable label. The UI looks up `sensor.<key>` in its catalogs and
/// substitutes `{arg}` (e.g. a thread index or a drive letter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Label {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arg: Option<String>,
}

impl Label {
    pub fn new(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            arg: None,
        }
    }

    pub fn with_arg(key: &str, arg: impl Into<String>) -> Self {
        Self {
            key: key.to_owned(),
            arg: Some(arg.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub kind: DeviceKind,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub properties: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sensor {
    pub id: String,
    pub device_id: String,
    pub kind: SensorKind,
    pub unit: Unit,
    pub label: Label,
    pub source: Source,
    pub category: String,
    /// Read through an undocumented or unverified interface (spec §5.2): the
    /// UI marks it. Serialized only when `true`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub experimental: bool,
}

impl Sensor {
    /// Builds a sensor whose stable id is `<device_id>/<kind>/<name>`.
    pub fn new(
        device_id: &str,
        kind: SensorKind,
        name: &str,
        unit: Unit,
        label: Label,
        source: Source,
    ) -> Self {
        Self {
            id: format!("{device_id}/{}/{name}", kind.as_str()),
            device_id: device_id.to_owned(),
            kind,
            unit,
            label,
            source,
            category: kind.as_str().to_owned(),
            experimental: false,
        }
    }

    /// Marks the sensor as experimental.
    pub fn experimental(mut self) -> Self {
        self.experimental = true;
        self
    }
}

/// Every device and sensor currently known. `revision` changes whenever the
/// hardware set changes; snapshot values are indexed by `sensors` order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Schema {
    pub revision: u64,
    pub devices: Vec<Device>,
    pub sensors: Vec<Sensor>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub sensor_id: String,
    pub value: Option<f64>,
    pub timestamp_ms: u64,
}

/// One sampling cycle. `values[i]` belongs to `schema.sensors[i]` of the
/// schema with the same `revision`; `None` means "not available".
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub seq: u64,
    pub timestamp_ms: u64,
    pub values: Vec<Option<f64>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sensor_id_is_device_kind_name() {
        let s = Sensor::new(
            "cpu/0",
            SensorKind::Load,
            "total",
            Unit::Percent,
            Label::new("cpu.load.total"),
            Source::Pdh,
        );
        assert_eq!(s.id, "cpu/0/load/total");
        assert_eq!(s.device_id, "cpu/0");
    }

    #[test]
    fn sensor_serializes_in_camel_case_with_snake_case_enums() {
        let s = Sensor::new(
            "network/abc",
            SensorKind::Throughput,
            "down",
            Unit::BytesPerSecond,
            Label::new("network.down"),
            Source::IpHelper,
        );
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            json!({
                "id": "network/abc/throughput/down",
                "deviceId": "network/abc",
                "kind": "throughput",
                "unit": "bytes_per_second",
                "label": { "key": "network.down" },
                "source": "ip_helper",
                "category": "throughput"
            })
        );
    }

    #[test]
    fn gpu_sources_serialize_in_snake_case() {
        let sources = [
            Source::Dxgi,
            Source::D3dkmt,
            Source::Nvml,
            Source::Nvapi,
            Source::Adl,
            Source::Igcl,
        ];
        assert_eq!(
            serde_json::to_value(sources).unwrap(),
            json!(["dxgi", "d3dkmt", "nvml", "nvapi", "adl", "igcl"])
        );
    }

    #[test]
    fn link_kind_units_and_pnp_source_serialize_in_snake_case() {
        assert_eq!(SensorKind::Link.as_str(), "link");
        assert_eq!(
            serde_json::to_value(SensorKind::Link).unwrap(),
            json!("link")
        );
        assert_eq!(
            serde_json::to_value([Unit::PcieGeneration, Unit::Lanes]).unwrap(),
            json!(["pcie_generation", "lanes"])
        );
        assert_eq!(serde_json::to_value(Source::Pnp).unwrap(), json!("pnp"));
    }

    #[test]
    fn experimental_is_serialized_only_when_true() {
        let s = Sensor::new(
            "gpu/pci-0000:01:00.0",
            SensorKind::Temperature,
            "hotspot",
            Unit::Celsius,
            Label::new("gpu.temperature.hotspot"),
            Source::Nvapi,
        );
        assert!(!s.experimental);
        assert!(serde_json::to_value(&s)
            .unwrap()
            .get("experimental")
            .is_none());
        let s = s.experimental();
        assert!(s.experimental);
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            json!({
                "id": "gpu/pci-0000:01:00.0/temperature/hotspot",
                "deviceId": "gpu/pci-0000:01:00.0",
                "kind": "temperature",
                "unit": "celsius",
                "label": { "key": "gpu.temperature.hotspot" },
                "source": "nvapi",
                "category": "temperature",
                "experimental": true
            })
        );
    }

    #[test]
    fn label_arg_is_serialized_when_present() {
        let label = Label::with_arg("cpu.load.thread", "3");
        assert_eq!(
            serde_json::to_value(&label).unwrap(),
            json!({ "key": "cpu.load.thread", "arg": "3" })
        );
    }

    #[test]
    fn device_kind_uses_snake_case() {
        let d = Device {
            id: "x".into(),
            kind: DeviceKind::FanController,
            name: "X".into(),
            vendor: None,
            properties: Default::default(),
        };
        assert_eq!(
            serde_json::to_value(&d).unwrap()["kind"],
            json!("fan_controller")
        );
    }

    #[test]
    fn lhm_source_serializes_as_lhm() {
        assert_eq!(serde_json::to_value(Source::Lhm).unwrap(), json!("lhm"));
        assert_eq!(serde_json::to_value(Unit::Hours).unwrap(), json!("hours"));
        assert_eq!(
            serde_json::to_value(SensorKind::Counter).unwrap(),
            json!("counter")
        );
    }

    #[test]
    fn snapshot_serializes_missing_values_as_null() {
        let snap = Snapshot {
            revision: 1,
            seq: 2,
            timestamp_ms: 3,
            values: vec![Some(1.5), None],
        };
        assert_eq!(
            serde_json::to_value(&snap).unwrap(),
            json!({ "revision": 1, "seq": 2, "timestampMs": 3, "values": [1.5, null] })
        );
    }
}
