//! Turns the service's wire schema/snapshot (`SvcFeed`, held by the link)
//! into core devices and sensors: `bind` is the pure mapping, `SvcProvider`
//! is the `Provider` that drives it from the feed and the storage
//! provider's `DriveIdTable` (spec §M4, D3).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use oma_ipc::{IdentityHint, WireSchema};
use serde::de::value::{Error as DeError, StrDeserializer};
use serde::Deserialize;

use crate::storage::{DriveEntry, DriveIdTable, DriveIds};
use crate::svc::feed::SvcFeed;

/// Parses a wire string (already snake_case, matching `oma_core::model`'s
/// `Serialize`) into `T`, using `T`'s own `Deserialize` impl. `None` for an
/// unrecognised value.
fn parse_wire<'de, T: Deserialize<'de>>(value: &'de str) -> Option<T> {
    T::deserialize(StrDeserializer::<DeError>::new(value)).ok()
}

/// A non-empty string, trimmed; `None` for missing, all-whitespace or absent
/// input. `None == None` is never a proof of identity (D3): callers must
/// require both sides of a comparison to produce `Some`.
fn trimmed(value: Option<&str>) -> Option<&str> {
    let text = value?.trim();
    (!text.is_empty()).then_some(text)
}

/// The disk in `drives` bound to `physical_drive`, if the wire hint's model
/// and serial both match the descriptor texts of that disk (D3) and that
/// model/serial pair is not shared by another disk in the table (an
/// ambiguous match binds nothing).
fn storage_binding<'a>(
    model: &Option<String>,
    serial: &Option<String>,
    physical_drive: u32,
    drives: &'a DriveIds,
) -> Option<&'a DriveEntry> {
    let hint_model = trimmed(model.as_deref())?;
    let hint_serial = trimmed(serial.as_deref())?;
    let entry = drives.drives.iter().find(|d| d.index == physical_drive)?;
    let drive_model = trimmed(entry.model.as_deref())?;
    let drive_serial = trimmed(entry.serial.as_deref())?;
    if hint_model != drive_model || hint_serial != drive_serial {
        return None;
    }
    let unique = drives
        .drives
        .iter()
        .filter(|d| {
            trimmed(d.model.as_deref()) == Some(drive_model)
                && trimmed(d.serial.as_deref()) == Some(drive_serial)
        })
        .count()
        == 1;
    unique.then_some(entry)
}

/// A schema device once its final core id and kind are known.
struct Bound {
    final_id: String,
    kind: DeviceKind,
}

