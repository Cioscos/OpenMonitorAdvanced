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
use crate::svc::feed::{SourceRequest, SvcFeed};

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

/// The module of `oma_ipc::MODULES` whose devices have this wire kind, as
/// `SchemaBuilder.cs` names them: `fan_controller` is the `controller` module.
fn module_of(wire_kind: &str) -> Option<&'static str> {
    match wire_kind {
        "cpu" => Some("cpu"),
        "motherboard" => Some("motherboard"),
        "memory" => Some("memory"),
        "storage" => Some("storage"),
        "fan_controller" => Some("controller"),
        "psu" => Some("psu"),
        _ => None,
    }
}

/// A schema device once its final core id and kind are known.
struct Bound {
    final_id: String,
    kind: DeviceKind,
    /// A storage device that did not bind onto a core disk: its own page
    /// shows the SMART data only (spec D3), see [`is_core_disk_io`].
    unbound_storage: bool,
}

/// The service's per-disk I/O sensors, which duplicate what the core's
/// storage provider already shows for every disk. On a bound disk they
/// merge with the core's (D2); on an unbound one they would repeat the same
/// I/O under a second entry, so they are dropped there.
fn is_core_disk_io(kind: &str, name: &str) -> bool {
    matches!(
        (kind, name),
        ("throughput", "read") | ("throughput", "write") | ("load", "active")
    )
}

