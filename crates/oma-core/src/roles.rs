//! Sensor roles: the well-known readings (GPU load, CPU temperature, ...) and
//! how to find their sensor in a schema. Shared by the tray and the built-in
//! overlay profiles, so both pick the same sensor on the same machine.

use crate::model::{DeviceKind, Schema};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    GpuLoad,
    GpuTemperature,
    GpuMemoryUsed,
    GpuClock,
    GpuPower,
    CpuLoad,
    CpuTemperature,
    CpuClock,
    CpuPower,
    RamUsed,
    RamLoad,
}

/// Sensor ids a role accepts for a device, most preferred first. GPU roles
/// have a suffix after the device id, the others a full id.
enum Candidates {
    Gpu(&'static str),
    Fixed(&'static [&'static str]),
}

impl Role {
    fn candidates(self) -> Candidates {
        match self {
            Role::GpuLoad => Candidates::Gpu("load/core"),
            Role::GpuTemperature => Candidates::Gpu("temperature/core"),
            Role::GpuMemoryUsed => Candidates::Gpu("data/memory-dedicated-used"),
            Role::GpuClock => Candidates::Gpu("clock/core"),
            Role::GpuPower => Candidates::Gpu("power/board"),
            Role::CpuLoad => Candidates::Fixed(&["cpu/0/load/total"]),
            Role::CpuTemperature => {
                Candidates::Fixed(&["cpu/0/temperature/package", "cpu/0/temperature/tctl"])
            }
            Role::CpuClock => Candidates::Fixed(&["cpu/0/clock/effective"]),
            Role::CpuPower => Candidates::Fixed(&["cpu/0/power/package"]),
            Role::RamUsed => Candidates::Fixed(&["memory/0/data/used"]),
            Role::RamLoad => Candidates::Fixed(&["memory/0/load/used"]),
        }
    }
}

fn find<'a>(schema: &'a Schema, id: &str) -> Option<&'a str> {
    schema
        .sensors
        .iter()
        .find(|sensor| sensor.id == id)
        .map(|sensor| sensor.id.as_str())
}

/// Id of the sensor that plays `role`, if the schema has one. GPU roles use
/// the first dedicated (non-integrated) GPU that has the sensor; a schema
/// with no dedicated GPU falls back to its GPUs in order.
pub fn role_sensor(schema: &Schema, role: Role) -> Option<&str> {
    match role.candidates() {
        Candidates::Fixed(ids) => ids.iter().find_map(|id| find(schema, id)),
        Candidates::Gpu(suffix) => {
            let gpus = || schema.devices.iter().filter(|d| d.kind == DeviceKind::Gpu);
            let dedicated = |d: &&crate::model::Device| {
                d.properties.get("integrated").map(String::as_str) != Some("true")
            };
            let pick = |id: &str| find(schema, &format!("{id}/{suffix}"));
            if gpus().any(|d| dedicated(&d)) {
                gpus().filter(dedicated).find_map(|d| pick(&d.id))
            } else {
                gpus().find_map(|d| pick(&d.id))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, Label, Sensor, SensorKind, Source, Unit};
    use std::collections::BTreeMap;

    pub(crate) fn this_machine() -> Schema {
        serde_json::from_str(include_str!("../tests/fixtures/this-machine-schema.json")).unwrap()
    }

    fn gpu(id: &str, integrated: bool) -> Device {
        Device {
            id: id.to_owned(),
            kind: DeviceKind::Gpu,
            name: id.to_owned(),
            vendor: None,
            properties: BTreeMap::from([("integrated".to_owned(), integrated.to_string())]),
        }
    }

    fn sensor(device: &str, kind: SensorKind, name: &str) -> Sensor {
        Sensor::new(
            device,
            kind,
            name,
            Unit::Percent,
            Label::new("t"),
            Source::Mock,
        )
    }

    #[test]
    fn roles_pick_first_dedicated_gpu() {
        let schema = this_machine();
        assert_eq!(
            role_sensor(&schema, Role::GpuLoad),
            Some("gpu/pci-0000:01:00.0/load/core")
        );
        assert_eq!(
            role_sensor(&schema, Role::GpuMemoryUsed),
            Some("gpu/pci-0000:01:00.0/data/memory-dedicated-used")
        );
        assert_eq!(
            role_sensor(&schema, Role::GpuPower),
            Some("gpu/pci-0000:01:00.0/power/board")
        );
        assert_eq!(
            role_sensor(&schema, Role::RamUsed),
            Some("memory/0/data/used")
        );
        assert_eq!(
            role_sensor(&schema, Role::CpuClock),
            Some("cpu/0/clock/effective")
        );
        assert_eq!(
            role_sensor(&schema, Role::CpuPower),
            Some("cpu/0/power/package")
        );
    }

    #[test]
    fn roles_fall_back_to_integrated_gpu_when_alone() {
        let schema = Schema {
            revision: 1,
            devices: vec![gpu("gpu/igpu", true)],
            sensors: vec![sensor("gpu/igpu", SensorKind::Load, "core")],
        };
        assert_eq!(
            role_sensor(&schema, Role::GpuLoad),
            Some("gpu/igpu/load/core")
        );
        assert_eq!(role_sensor(&schema, Role::GpuTemperature), None);
    }

    #[test]
    fn dedicated_gpu_without_the_sensor_does_not_borrow_the_integrated_one() {
        let schema = Schema {
            revision: 1,
            devices: vec![gpu("gpu/dgpu", false), gpu("gpu/igpu", true)],
            sensors: vec![sensor("gpu/igpu", SensorKind::Load, "core")],
        };
        assert_eq!(role_sensor(&schema, Role::GpuLoad), None);
    }

    #[test]
    fn cpu_temperature_prefers_package_then_tctl() {
        let mut schema = Schema {
            revision: 1,
            devices: vec![],
            sensors: vec![
                sensor("cpu/0", SensorKind::Temperature, "tctl"),
                sensor("cpu/0", SensorKind::Temperature, "package"),
            ],
        };
        assert_eq!(
            role_sensor(&schema, Role::CpuTemperature),
            Some("cpu/0/temperature/package")
        );
        schema.sensors.remove(1);
        assert_eq!(
            role_sensor(&schema, Role::CpuTemperature),
            Some("cpu/0/temperature/tctl")
        );
        schema.sensors.clear();
        assert_eq!(role_sensor(&schema, Role::CpuTemperature), None);
    }
}