/// Binds `schema`'s devices onto core ids and builds the resulting
/// inventory: `Cpu`/`Memory` hints map to the CPU/memory provider's device
/// ids, a `Storage` hint maps onto a disk of `drives` only when
/// [`storage_binding`] agrees, and every other device (or a hint that did
/// not match) gets `<kind>/<device id>`. A device or sensor whose kind/unit
/// is not one `oma_core::model` knows is skipped, with one log warning for
/// the whole schema. Returns the kept sensors' wire indices, in the same
/// order as `Inventory::sensors`, for `poll` to read the matching values.
///
/// Fails without publishing a partial inventory if binding produces two
/// devices with the same final id (e.g. two service devices bound onto the
/// same core disk): the link already rejects duplicate/invalid ids and
/// dangling sensor references before a schema reaches the feed, so this is
/// the last check specific to the binding step itself.
pub(crate) fn bind(
    schema: &WireSchema,
    drives: &DriveIds,
) -> Result<(Inventory, Vec<usize>), ProviderError> {
    let mut unknown = false;
    let mut bound: HashMap<&str, Bound> = HashMap::with_capacity(schema.devices.len());
    for device in &schema.devices {
        let Some(kind) = parse_wire::<DeviceKind>(&device.kind) else {
            unknown = true;
            continue;
        };
        let final_id = match &device.hint {
            Some(IdentityHint::Cpu { index }) => format!("cpu/{index}"),
            Some(IdentityHint::Memory {}) => "memory/0".to_owned(),
            Some(IdentityHint::Storage {
                physical_drive,
                model,
                serial,
            }) => match storage_binding(model, serial, *physical_drive, drives) {
                Some(entry) => entry.device_id.clone(),
                None => format!("{}/{}", device.kind, device.id),
            },
            None => format!("{}/{}", device.kind, device.id),
        };
        bound.insert(device.id.as_str(), Bound { final_id, kind });
    }

    let mut seen_ids = HashSet::with_capacity(bound.len());
    for b in bound.values() {
        if !seen_ids.insert(b.final_id.as_str()) {
            return Err(ProviderError::Failed(format!(
                "svc: two devices bound onto {:?}",
                b.final_id
            )));
        }
    }

    let mut devices = Vec::with_capacity(bound.len());
    for device in &schema.devices {
        let Some(b) = bound.get(device.id.as_str()) else {
            continue;
        };
        devices.push(Device {
            id: b.final_id.clone(),
            kind: b.kind,
            name: device.name.clone(),
            vendor: device.vendor.clone(),
            properties: device.properties.clone(),
        });
    }

    let mut sensors = Vec::new();
    let mut kept = Vec::new();
    for (index, sensor) in schema.sensors.iter().enumerate() {
        let Some(b) = bound.get(sensor.device_id.as_str()) else {
            continue;
        };
        let Some(kind) = parse_wire::<SensorKind>(&sensor.kind) else {
            unknown = true;
            continue;
        };
        let Some(unit) = parse_wire::<Unit>(&sensor.unit) else {
            unknown = true;
            continue;
        };
        let label = Label {
            key: sensor.label_key.clone(),
            arg: sensor.label_arg.clone(),
        };
        let mut s = Sensor::new(&b.final_id, kind, &sensor.name, unit, label, Source::Lhm);
        // An unrecognised category still needs a translated group: fall back
        // to the sensor's own kind rather than shipping an untranslated one.
        s.category = if parse_wire::<SensorKind>(&sensor.category).is_some() {
            sensor.category.clone()
        } else {
            kind.as_str().to_owned()
        };
        sensors.push(s);
        kept.push(index);
    }

    if unknown {
        tracing::warn!("svc schema: unknown device kind, sensor kind or unit skipped");
    }

    Ok((Inventory { devices, sensors }, kept))
}

/// `Provider` fed by the sensor service's schema/snapshot feed, binding its
/// devices onto core ids with the storage provider's drive table.
pub struct SvcProvider {
    feed: SvcFeed,
    drives: DriveIdTable,
    /// The feed/drive-table generations `discover` last bound to; `poll`
    /// requests a rediscovery when either has since changed.
    bound_generation: u64,
    bound_drives_generation: u64,
    /// Wire indices of the sensors kept by the last `discover`, in
    /// `Inventory::sensors` order.
    kept: Vec<usize>,
    interval: Duration,
}

impl SvcProvider {
    pub fn new(feed: SvcFeed, drives: DriveIdTable) -> Self {
        Self {
            feed,
            drives,
            bound_generation: 0,
            bound_drives_generation: 0,
            kept: Vec::new(),
            interval: Duration::default(),
        }
    }
}

