//! Anonymous sensor report attached to bug reports (spec §3).
//!
//! `build_report` is pure: the shell gathers the input under its locks and
//! writes the file. Device ids that identify the machine (disk hashes,
//! network GUIDs) and network aliases never reach the output.

use serde_json::{json, Value};

use crate::model::{Schema, Snapshot};
use crate::provider::Quality;
use crate::settings::Sources;
use crate::stats::SensorStats;

/// Version of the report's JSON shape.
pub const REPORT_FORMAT: u32 = 1;

/// Device properties that may appear in the report; any other key is dropped.
pub const REPORT_PROPERTIES: &[&str] = &[
    "pciAddress",
    "integrated",
    "pcieMaxGen",
    "pcieMaxWidth",
    "powerLimitMinW",
    "powerLimitMaxW",
    "powerLimitDefaultW",
    "tempSlowdownC",
    "tempShutdownC",
    "tempMaxC",
    "tempWarningC",
    "tempCriticalC",
    "tjMaxC",
    "availableSpareThresholdPct",
    "adapterType",
];

/// Everything the report describes, gathered by the shell.
pub struct ReportInput<'a> {
    pub generated_at_ms: u64,
    pub app_version: &'a str,
    pub service_version: Option<&'a str>,
    pub protocol_version: u32,
    pub os_version: Option<&'a str>,
    /// camelCase serialization of the service link state.
    pub service_state: &'a str,
    pub anti_cheat: bool,
    pub safe_mode: bool,
    pub safe_mode_reason: Option<&'a str>,
    pub sources: &'a Sources,
    /// `(core device id, state)` of every disk.
    pub disk_states: &'a [(String, &'a str)],
    pub schema: &'a Schema,
    pub snapshot: Option<(&'a Snapshot, &'a [Quality])>,
    /// Aligned with `schema.sensors` when `stats_revision == schema.revision`.
    pub stats: &'a [Option<SensorStats>],
    pub stats_revision: u64,
}

/// Builds the report's JSON (spec §3.1): camelCase keys, values in internal
/// units, labels as i18n keys, identifiers replaced (spec §3.2).
pub fn build_report(input: &ReportInput) -> Value {
    let schema = input.schema;
    let mut anon = Anonymizer::new(input);

    let devices: Vec<Value> = schema
        .devices
        .iter()
        .map(|device| {
            let properties: serde_json::Map<String, Value> = device
                .properties
                .iter()
                .filter(|(key, _)| REPORT_PROPERTIES.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), json!(value)))
                .collect();
            json!({
                "id": anon.id(&device.id),
                "kind": device.kind,
                "name": anon.name(&device.name),
                "vendor": device.vendor.as_deref().map(|v| anon.text(v)),
                "properties": properties,
            })
        })
        .collect();

    let snapshot = input
        .snapshot
        .filter(|(snapshot, _)| snapshot.revision == schema.revision);
    let stats_valid = input.stats_revision == schema.revision;
    let sensors: Vec<Value> = schema
        .sensors
        .iter()
        .enumerate()
        .map(|(i, sensor)| {
            let value = snapshot.and_then(|(s, _)| s.values.get(i).copied().flatten());
            let quality = snapshot
                .and_then(|(_, q)| q.get(i).copied())
                .unwrap_or(Quality::Fresh);
            let stats = stats_valid
                .then(|| input.stats.get(i).copied().flatten())
                .flatten();
            json!({
                "id": anon.id(&sensor.id),
                "deviceId": anon.id(&sensor.device_id),
                "kind": sensor.kind,
                "unit": sensor.unit,
                "label": {
                    "key": sensor.label.key,
                    "arg": sensor.label.arg.as_deref().map(|arg| anon.name(arg)),
                },
                "source": sensor.source,
                "category": sensor.category,
                "experimental": sensor.experimental,
                "value": number(value),
                "quality": quality_name(quality),
                "stats": stats.map(|s| json!({
                    "min": number(Some(s.min)),
                    "avg": number(Some(s.avg)),
                    "max": number(Some(s.max)),
                    "count": s.count,
                })),
            })
        })
        .collect();

    let disks: Vec<Value> = input
        .disk_states
        .iter()
        .map(|(id, state)| json!({ "deviceId": anon.id(id), "state": state }))
        .collect();

    let sources = input.sources;
    json!({
        "format": REPORT_FORMAT,
        "generatedAt": utc_iso8601(input.generated_at_ms),
        "app": {
            "version": input.app_version,
            "serviceVersion": input.service_version,
            "protocolVersion": input.protocol_version,
        },
        "os": { "version": input.os_version },
        "state": {
            "service": { "state": input.service_state, "antiCheat": input.anti_cheat },
            "safeMode": { "active": input.safe_mode, "reason": input.safe_mode_reason },
            // The SMART drive lists are left out: they hold original disk ids.
            "sources": {
                "vendorLibraries": {
                    "nvml": sources.vendor_libraries.nvml,
                    "nvapi": sources.vendor_libraries.nvapi,
                    "adl": sources.vendor_libraries.adl,
                    "igcl": sources.vendor_libraries.igcl,
                },
                "antiCheat": sources.anti_cheat,
                "serviceModules": {
                    "cpu": sources.service_modules.cpu,
                    "motherboard": sources.service_modules.motherboard,
                    "memory": sources.service_modules.memory,
                    "storage": sources.service_modules.storage,
                    "controller": sources.service_modules.controller,
                    "psu": sources.service_modules.psu,
                },
            },
            "disks": disks,
        },
        "devices": devices,
        "sensors": sensors,
    })
}

