//! Data model shared by providers, the engine and the UI.

use serde::Serialize;

/// Kind of hardware component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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
        }
    }
}

/// Unit of a sensor value. Values are stored in these base units; the UI
/// converts for display (e.g. bytes/s to bit/s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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
}

/// Where a reading comes from; shown as a badge in the Advanced view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Pdh,
    Win32,
    IpHelper,
    Mock,
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
        }
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