/// Binds `schema`'s devices onto core ids and builds the resulting
/// inventory: `Cpu`/`Memory` hints map to the CPU/memory provider's device
/// ids, a `Storage` hint maps onto a disk of `drives` only when
/// [`storage_binding`] agrees, and every other device (or a hint that did
/// not match) gets `<kind>/<device id>`; such an unbound storage device keeps
/// only its SMART data ([`is_core_disk_io`]), and is left out when nothing
/// (no sensor, no property) remains. A device or sensor whose kind/unit
/// is not one `oma_core::model` knows is skipped, with one log warning for
/// the whole schema. Returns the kept sensors' wire indices, in the same
/// order as `Inventory::sensors`, for `poll` to read the matching values.
///
/// `request` is the user's own choice, applied here whatever the service
/// sends (another client may keep a source on that this app turned off): a
/// device of a module in `disabled_modules` is left out with its sensors, and
/// so is a service disk bound onto a core disk in `smart_disabled_drives`
/// (the core's own I/O for that disk is not touched).
///
/// Fails without publishing a partial inventory if binding produces two
/// devices with the same final id (e.g. two service devices bound onto the
/// same core disk): the link already rejects duplicate/invalid ids and
/// dangling sensor references before a schema reaches the feed, so this is
/// the last check specific to the binding step itself.
pub(crate) fn bind(
    schema: &WireSchema,
    drives: &DriveIds,
    request: &SourceRequest,
) -> Result<(Inventory, Vec<usize>), ProviderError> {
    let mut unknown = false;
    let mut bound: HashMap<&str, Bound> = HashMap::with_capacity(schema.devices.len());
    for device in &schema.devices {
        let Some(kind) = parse_wire::<DeviceKind>(&device.kind) else {
            unknown = true;
            continue;
        };
        let turned_off = module_of(&device.kind)
            .is_some_and(|module| request.disabled_modules.iter().any(|m| m == module));
        if turned_off {
            continue;
        }
        let mut bound_to_disk = false;
        let final_id = match &device.hint {
            Some(IdentityHint::Cpu { index }) => format!("cpu/{index}"),
            Some(IdentityHint::Memory {}) => "memory/0".to_owned(),
            Some(IdentityHint::Storage {
                physical_drive,
                model,
                serial,
            }) => match storage_binding(model, serial, *physical_drive, drives) {
                Some(entry) => {
                    bound_to_disk = true;
                    entry.device_id.clone()
                }
                None => format!("{}/{}", device.kind, device.id),
            },
            None => format!("{}/{}", device.kind, device.id),
        };
        if bound_to_disk && request.smart_disabled_drives.contains(&final_id) {
            continue;
        }
        let unbound_storage = kind == DeviceKind::Storage && !bound_to_disk;
        bound.insert(
            device.id.as_str(),
            Bound {
                final_id,
                kind,
                unbound_storage,
            },
        );
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

    let mut sensors = Vec::new();
    let mut kept = Vec::new();
    for (index, sensor) in schema.sensors.iter().enumerate() {
        let Some(b) = bound.get(sensor.device_id.as_str()) else {
            continue;
        };
        if b.unbound_storage && is_core_disk_io(&sensor.kind, &sensor.name) {
            continue;
        }
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

    // An unbound disk left with no sensor and no property (e.g. a USB disk
    // without SMART, once its I/O duplicates are gone) would be an empty page.
    let with_sensors: HashSet<&str> = sensors.iter().map(|s| s.device_id.as_str()).collect();
    let mut devices = Vec::with_capacity(bound.len());
    for device in &schema.devices {
        let Some(b) = bound.get(device.id.as_str()) else {
            continue;
        };
        if b.unbound_storage
            && device.properties.is_empty()
            && !with_sensors.contains(b.final_id.as_str())
        {
            continue;
        }
        devices.push(Device {
            id: b.final_id.clone(),
            kind: b.kind,
            name: device.name.clone(),
            vendor: device.vendor.clone(),
            properties: device.properties.clone(),
        });
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
        let (inventory, kept) = bind(&schema, &drives, &view.request)?;
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
        DriveEntry::new(
            index,
            id.to_owned(),
            model.map(str::to_owned),
            serial.map(str::to_owned),
        )
    }

    #[test]
    fn cpu_and_memory_hints_bind_to_core_ids() {
        let schema = WireSchema {
            service: Default::default(),
            devices: vec![
                device("cpu-hw", "cpu", Some(IdentityHint::Cpu { index: 0 })),
                device("mem-hw", "memory", Some(IdentityHint::Memory {})),
            ],
            sensors: vec![],
        };
        let (inventory, _) =
            bind(&schema, &DriveIds::default(), &SourceRequest::default()).expect("bind");
        let ids: Vec<&str> = inventory.devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["cpu/0", "memory/0"]);
    }

    #[test]
    fn storage_hint_binds_only_when_model_and_serial_match() {
        // An unbound disk needs a SMART sensor to be published at all.
        let smart = |id: &str| sensor(id, "temperature", "drive", "celsius", "temperature");
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
                service: Default::default(),
                devices: vec![matching],
                sensors: vec![],
            },
            &drives,
            &SourceRequest::default(),
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
                service: Default::default(),
                devices: vec![device(
                    &format!("svc-disk-{i}"),
                    "storage",
                    Some(hint.clone()),
                )],
                sensors: vec![smart(&format!("svc-disk-{i}"))],
            };
            let (inventory, _) = bind(&schema, &drives, &SourceRequest::default()).expect("bind");
            assert_eq!(
                inventory.devices[0].id,
                format!("storage/svc-disk-{i}"),
                "case {i}: not a fallback id"
            );
        }

        // Both sides missing (None == None) is never identity.
        let no_identity_drives = drives_with_no_identity();
        let schema = WireSchema {
            service: Default::default(),
            devices: vec![device(
                "svc-disk-none",
                "storage",
                Some(IdentityHint::Storage {
                    physical_drive: 5,
                    model: None,
                    serial: None,
                }),
            )],
            sensors: vec![smart("svc-disk-none")],
        };
        let (inventory, _) =
            bind(&schema, &no_identity_drives, &SourceRequest::default()).expect("bind");
        assert_eq!(inventory.devices[0].id, "storage/svc-disk-none");

        // Duplicated pair on the drive side is ambiguous: no binding.
        let ambiguous = drive_table(vec![
            drive(0, "storage/device-a", Some("Same"), Some("Same-SN")),
            drive(1, "storage/device-b", Some("Same"), Some("Same-SN")),
        ]);
        let schema = WireSchema {
            service: Default::default(),
            devices: vec![device(
                "svc-disk-amb",
                "storage",
                Some(IdentityHint::Storage {
                    physical_drive: 0,
                    model: Some("Same".to_owned()),
                    serial: Some("Same-SN".to_owned()),
                }),
            )],
            sensors: vec![smart("svc-disk-amb")],
        };
        let (inventory, _) = bind(&schema, &ambiguous, &SourceRequest::default()).expect("bind");
        assert_eq!(inventory.devices[0].id, "storage/svc-disk-amb");
    }

    #[test]
    fn an_unbound_disk_keeps_only_its_smart_data() {
        // The core already shows every disk's I/O (spec D3): a disk the service could not
        // bind onto a core disk gets its own page with the SMART data only, never a second
        // copy of read/write/active under another entry.
        let drives = drive_table(vec![drive(0, "storage/device-aaa", Some("M"), Some("S"))]);
        let bound_hint = IdentityHint::Storage {
            physical_drive: 0,
            model: Some("M".to_owned()),
            serial: Some("S".to_owned()),
        };
        let unbound_hint = IdentityHint::Storage {
            physical_drive: 1,
            model: Some("Other".to_owned()),
            serial: Some("X".to_owned()),
        };
        let io_and_smart = |id: &str| {
            vec![
                sensor(id, "throughput", "read", "bytes_per_second", "throughput"),
                sensor(id, "throughput", "write", "bytes_per_second", "throughput"),
                sensor(id, "load", "active", "percent", "load"),
                sensor(id, "temperature", "drive", "celsius", "temperature"),
                sensor(id, "percent", "life", "percent", "percent"),
            ]
        };
        let schema = WireSchema {
            service: Default::default(),
            devices: vec![
                device("svc-bound", "storage", Some(bound_hint)),
                device("svc-unbound", "storage", Some(unbound_hint)),
                device("svc-nohint", "storage", None),
            ],
            sensors: [
                io_and_smart("svc-bound"),
                io_and_smart("svc-unbound"),
                io_and_smart("svc-nohint"),
            ]
            .concat(),
        };

        let (inventory, kept) = bind(&schema, &drives, &SourceRequest::default()).expect("bind");

        let ids: Vec<&str> = inventory.sensors.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "storage/device-aaa/throughput/read",
                "storage/device-aaa/throughput/write",
                "storage/device-aaa/load/active",
                "storage/device-aaa/temperature/drive",
                "storage/device-aaa/percent/life",
                "storage/svc-unbound/temperature/drive",
                "storage/svc-unbound/percent/life",
                "storage/svc-nohint/temperature/drive",
                "storage/svc-nohint/percent/life",
            ]
        );
        assert_eq!(kept, vec![0, 1, 2, 3, 4, 8, 9, 13, 14]);
    }

    #[test]
    fn an_unbound_disk_left_empty_is_not_published() {
        // A USB disk without SMART: once its I/O duplicates are gone nothing is
        // left, so no empty page. With a property (or any SMART sensor) it stays.
        let io_only = |id: &str| {
            vec![
                sensor(id, "throughput", "read", "bytes_per_second", "throughput"),
                sensor(id, "throughput", "write", "bytes_per_second", "throughput"),
                sensor(id, "load", "active", "percent", "load"),
            ]
        };
        let mut with_property = device("svc-prop", "storage", None);
        with_property
            .properties
            .insert("availableSpareThresholdPct".to_owned(), "10".to_owned());
        let schema = WireSchema {
            service: Default::default(),
            devices: vec![
                device("svc-usb", "storage", None),
                with_property,
                device("mb-1", "motherboard", None),
            ],
            sensors: [io_only("svc-usb"), io_only("svc-prop")].concat(),
        };

        let (inventory, kept) =
            bind(&schema, &DriveIds::default(), &SourceRequest::default()).expect("bind");

        let ids: Vec<&str> = inventory.devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["storage/svc-prop", "motherboard/mb-1"]);
        assert!(inventory.sensors.is_empty());
        assert!(kept.is_empty());
    }

    fn drives_with_no_identity() -> DriveIds {
        drive_table(vec![drive(5, "storage/device-x", None, None)])
    }

    #[test]
    fn unbound_devices_get_kind_slash_id() {
        let schema = WireSchema {
            service: Default::default(),
            devices: vec![device("mb-1", "motherboard", None)],
            sensors: vec![],
        };
        let (inventory, _) =
            bind(&schema, &DriveIds::default(), &SourceRequest::default()).expect("bind");
        assert_eq!(inventory.devices[0].id, "motherboard/mb-1");
    }

    #[test]
    fn sensor_ids_labels_and_source() {
        let schema = WireSchema {
            service: Default::default(),
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
        let (inventory, kept) =
            bind(&schema, &DriveIds::default(), &SourceRequest::default()).expect("bind");
        let s = &inventory.sensors[0];
        assert_eq!(s.id, "cpu/0/temperature/package");
        assert_eq!(s.label, Label::new("cpu.temperature.package"));
        assert_eq!(s.source, Source::Lhm);
        assert_eq!(kept, vec![0]);

        let schema = WireSchema {
            service: Default::default(),
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
        let (inventory, _) =
            bind(&schema, &DriveIds::default(), &SourceRequest::default()).expect("bind");
        assert_eq!(
            inventory.sensors[0].label,
            Label::with_arg("lhm.raw", "Fan #1")
        );
    }

    #[test]
    fn unknown_kind_or_unit_is_skipped() {
        let schema = WireSchema {
            service: Default::default(),
            devices: vec![device("mb-1", "motherboard", None)],
            sensors: vec![
                sensor("mb-1", "temperature", "a", "celsius", "temperature"),
                sensor("mb-1", "quantum-flux", "b", "celsius", "temperature"),
                sensor("mb-1", "temperature", "c", "a-made-up-unit", "temperature"),
                sensor("mb-1", "temperature", "d", "celsius", "temperature"),
            ],
        };
        let (inventory, kept) =
            bind(&schema, &DriveIds::default(), &SourceRequest::default()).expect("bind");
        assert_eq!(inventory.sensors.len(), 2);
        assert_eq!(kept, vec![0, 3]);
        assert_eq!(inventory.sensors[0].id, "motherboard/mb-1/temperature/a");
        assert_eq!(inventory.sensors[1].id, "motherboard/mb-1/temperature/d");
    }

    #[test]
    fn unknown_category_falls_back_to_the_sensor_kind() {
        let schema = WireSchema {
            service: Default::default(),
            devices: vec![device("mb-1", "motherboard", None)],
            sensors: vec![sensor(
                "mb-1",
                "temperature",
                "a",
                "celsius",
                "not-a-known-category",
            )],
        };
        let (inventory, _) =
            bind(&schema, &DriveIds::default(), &SourceRequest::default()).expect("bind");
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
            service: Default::default(),
            devices: vec![
                device("svc-disk-a", "storage", hint.clone()),
                device("svc-disk-b", "storage", hint),
            ],
            sensors: vec![],
        };
        assert!(bind(&schema, &drives, &SourceRequest::default()).is_err());
    }

    // ---- SvcProvider ----

    fn wire_schema() -> WireSchema {
        WireSchema {
            service: Default::default(),
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

        // The same schema again is no news: a resubscribe must not rediscover.
        feed.set_schema(wire_schema());
        p.poll().expect("an identical schema does not invalidate");

        let mut changed = wire_schema();
        changed.devices[0].name.push_str(" (renamed)");
        feed.set_schema(changed);
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

    // ---- the local filter ----

    fn all_devices_schema() -> WireSchema {
        let kinds = [
            ("cpu-hw", "cpu"),
            ("mb-1", "motherboard"),
            ("ram-hw", "memory"),
            ("fc-1", "fan_controller"),
            ("psu-1", "psu"),
            ("disk-1", "storage"),
        ];
        WireSchema {
            service: Default::default(),
            devices: kinds
                .iter()
                .map(|(id, kind)| device(id, kind, None))
                .collect(),
            sensors: kinds
                .iter()
                .map(|(id, _)| sensor(id, "temperature", "t", "celsius", "temperature"))
                .collect(),
        }
    }

    fn request(modules: &[&str], drives: &[&str]) -> SourceRequest {
        SourceRequest {
            disabled_modules: modules.iter().map(|m| (*m).to_owned()).collect(),
            smart_disabled_drives: drives.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    #[test]
    fn excluded_module_devices_are_filtered_locally() {
        let schema = all_devices_schema();
        let (all, _) = bind(&schema, &DriveIds::default(), &SourceRequest::default()).unwrap();
        assert_eq!(all.devices.len(), 6);

        // Each module name switches off exactly the wire kind it stands for,
        // and takes the device's sensors with it.
        for (module, wire_id) in [
            ("cpu", "cpu-hw"),
            ("motherboard", "mb-1"),
            ("memory", "ram-hw"),
            ("storage", "disk-1"),
            ("controller", "fc-1"),
            ("psu", "psu-1"),
        ] {
            let (inventory, kept) =
                bind(&schema, &DriveIds::default(), &request(&[module], &[])).unwrap();
            assert_eq!(inventory.devices.len(), 5, "{module}");
            assert!(
                inventory.devices.iter().all(|d| !d.id.ends_with(wire_id)),
                "{module} still shows {wire_id}"
            );
            assert_eq!(inventory.sensors.len(), 5, "{module}");
            assert_eq!(kept.len(), 5, "{module}");
        }

        let (none, kept) = bind(
            &schema,
            &DriveIds::default(),
            &request(&oma_ipc::MODULES, &[]),
        )
        .unwrap();
        assert!(none.devices.is_empty() && none.sensors.is_empty() && kept.is_empty());
    }

    #[test]
    fn smart_disabled_drive_is_filtered_locally() {
        let drives = drive_table(vec![
            drive(0, "storage/device-aaa", Some("M0"), Some("S0")),
            drive(1, "storage/device-bbb", Some("M1"), Some("S1")),
        ]);
        let hint = |index: u32, model: &str, serial: &str| {
            Some(IdentityHint::Storage {
                physical_drive: index,
                model: Some(model.to_owned()),
                serial: Some(serial.to_owned()),
            })
        };
        let schema = WireSchema {
            service: Default::default(),
            devices: vec![
                device("svc-a", "storage", hint(0, "M0", "S0")),
                device("svc-b", "storage", hint(1, "M1", "S1")),
            ],
            sensors: vec![
                sensor("svc-a", "temperature", "drive", "celsius", "temperature"),
                sensor("svc-b", "temperature", "drive", "celsius", "temperature"),
            ],
        };

        // Another client keeps disk A's SMART on: the service still sends it,
        // and this app hides it.
        let (inventory, kept) =
            bind(&schema, &drives, &request(&[], &["storage/device-aaa"])).unwrap();
        let ids: Vec<&str> = inventory.devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["storage/device-bbb"]);
        let sensor_ids: Vec<&str> = inventory.sensors.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(sensor_ids, vec!["storage/device-bbb/temperature/drive"]);
        assert_eq!(kept, vec![1]);

        // A disk that is not in the request stays.
        let (inventory, _) = bind(&schema, &drives, &request(&[], &["storage/other"])).unwrap();
        assert_eq!(inventory.devices.len(), 2);
    }

    #[test]
    fn a_new_request_triggers_rediscovery() {
        let feed = SvcFeed::default();
        feed.set_schema(wire_schema());
        let mut p = SvcProvider::new(feed.clone(), DriveIdTable::default());
        p.discover().expect("discover");
        p.poll().expect("first poll after discover is fine");

        feed.set_request(request(&["cpu"], &[]));
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
        let inventory = p.discover().expect("discover with the filter");
        assert!(inventory.devices.is_empty(), "the cpu module is off");
        p.poll().expect("settled");
    }
}