impl Provider for SvcProvider {
    fn name(&self) -> &'static str {
        "svc"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        let view = self.feed.view();
        let drives = self.drives.get();
        self.bound_generation = view.generation;
        self.bound_drives_generation = drives.generation;
        self.interval = view.interval;
        let Some(schema) = view.schema else {
            self.kept = Vec::new();
            return Ok(Inventory::default());
        };
        let (inventory, kept) = bind(&schema, &drives)?;
        self.kept = kept;
        Ok(inventory)
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let view = self.feed.view();
        let drives = self.drives.get();
        if view.generation != self.bound_generation
            || drives.generation != self.bound_drives_generation
        {
            return Err(ProviderError::Rediscover);
        }
        let Some((received, snapshot)) = view.snapshot else {
            return Ok(vec![None; self.kept.len()]);
        };
        if received.elapsed() > view.interval * 3 {
            return Ok(vec![None; self.kept.len()]);
        }
        Ok(self
            .kept
            .iter()
            .map(|&i| snapshot.values.get(i).copied().flatten())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Instant;

    use oma_ipc::{WireDevice, WireSensor, WireSnapshot};

    use super::*;

    fn device(id: &str, kind: &str, hint: Option<IdentityHint>) -> WireDevice {
        WireDevice {
            id: id.to_owned(),
            kind: kind.to_owned(),
            name: id.to_owned(),
            vendor: None,
            properties: BTreeMap::new(),
            hint,
        }
    }

    fn sensor(device_id: &str, kind: &str, name: &str, unit: &str, category: &str) -> WireSensor {
        WireSensor {
            device_id: device_id.to_owned(),
            kind: kind.to_owned(),
            name: name.to_owned(),
            unit: unit.to_owned(),
            label_key: format!("{device_id}.{kind}.{name}"),
            label_arg: None,
            category: category.to_owned(),
        }
    }

    fn drive_table(entries: Vec<DriveEntry>) -> DriveIds {
        DriveIds {
            generation: 1,
            drives: entries,
        }
    }

    fn drive(index: u32, id: &str, model: Option<&str>, serial: Option<&str>) -> DriveEntry {
        DriveEntry {
            index,
            device_id: id.to_owned(),
            model: model.map(str::to_owned),
            serial: serial.map(str::to_owned),
        }
    }

    #[test]
    fn cpu_and_memory_hints_bind_to_core_ids() {
        let schema = WireSchema {
            devices: vec![
                device("cpu-hw", "cpu", Some(IdentityHint::Cpu { index: 0 })),
                device("mem-hw", "memory", Some(IdentityHint::Memory {})),
            ],
            sensors: vec![],
        };
        let (inventory, _) = bind(&schema, &DriveIds::default()).expect("bind");
        let ids: Vec<&str> = inventory.devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["cpu/0", "memory/0"]);
    }

    #[test]
    fn storage_hint_binds_only_when_model_and_serial_match() {
        let drives = drive_table(vec![
            drive(0, "storage/device-aaa", Some("WD Black"), Some("SN-1")),
            drive(1, "storage/device-bbb", Some("Other"), Some("SN-2")),
        ]);

        let matching = device(
            "svc-disk-0",
            "storage",
            Some(IdentityHint::Storage {
                physical_drive: 0,
                model: Some(" WD Black ".to_owned()),
                serial: Some(" SN-1 ".to_owned()),
            }),
        );
        let (inventory, _) = bind(
            &WireSchema {
                devices: vec![matching],
                sensors: vec![],
            },
            &drives,
        )
        .expect("bind");
        assert_eq!(inventory.devices[0].id, "storage/device-aaa");

        let cases = [
            // model differs
            IdentityHint::Storage {
                physical_drive: 0,
                model: Some("Different".to_owned()),
                serial: Some("SN-1".to_owned()),
            },
            // serial differs
            IdentityHint::Storage {
                physical_drive: 0,
                model: Some("WD Black".to_owned()),
                serial: Some("Different".to_owned()),
            },
            // empty serial
            IdentityHint::Storage {
                physical_drive: 0,
                model: Some("WD Black".to_owned()),
                serial: Some("   ".to_owned()),
            },
            // missing model
            IdentityHint::Storage {
                physical_drive: 0,
                model: None,
                serial: Some("SN-1".to_owned()),
            },
            // missing serial, disk also has none
            IdentityHint::Storage {
                physical_drive: 2,
                model: Some("Solo".to_owned()),
                serial: None,
            },
        ];
        for (i, hint) in cases.into_iter().enumerate() {
            let schema = WireSchema {
                devices: vec![device(
                    &format!("svc-disk-{i}"),
                    "storage",
                    Some(hint.clone()),
                )],
                sensors: vec![],
            };
            let (inventory, _) = bind(&schema, &drives).expect("bind");
            assert_eq!(
                inventory.devices[0].id,
                format!("storage/svc-disk-{i}"),
                "case {i}: not a fallback id"
            );
        }

        // Both sides missing (None == None) is never identity.
        let no_identity_drives = drives_with_no_identity();
        let schema = WireSchema {
            devices: vec![device(
                "svc-disk-none",
                "storage",
                Some(IdentityHint::Storage {
                    physical_drive: 5,
                    model: None,
                    serial: None,
                }),
            )],
            sensors: vec![],
        };
        let (inventory, _) = bind(&schema, &no_identity_drives).expect("bind");
        assert_eq!(inventory.devices[0].id, "storage/svc-disk-none");

        // Duplicated pair on the drive side is ambiguous: no binding.
        let ambiguous = drive_table(vec![
            drive(0, "storage/device-a", Some("Same"), Some("Same-SN")),
            drive(1, "storage/device-b", Some("Same"), Some("Same-SN")),
        ]);
        let schema = WireSchema {
            devices: vec![device(
                "svc-disk-amb",
                "storage",
                Some(IdentityHint::Storage {
                    physical_drive: 0,
                    model: Some("Same".to_owned()),
                    serial: Some("Same-SN".to_owned()),
                }),
            )],
            sensors: vec![],
        };
        let (inventory, _) = bind(&schema, &ambiguous).expect("bind");
        assert_eq!(inventory.devices[0].id, "storage/svc-disk-amb");
    }

