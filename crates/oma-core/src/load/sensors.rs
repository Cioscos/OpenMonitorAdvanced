//! CPU (plan DA5) and GPU (plan DG12) sensor pick and sampling for the stop guard and the
//! summary.

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
    /// GPU only: a power or thermal throttle flag is on; `None` when no flag reads.
    pub throttling: Option<bool>,
    /// GPU only: the thermal throttle flag alone (the power one is normal under load).
    pub thermal_throttling: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuSensorIds {
    pub temp: Option<usize>,
    pub power: Option<usize>,
    pub clock: Option<usize>,
    pub throttle: Vec<usize>,
    pub thermal: Option<usize>,
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
            .filter(|v| v.is_finite() && *v > 0.0),
    }
}

/// Indices of the GPU `device_id`'s sensors (DG12).
pub fn resolve_gpu_sensors(schema: &Schema, device_id: &str) -> GpuSensorIds {
    let find = |rest: &str| {
        let id = format!("{device_id}/{rest}");
        schema.sensors.iter().position(|s| s.id == id)
    };
    GpuSensorIds {
        temp: find("temperature/core").or_else(|| find("temperature/hotspot")),
        power: find("power/board"),
        clock: find("clock/core"),
        throttle: ["flag/throttle-power", "flag/throttle-thermal"]
            .iter()
            .filter_map(|r| find(r))
            .collect(),
        thermal: find("flag/throttle-thermal"),
    }
}

/// The value at `i` if its quality is `Fresh` or `Held`.
fn value(snapshot: &Snapshot, quality: &[Quality], i: usize) -> Option<f64> {
    match quality.get(i)? {
        Quality::Fresh | Quality::Held => snapshot
            .values
            .get(i)
            .copied()
            .flatten()
            .filter(|v| v.is_finite()),
        _ => None,
    }
}

/// Only `Fresh` and `Held` readings count.
pub fn read_sample(ids: &CpuSensorIds, snapshot: &Snapshot, quality: &[Quality]) -> SensorSample {
    let get = |i: Option<usize>| value(snapshot, quality, i?);
    SensorSample {
        temp_c: get(ids.temp),
        power_w: get(ids.power),
        clock_mhz: get(ids.clock),
        core_clock_mhz: ids.core_clock.iter().map(|i| get(*i)).collect(),
        throttling: None,
        thermal_throttling: None,
    }
}

/// Only `Fresh` and `Held` readings count; a flag is on at 1.
pub fn read_gpu_sample(
    ids: &GpuSensorIds,
    snapshot: &Snapshot,
    quality: &[Quality],
) -> SensorSample {
    let get = |i: Option<usize>| value(snapshot, quality, i?);
    let flags: Vec<f64> = ids
        .throttle
        .iter()
        .filter_map(|i| value(snapshot, quality, *i))
        .collect();
    SensorSample {
        temp_c: get(ids.temp),
        power_w: get(ids.power),
        clock_mhz: get(ids.clock),
        core_clock_mhz: vec![],
        throttling: (!flags.is_empty()).then(|| flags.iter().any(|v| *v >= 1.0)),
        thermal_throttling: get(ids.thermal).map(|v| v >= 1.0),
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
    fn non_positive_or_non_finite_tjmax_is_ignored() {
        for t in ["0", "-5", "NaN", "inf"] {
            let s = schema(&[("package", SensorKind::Temperature)], Some(t));
            assert_eq!(resolve_cpu_sensors(&s, 0).tjmax_c, None, "{t}");
        }
    }

    const GPU: &str = "gpu/pci-0000:01:00.0";

    fn gpu_schema(names: &[(&str, SensorKind)]) -> Schema {
        Schema {
            revision: 1,
            devices: vec![],
            sensors: names
                .iter()
                .map(|(n, k)| Sensor::new(GPU, *k, n, Unit::Celsius, Label::new(n), Source::Lhm))
                .collect(),
        }
    }

    fn snap(values: Vec<Option<f64>>) -> Snapshot {
        Snapshot {
            revision: 1,
            seq: 1,
            timestamp_ms: 0,
            values,
        }
    }

    #[test]
    fn gpu_sensor_ids_follow_dg12() {
        let s = gpu_schema(&[
            ("hotspot", SensorKind::Temperature),
            ("core", SensorKind::Temperature),
            ("board", SensorKind::Power),
            ("core", SensorKind::Clock),
            ("throttle-power", SensorKind::Flag),
            ("throttle-thermal", SensorKind::Flag),
            ("memory", SensorKind::Clock),
        ]);
        let ids = resolve_gpu_sensors(&s, GPU);
        assert_eq!(
            ids,
            GpuSensorIds {
                temp: Some(1),
                power: Some(2),
                clock: Some(3),
                throttle: vec![4, 5],
                thermal: Some(5),
            }
        );
        // Another device's sensors never match.
        assert_eq!(
            resolve_gpu_sensors(&s, "gpu/pci-0000:02:00.0"),
            GpuSensorIds::default()
        );
    }

    #[test]
    fn gpu_temperature_falls_back_to_hotspot() {
        let s = gpu_schema(&[("hotspot", SensorKind::Temperature)]);
        assert_eq!(resolve_gpu_sensors(&s, GPU).temp, Some(0));
    }

    #[test]
    fn throttling_is_true_when_any_flag_is_one() {
        let s = gpu_schema(&[
            ("core", SensorKind::Temperature),
            ("throttle-power", SensorKind::Flag),
            ("throttle-thermal", SensorKind::Flag),
        ]);
        let ids = resolve_gpu_sensors(&s, GPU);
        let q = [Quality::Fresh; 3];
        let r = read_gpu_sample(&ids, &snap(vec![Some(70.0), Some(0.0), Some(1.0)]), &q);
        assert_eq!(r.temp_c, Some(70.0));
        assert_eq!(r.throttling, Some(true));
        let r = read_gpu_sample(&ids, &snap(vec![Some(70.0), Some(0.0), Some(0.0)]), &q);
        assert_eq!(r.throttling, Some(false));
        // No readable flag: unknown.
        let r = read_gpu_sample(&ids, &snap(vec![Some(70.0), None, None]), &q);
        assert_eq!(r.throttling, None);
        assert!(r.core_clock_mhz.is_empty());
    }

    #[test]
    fn thermal_flag_is_read_apart_from_power() {
        let s = gpu_schema(&[
            ("throttle-power", SensorKind::Flag),
            ("throttle-thermal", SensorKind::Flag),
        ]);
        let ids = resolve_gpu_sensors(&s, GPU);
        let r = read_gpu_sample(
            &ids,
            &snap(vec![Some(1.0), Some(0.0)]),
            &[Quality::Fresh; 2],
        );
        assert_eq!(r.throttling, Some(true));
        assert_eq!(r.thermal_throttling, Some(false));
    }

    #[test]
    fn cpu_samples_have_no_throttling() {
        let s = schema(&[("package", SensorKind::Temperature)], None);
        let ids = resolve_cpu_sensors(&s, 0);
        let r = read_sample(&ids, &snap(vec![Some(70.0)]), &[Quality::Fresh]);
        assert_eq!(r.throttling, None);
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
