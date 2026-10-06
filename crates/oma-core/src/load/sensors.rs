//! CPU sensor pick and sampling for the stop guard and the summary (plan DA5).

use crate::model::{Schema, Snapshot};
use crate::provider::Quality;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CpuSensorIds {
    pub temp: Option<usize>,
    pub power: Option<usize>,
    pub clock: Option<usize>,
    pub core_clock: Vec<Option<usize>>,
    pub tjmax_c: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SensorSample {
    pub temp_c: Option<f64>,
    pub power_w: Option<f64>,
    pub clock_mhz: Option<f64>,
    pub core_clock_mhz: Vec<Option<f64>>,
}

/// Indices into `schema.sensors` (and so into snapshot values). Core `N`
/// maps to LibreHardwareMonitor's `core-<N+1>` clocks.
pub fn resolve_cpu_sensors(schema: &Schema, cores: usize) -> CpuSensorIds {
    let find = |id: &str| schema.sensors.iter().position(|s| s.id == id);
    let first = |ids: &[&str]| ids.iter().find_map(|id| find(id));
    CpuSensorIds {
        temp: first(&[
            "cpu/0/temperature/tdie",
            "cpu/0/temperature/tctl",
            "cpu/0/temperature/package",
            "cpu/0/temperature/core-max",
        ]),
        power: find("cpu/0/power/package"),
        clock: first(&[
            "cpu/0/clock/average-effective",
            "cpu/0/clock/average",
            "cpu/0/clock/effective",
        ]),
        core_clock: (1..=cores)
            .map(|n| {
                first(&[
                    &format!("cpu/0/clock/core-{n}-effective"),
                    &format!("cpu/0/clock/core-{n}"),
                ])
            })
            .collect(),
        tjmax_c: schema
            .devices
            .iter()
            .find(|d| d.id == "cpu/0")
            .and_then(|d| d.properties.get("tjMaxC"))
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite()),
    }
}

/// Only `Fresh` and `Held` readings count.
pub fn read_sample(ids: &CpuSensorIds, snapshot: &Snapshot, quality: &[Quality]) -> SensorSample {
    let get = |i: Option<usize>| {
        let i = i?;
        match quality.get(i)? {
            Quality::Fresh | Quality::Held => snapshot
                .values
                .get(i)
                .copied()
                .flatten()
                .filter(|v| v.is_finite()),
            _ => None,
        }
    };
    SensorSample {
        temp_c: get(ids.temp),
        power_w: get(ids.power),
        clock_mhz: get(ids.clock),
        core_clock_mhz: ids.core_clock.iter().map(|i| get(*i)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};

    fn schema(names: &[(&str, SensorKind)], tj: Option<&str>) -> Schema {
        let mut properties = std::collections::BTreeMap::new();
        if let Some(t) = tj {
            properties.insert("tjMaxC".to_string(), t.to_string());
        }
        Schema {
            revision: 1,
            devices: vec![Device {
                id: "cpu/0".into(),
                kind: DeviceKind::Cpu,
                name: "CPU".into(),
                vendor: None,
                properties,
            }],
            sensors: names
                .iter()
                .map(|(n, k)| {
                    Sensor::new("cpu/0", *k, n, Unit::Celsius, Label::new(n), Source::Lhm)
                })
                .collect(),
        }
    }

    #[test]
    fn resolves_amd_and_intel_names() {
        let text = include_str!("../../tests/fixtures/this-machine-schema.json");
        let s: Schema = serde_json::from_str(text).unwrap();
        let ids = resolve_cpu_sensors(&s, 8);
        let id = |i: Option<usize>| s.sensors[i.unwrap()].id.clone();
        assert_eq!(id(ids.temp), "cpu/0/temperature/tctl");
        assert_eq!(id(ids.power), "cpu/0/power/package");
        assert_eq!(id(ids.clock), "cpu/0/clock/average-effective");
        assert_eq!(id(ids.core_clock[0]), "cpu/0/clock/core-1-effective");
        assert_eq!(id(ids.core_clock[7]), "cpu/0/clock/core-8-effective");

        let intel = schema(
            &[
                ("package", SensorKind::Temperature),
                ("package", SensorKind::Power),
                ("core-1", SensorKind::Clock),
                ("average", SensorKind::Clock),
            ],
            Some("100"),
        );
        let ids = resolve_cpu_sensors(&intel, 2);
        assert_eq!(ids.temp, Some(0));
        assert_eq!(ids.power, Some(1));
        assert_eq!(ids.clock, Some(3));
        assert_eq!(ids.core_clock, vec![Some(2), None]);
        assert_eq!(ids.tjmax_c, Some(100.0));
    }

    #[test]
    fn suspended_quality_is_missing() {
        let s = schema(
            &[
                ("package", SensorKind::Temperature),
                ("package", SensorKind::Power),
            ],
            None,
        );
        let ids = resolve_cpu_sensors(&s, 0);
        let snap = Snapshot {
            revision: 1,
            seq: 1,
            timestamp_ms: 0,
            values: vec![Some(70.0), Some(50.0)],
        };
        let r = read_sample(&ids, &snap, &[Quality::Held, Quality::Suspended]);
        assert_eq!(r.temp_c, Some(70.0));
        assert_eq!(r.power_w, None);
        let r = read_sample(&ids, &snap, &[]);
        assert_eq!(r.temp_c, None);
    }
}