    fn drives_with_no_identity() -> DriveIds {
        drive_table(vec![drive(5, "storage/device-x", None, None)])
    }

    #[test]
    fn unbound_devices_get_kind_slash_id() {
        let schema = WireSchema {
            devices: vec![device("mb-1", "motherboard", None)],
            sensors: vec![],
        };
        let (inventory, _) = bind(&schema, &DriveIds::default()).expect("bind");
        assert_eq!(inventory.devices[0].id, "motherboard/mb-1");
    }

    #[test]
    fn sensor_ids_labels_and_source() {
        let schema = WireSchema {
            devices: vec![device(
                "cpu-hw",
                "cpu",
                Some(IdentityHint::Cpu { index: 0 }),
            )],
            sensors: vec![WireSensor {
                device_id: "cpu-hw".to_owned(),
                kind: "temperature".to_owned(),
                name: "package".to_owned(),
                unit: "celsius".to_owned(),
                label_key: "cpu.temperature.package".to_owned(),
                label_arg: None,
                category: "temperature".to_owned(),
            }],
        };
        let (inventory, kept) = bind(&schema, &DriveIds::default()).expect("bind");
        let s = &inventory.sensors[0];
        assert_eq!(s.id, "cpu/0/temperature/package");
        assert_eq!(s.label, Label::new("cpu.temperature.package"));
        assert_eq!(s.source, Source::Lhm);
        assert_eq!(kept, vec![0]);

        let schema = WireSchema {
            devices: vec![device("mb-1", "motherboard", None)],
            sensors: vec![WireSensor {
                device_id: "mb-1".to_owned(),
                kind: "fan".to_owned(),
                name: "fan1".to_owned(),
                unit: "rpm".to_owned(),
                label_key: "lhm.raw".to_owned(),
                label_arg: Some("Fan #1".to_owned()),
                category: "fan".to_owned(),
            }],
        };
        let (inventory, _) = bind(&schema, &DriveIds::default()).expect("bind");
        assert_eq!(
            inventory.sensors[0].label,
            Label::with_arg("lhm.raw", "Fan #1")
        );
    }

    #[test]
    fn unknown_kind_or_unit_is_skipped() {
        let schema = WireSchema {
            devices: vec![device("mb-1", "motherboard", None)],
            sensors: vec![
                sensor("mb-1", "temperature", "a", "celsius", "temperature"),
                sensor("mb-1", "quantum-flux", "b", "celsius", "temperature"),
                sensor("mb-1", "temperature", "c", "a-made-up-unit", "temperature"),
                sensor("mb-1", "temperature", "d", "celsius", "temperature"),
            ],
        };
        let (inventory, kept) = bind(&schema, &DriveIds::default()).expect("bind");
        assert_eq!(inventory.sensors.len(), 2);
        assert_eq!(kept, vec![0, 3]);
        assert_eq!(inventory.sensors[0].id, "motherboard/mb-1/temperature/a");
        assert_eq!(inventory.sensors[1].id, "motherboard/mb-1/temperature/d");
    }