/// A finite number, or `null`.
fn number(value: Option<f64>) -> Value {
    value
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

fn quality_name(quality: Quality) -> &'static str {
    match quality {
        Quality::Fresh => "fresh",
        Quality::Held => "held",
        Quality::Suspended => "suspended",
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
fn utc_iso8601(unix_ms: u64) -> String {
    let t = crate::csv::local_time(unix_ms, 0);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

const STORAGE_PREFIX: &str = "storage/";
const NETWORK_PREFIX: &str = "network/";

/// Replaces the identifying parts of the report's strings, once per report,
/// so that every occurrence of an id gets the same replacement.
struct Anonymizer {
    /// Original device id -> new id, for `storage/` and `network/` ids.
    ids: Vec<(String, String)>,
    /// Original network adapter name (the user's alias) -> new name.
    names: Vec<(String, String)>,
    /// GUIDs met in other strings (e.g. volume GUIDs in sensor names),
    /// lowercase, numbered in order of appearance.
    guids: Vec<String>,
}

impl Anonymizer {
    /// Numbers the ids in schema order: devices, then device ids only
    /// sensors carry, then disks only the disk states know.
    fn new(input: &ReportInput) -> Self {
        let schema = input.schema;
        let mut ids: Vec<(String, String)> = Vec::new();
        let (mut disks, mut adapters) = (0, 0);
        let candidates = schema
            .devices
            .iter()
            .map(|d| d.id.as_str())
            .chain(schema.sensors.iter().map(|s| s.device_id.as_str()))
            .chain(input.disk_states.iter().map(|(id, _)| id.as_str()));
        for id in candidates {
            if ids.iter().any(|(original, _)| original == id) {
                continue;
            }
            let new = if id.starts_with(STORAGE_PREFIX) {
                disks += 1;
                format!("{STORAGE_PREFIX}disk-{disks}")
            } else if id.starts_with(NETWORK_PREFIX) {
                adapters += 1;
                format!("{NETWORK_PREFIX}adapter-{adapters}")
            } else {
                continue;
            };
            ids.push((id.to_owned(), new));
        }

        let mut names: Vec<(String, String)> = Vec::new();
        let (mut ethernet, mut wifi, mut other) = (0, 0, 0);
        for device in &schema.devices {
            if !device.id.starts_with(NETWORK_PREFIX)
                || names.iter().any(|(original, _)| *original == device.name)
            {
                continue;
            }
            let name = match device.properties.get("adapterType").map(String::as_str) {
                Some("ethernet") => {
                    ethernet += 1;
                    format!("Ethernet {ethernet}")
                }
                Some("wifi") => {
                    wifi += 1;
                    format!("Wi-Fi {wifi}")
                }
                _ => {
                    other += 1;
                    format!("Adapter {other}")
                }
            };
            names.push((device.name.clone(), name));
        }
        Self {
            ids,
            names,
            guids: Vec::new(),
        }
    }

    /// An id, or an id followed by `/...` (a sensor id), with its device
    /// part replaced; GUIDs left in the rest are numbered.
    fn id(&mut self, text: &str) -> String {
        let rewritten = self.ids.iter().find_map(|(original, new)| {
            let rest = text.strip_prefix(original.as_str())?;
            (rest.is_empty() || rest.starts_with('/')).then(|| format!("{new}{rest}"))
        });
        match rewritten {
            Some(text) => self.text(&text),
            None => self.text(text),
        }
    }

    /// A device name or label argument: a network alias becomes the
    /// adapter's new name, anything else is treated as a possible id.
    fn name(&mut self, text: &str) -> String {
        match self.names.iter().find(|(original, _)| original == text) {
            Some((_, new)) => new.clone(),
            None => self.id(text),
        }
    }

    /// Free text with every GUID (and its braces) replaced by its number.
    fn text(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = find_guid(rest) {
            let end = start + GUID_LEN;
            let guid = rest[start..end].to_ascii_lowercase();
            let bytes = rest.as_bytes();
            let braced = start > 0 && bytes[start - 1] == b'{' && bytes.get(end) == Some(&b'}');
            let (cut_start, cut_end) = if braced {
                (start - 1, end + 1)
            } else {
                (start, end)
            };
            let n = match self.guids.iter().position(|g| *g == guid) {
                Some(i) => i + 1,
                None => {
                    self.guids.push(guid);
                    self.guids.len()
                }
            };
            out.push_str(&rest[..cut_start]);
            out.push_str(&n.to_string());
            rest = &rest[cut_end..];
        }
        out.push_str(rest);
        out
    }
}

/// Length of a GUID without braces: `8-4-4-4-12` hex digits.
const GUID_LEN: usize = 36;

/// Byte offset of the first GUID in `text`.
fn find_guid(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let last = bytes.len().checked_sub(GUID_LEN)?;
    (0..=last).find(|&start| {
        bytes[start..start + GUID_LEN]
            .iter()
            .enumerate()
            .all(|(i, b)| match i {
                8 | 13 | 18 | 23 => *b == b'-',
                _ => b.is_ascii_hexdigit(),
            })
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};

    const HASH_1: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    const HASH_2: &str = "60303ae22b998861bce3b28f33eec1be758a213c86c93c076dbe9f558c11c752";
    const GUID_1: &str = "6B29FC40-CA47-1067-B31D-00DD010662DA";
    const GUID_2: &str = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
    const VOLUME_GUID: &str = "a1b2c3d4-0000-1111-2222-333344445555";
    const ALIAS_1: &str = "VPN ufficio";
    const ALIAS_2: &str = "Casa di Mario";
    const SERIAL: &str = "SN-INVENTED-0042";

    fn props(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn device(id: &str, kind: DeviceKind, name: &str, p: &[(&str, &str)]) -> Device {
        Device {
            id: id.into(),
            kind,
            name: name.into(),
            vendor: None,
            properties: props(p),
        }
    }

    fn disk_1() -> String {
        format!("storage/device-{HASH_1}")
    }
    fn disk_2() -> String {
        format!("storage/device-{HASH_2}")
    }
    fn net_1() -> String {
        format!("network/{{{GUID_1}}}")
    }
    fn net_2() -> String {
        format!("network/{{{GUID_2}}}")
    }

    fn sensor(dev: &str, kind: SensorKind, name: &str, unit: Unit, label: Label) -> Sensor {
        Sensor::new(dev, kind, name, unit, label, Source::Win32)
    }

    /// CPU, GPU, LHM board, two disks and two network adapters.
    fn schema() -> Schema {
        let mut cpu = device("cpu/0", DeviceKind::Cpu, "AMD Ryzen 7", &[("tjMaxC", "95")]);
        cpu.vendor = Some("AMD".into());
        Schema {
            revision: 4,
            devices: vec![
                cpu,
                device(
                    "gpu/pci-0000:01:00.0",
                    DeviceKind::Gpu,
                    "NVIDIA GeForce RTX 4080",
                    &[
                        ("pciAddress", "0000:01:00.0"),
                        ("integrated", "false"),
                        ("serialNumber", SERIAL),
                    ],
                ),
                device("lhm/motherboard/0", DeviceKind::Motherboard, "Board", &[]),
                device(&disk_1(), DeviceKind::Storage, "Samsung SSD 980", &[]),
                device(&disk_2(), DeviceKind::Storage, "WDC WD40", &[]),
                device(
                    &net_1(),
                    DeviceKind::Network,
                    ALIAS_1,
                    &[("adapterType", "ethernet")],
                ),
                device(
                    &net_2(),
                    DeviceKind::Network,
                    ALIAS_2,
                    &[("adapterType", "wifi")],
                ),
            ],
            sensors: vec![
                sensor(
                    "cpu/0",
                    SensorKind::Temperature,
                    "package",
                    Unit::Celsius,
                    Label::new("cpu.temperature.package"),
                ),
                sensor(
                    "gpu/pci-0000:01:00.0",
                    SensorKind::Load,
                    "core",
                    Unit::Percent,
                    Label::new("gpu.load.core"),
                )
                .experimental(),
                sensor(
                    "lhm/motherboard/0",
                    SensorKind::Fan,
                    "1",
                    Unit::Rpm,
                    Label::with_arg("board.fan", "1"),
                ),
                sensor(
                    &disk_1(),
                    SensorKind::Temperature,
                    "drive",
                    Unit::Celsius,
                    Label::new("storage.temperature"),
                ),
                sensor(
                    &disk_1(),
                    SensorKind::Percent,
                    &format!("volume-{VOLUME_GUID}"),
                    Unit::Percent,
                    Label::with_arg("storage.volumeUsed", "C:"),
                ),
                sensor(
                    &disk_2(),
                    SensorKind::Throughput,
                    "read",
                    Unit::BytesPerSecond,
                    Label::new("storage.read"),
                ),
                sensor(
                    &net_1(),
                    SensorKind::Throughput,
                    "down",
                    Unit::BytesPerSecond,
                    Label::with_arg("network.down", ALIAS_1),
                ),
                sensor(
                    &net_2(),
                    SensorKind::Throughput,
                    "up",
                    Unit::BytesPerSecond,
                    Label::new("network.up"),
                ),
            ],
        }
    }

    fn st(min: f64, avg: f64, max: f64, count: u64) -> Option<SensorStats> {
        Some(SensorStats {
            min,
            max,
            avg,
            count,
        })
    }

    struct Fixture {
        schema: Schema,
        snapshot: Snapshot,
        quality: Vec<Quality>,
        stats: Vec<Option<SensorStats>>,
        sources: Sources,
        disks: Vec<(String, &'static str)>,
    }

    fn fixture() -> Fixture {
        let schema = schema();
        let mut sources = Sources::default();
        sources.vendor_libraries.adl = false;
        sources.service_modules.psu = false;
        sources.smart_disabled_drives = vec![disk_2()];
        sources.smart_enabled_drives = vec![disk_1()];
        Fixture {
            snapshot: Snapshot {
                revision: schema.revision,
                seq: 9,
                timestamp_ms: 1_000,
                values: vec![
                    Some(55.5),
                    Some(f64::NAN),
                    Some(900.0),
                    Some(41.0),
                    Some(63.2),
                    Some(f64::INFINITY),
                    Some(1_024.0),
                    None,
                ],
            },
            quality: vec![
                Quality::Fresh,
                Quality::Fresh,
                Quality::Held,
                Quality::Suspended,
                Quality::Fresh,
                Quality::Fresh,
                Quality::Fresh,
                Quality::Fresh,
            ],
            stats: vec![
                st(40.0, 50.0, 60.0, 10),
                None,
                st(800.0, 850.0, 900.0, 10),
                st(35.0, 38.0, 41.0, 10),
                st(63.0, 63.1, 63.2, 10),
                st(0.0, 1.0, 2.0, 10),
                st(0.0, 512.0, 1_024.0, 10),
                None,
            ],
            schema,
            sources,
            disks: vec![(disk_1(), "active"), (disk_2(), "standby")],
        }
    }

    fn input(f: &Fixture) -> ReportInput<'_> {
        ReportInput {
            generated_at_ms: 1_759_536_000_000,
            app_version: "0.4.0",
            service_version: Some("0.4.0"),
            protocol_version: 3,
            os_version: Some("10.0.26300"),
            service_state: "connected",
            anti_cheat: false,
            safe_mode: true,
            safe_mode_reason: Some("crash"),
            sources: &f.sources,
            disk_states: &f.disks,
            schema: &f.schema,
            snapshot: Some((&f.snapshot, &f.quality)),
            stats: &f.stats,
            stats_revision: f.schema.revision,
        }
    }

    fn sensor_ids(report: &Value) -> Vec<String> {
        report["sensors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn report_has_format_versions_and_state() {
        let f = fixture();
        let report = build_report(&input(&f));
        assert_eq!(report["format"], json!(1));
        assert_eq!(report["generatedAt"], json!("2025-10-04T00:00:00Z"));
        assert_eq!(
            report["app"],
            json!({"version": "0.4.0", "serviceVersion": "0.4.0", "protocolVersion": 3})
        );
        assert_eq!(report["os"], json!({"version": "10.0.26300"}));
        assert_eq!(
            report["state"]["service"],
            json!({"state": "connected", "antiCheat": false})
        );
        assert_eq!(
            report["state"]["safeMode"],
            json!({"active": true, "reason": "crash"})
        );

        let mut at_epoch = input(&f);
        at_epoch.generated_at_ms = 0;
        at_epoch.service_version = None;
        at_epoch.os_version = None;
        at_epoch.safe_mode = false;
        at_epoch.safe_mode_reason = None;
        let report = build_report(&at_epoch);
        assert_eq!(report["generatedAt"], json!("1970-01-01T00:00:00Z"));
        assert_eq!(report["app"]["serviceVersion"], Value::Null);
        assert_eq!(report["os"], json!({"version": null}));
        assert_eq!(
            report["state"]["safeMode"],
            json!({"active": false, "reason": null})
        );
    }

    #[test]
    fn report_sources_exclude_drive_lists() {
        let f = fixture();
        let sources = &build_report(&input(&f))["state"]["sources"];
        assert_eq!(
            sources,
            &json!({
                "vendorLibraries": {"nvml": true, "nvapi": true, "adl": false, "igcl": true},
                "antiCheat": false,
                "serviceModules": {
                    "cpu": true, "motherboard": true, "memory": true,
                    "storage": true, "controller": true, "psu": false
                }
            })
        );
        assert!(sources.get("smartDisabledDrives").is_none());
        assert!(sources.get("smartEnabledDrives").is_none());
    }

    #[test]
    fn sensors_carry_value_quality_and_stats() {
        let f = fixture();
        let report = build_report(&input(&f));
        let sensors = report["sensors"].as_array().unwrap();
        assert_eq!(sensors.len(), 8);
        assert_eq!(
            sensors[0],
            json!({
                "id": "cpu/0/temperature/package",
                "deviceId": "cpu/0",
                "kind": "temperature",
                "unit": "celsius",
                "label": {"key": "cpu.temperature.package", "arg": null},
                "source": "win32",
                "category": "temperature",
                "experimental": false,
                "value": 55.5,
                "quality": "fresh",
                "stats": {"min": 40.0, "avg": 50.0, "max": 60.0, "count": 10}
            })
        );
        // NaN and infinity become null, missing stats too.
        assert_eq!(sensors[1]["value"], Value::Null);
        assert_eq!(sensors[1]["experimental"], json!(true));
        assert_eq!(sensors[1]["stats"], Value::Null);
        assert_eq!(sensors[5]["value"], Value::Null);
        assert_eq!(sensors[7]["value"], Value::Null);
        assert_eq!(sensors[2]["quality"], json!("held"));
        assert_eq!(sensors[3]["quality"], json!("suspended"));
        assert_eq!(sensors[3]["value"], json!(41.0));

        let mut without = input(&f);
        without.snapshot = None;
        let report = build_report(&without);
        for s in report["sensors"].as_array().unwrap() {
            assert_eq!(s["value"], Value::Null);
            assert_eq!(s["quality"], json!("fresh"));
        }
        assert_eq!(report["sensors"][0]["stats"]["count"], json!(10));
    }

    #[test]
    fn non_finite_stats_become_null() {
        let mut f = fixture();
        f.stats[0] = st(f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 3);
        let report = build_report(&input(&f));
        assert_eq!(
            report["sensors"][0]["stats"],
            json!({"min": null, "avg": null, "max": null, "count": 3})
        );
    }

    #[test]
    fn mismatched_revision_gives_null_values() {
        let mut f = fixture();
        f.snapshot.revision = f.schema.revision + 1;
        let mut i = input(&f);
        i.stats_revision = f.schema.revision - 1;
        let report = build_report(&i);
        for s in report["sensors"].as_array().unwrap() {
            assert_eq!(s["value"], Value::Null, "{}", s["id"]);
            assert_eq!(s["quality"], json!("fresh"), "{}", s["id"]);
            assert_eq!(s["stats"], Value::Null, "{}", s["id"]);
        }
    }

    #[test]
    fn misaligned_lengths_give_null_instead_of_panicking() {
        let mut f = fixture();
        f.snapshot.values.truncate(2);
        f.quality.truncate(1);
        f.stats.truncate(1);
        let report = build_report(&input(&f));
        let sensors = report["sensors"].as_array().unwrap();
        assert_eq!(sensors[0]["value"], json!(55.5));
        assert_eq!(sensors[4]["value"], Value::Null);
        assert_eq!(sensors[4]["quality"], json!("fresh"));
        assert_eq!(sensors[4]["stats"], Value::Null);
    }

    #[test]
    fn ids_are_replaced_consistently() {
        let f = fixture();
        let report = build_report(&input(&f));
        let devices: Vec<&str> = report["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            devices,
            vec![
                "cpu/0",
                "gpu/pci-0000:01:00.0",
                "lhm/motherboard/0",
                "storage/disk-1",
                "storage/disk-2",
                "network/adapter-1",
                "network/adapter-2",
            ]
        );
        assert_eq!(
            sensor_ids(&report),
            vec![
                "cpu/0/temperature/package",
                "gpu/pci-0000:01:00.0/load/core",
                "lhm/motherboard/0/fan/1",
                "storage/disk-1/temperature/drive",
                "storage/disk-1/percent/volume-1",
                "storage/disk-2/throughput/read",
                "network/adapter-1/throughput/down",
                "network/adapter-2/throughput/up",
            ]
        );
        for s in report["sensors"].as_array().unwrap() {
            let id = s["id"].as_str().unwrap();
            let device = s["deviceId"].as_str().unwrap();
            assert!(id.starts_with(&format!("{device}/")), "{id} vs {device}");
            assert!(devices.contains(&device), "{device}");
        }
        assert_eq!(
            report["state"]["disks"],
            json!([
                {"deviceId": "storage/disk-1", "state": "active"},
                {"deviceId": "storage/disk-2", "state": "standby"}
            ])
        );
    }

    #[test]
    fn network_names_become_type_and_index() {
        let mut f = fixture();
        f.schema.devices.push(device(
            "network/{0F0F0F0F-1111-2222-3333-444455556666}",
            DeviceKind::Network,
            "Ponte di Mario",
            &[],
        ));
        f.schema.devices.push(device(
            "network/{0E0E0E0E-1111-2222-3333-444455556666}",
            DeviceKind::Network,
            "Cavo 2",
            &[("adapterType", "ethernet")],
        ));
        let report = build_report(&input(&f));
        let names: Vec<(&str, &str)> = report["devices"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["kind"] == "network")
            .map(|d| (d["id"].as_str().unwrap(), d["name"].as_str().unwrap()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("network/adapter-1", "Ethernet 1"),
                ("network/adapter-2", "Wi-Fi 1"),
                ("network/adapter-3", "Adapter 1"),
                ("network/adapter-4", "Ethernet 2"),
            ]
        );
        // A label argument equal to an alias takes the new name.
        assert_eq!(
            report["sensors"][6]["label"],
            json!({"key": "network.down", "arg": "Ethernet 1"})
        );
        // Other names stay.
        assert_eq!(report["devices"][3]["name"], json!("Samsung SSD 980"));
    }

    #[test]
    fn properties_are_whitelisted() {
        let f = fixture();
        let report = build_report(&input(&f));
        let gpu = &report["devices"][1];
        assert_eq!(
            gpu["properties"],
            json!({"pciAddress": "0000:01:00.0", "integrated": "false"})
        );
        assert_eq!(
            report["devices"][5]["properties"],
            json!({"adapterType": "ethernet"})
        );
        assert_eq!(report["devices"][0]["vendor"], json!("AMD"));
        assert_eq!(report["devices"][0]["kind"], json!("cpu"));
        assert_eq!(report["devices"][2]["properties"], json!({}));
        assert_eq!(report["devices"][2]["vendor"], Value::Null);
    }

    #[test]
    fn no_original_identifier_anywhere() {
        let mut f = fixture();
        // A disk the schema no longer lists still gets a new id.
        let gone =
            "storage/device-ffffeeeeddddccccbbbbaaaa99998888777766665555444433332222111100ff";
        f.disks.push((gone.to_owned(), "unknown"));
        let text = serde_json::to_string(&build_report(&input(&f)))
            .unwrap()
            .to_lowercase();
        for secret in [
            HASH_1,
            HASH_2,
            GUID_1,
            GUID_2,
            VOLUME_GUID,
            ALIAS_1,
            ALIAS_2,
            SERIAL,
            "ffffeeeeddddcccc",
            "device-",
        ] {
            assert!(
                !text.contains(&secret.to_lowercase()),
                "{secret} leaked in {text}"
            );
        }
        assert!(text.contains("storage/disk-3"));
    }
}