    #[test]
    fn unknown_category_falls_back_to_the_sensor_kind() {
        let schema = WireSchema {
            devices: vec![device("mb-1", "motherboard", None)],
            sensors: vec![sensor(
                "mb-1",
                "temperature",
                "a",
                "celsius",
                "not-a-known-category",
            )],
        };
        let (inventory, _) = bind(&schema, &DriveIds::default()).expect("bind");
        assert_eq!(inventory.sensors[0].category, "temperature");
    }

    #[test]
    fn two_devices_bound_onto_the_same_disk_is_an_error() {
        let drives = drive_table(vec![drive(0, "storage/device-aaa", Some("M"), Some("S"))]);
        let hint = Some(IdentityHint::Storage {
            physical_drive: 0,
            model: Some("M".to_owned()),
            serial: Some("S".to_owned()),
        });
        let schema = WireSchema {
            devices: vec![
                device("svc-disk-a", "storage", hint.clone()),
                device("svc-disk-b", "storage", hint),
            ],
            sensors: vec![],
        };
        assert!(bind(&schema, &drives).is_err());
    }

    // ---- SvcProvider ----

    fn wire_schema() -> WireSchema {
        WireSchema {
            devices: vec![device(
                "cpu-hw",
                "cpu",
                Some(IdentityHint::Cpu { index: 0 }),
            )],
            sensors: vec![sensor("cpu-hw", "load", "total", "percent", "load")],
        }
    }

    #[test]
    fn no_schema_means_an_empty_inventory() {
        let feed = SvcFeed::default();
        let mut p = SvcProvider::new(feed, DriveIdTable::default());
        let inventory = p.discover().expect("discover");
        assert!(inventory.devices.is_empty());
        assert!(inventory.sensors.is_empty());
    }

    #[test]
    fn new_schema_triggers_rediscovery() {
        let feed = SvcFeed::default();
        feed.set_schema(wire_schema());
        let mut p = SvcProvider::new(feed.clone(), DriveIdTable::default());
        p.discover().expect("discover");
        p.poll().expect("first poll after discover is fine");

        feed.set_schema(wire_schema());
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
    }

    #[test]
    fn drive_table_change_triggers_rediscovery() {
        let feed = SvcFeed::default();
        feed.set_schema(wire_schema());
        let drives = DriveIdTable::default();
        let mut p = SvcProvider::new(feed, drives.clone());
        p.discover().expect("discover");
        p.poll().expect("first poll after discover is fine");

        drives.publish(vec![drive(0, "storage/device-x", Some("m"), Some("s"))]);
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
    }

    #[test]
    fn stale_snapshot_reads_as_missing() {
        let feed = SvcFeed::default();
        feed.set_schema(wire_schema());
        feed.set_interval(Duration::from_millis(100));
        let mut p = SvcProvider::new(feed.clone(), DriveIdTable::default());
        p.discover().expect("discover");
        let stale_at = Instant::now() - Duration::from_millis(400); // 4x the interval
        feed.set_snapshot(
            WireSnapshot {
                seq: 1,
                timestamp_ms: 0,
                values: vec![Some(1.0)],
            },
            stale_at,
        );
        assert_eq!(p.poll().expect("poll"), vec![None]);
    }

    #[test]
    fn disconnect_clears_the_feed_and_requests_rediscovery() {
        let feed = SvcFeed::default();
        feed.set_schema(wire_schema());
        let mut p = SvcProvider::new(feed.clone(), DriveIdTable::default());
        p.discover().expect("discover");
        p.poll().expect("first poll after discover is fine");

        feed.clear();
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
        let inventory = p.discover().expect("discover after clear");
        assert!(inventory.devices.is_empty());
        assert!(inventory.sensors.is_empty());
    }
}
