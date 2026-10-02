//! How the service's disks and this app's core disks are told to be the same
//! disk, and which of them this client takes from the service (spec M6b §3.3,
//! §4.2, §7). Pure: no disk is touched here.

use std::time::Instant;

use oma_ipc::{
    DriveState, IdentityHint, WireDrive, WireSchema, WireServiceState, WireSnapshot, MAX_DRIVE_KEYS,
};

use crate::storage::{drive_keys_for, DriveEntry, DriveIds};
use crate::storage_gate::{ServiceDisk, ServiceTemperature};
use crate::svc::feed::{FeedView, SourceRequest};

/// Wire kind and name of a disk's main temperature: the storage provider owns it for every
/// disk the service binds onto (spec M6b §5.3).
pub(crate) const MAIN: (&str, &str) = ("temperature", "drive");

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
pub(crate) fn storage_binding<'a>(
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

/// The service's entry for this core disk: same physical drive number, both
/// keys present and equal, and that key unique in both tables (spec M6b §7).
/// A number or a key alone never associates.
pub(crate) fn wire_drive_for<'a>(
    entry: &DriveEntry,
    drives: &DriveIds,
    service: &'a WireServiceState,
) -> Option<&'a WireDrive> {
    let key = entry.key.as_deref()?;
    let wire = service
        .drives
        .iter()
        .find(|d| d.physical_drive == entry.index)?;
    if wire.key.as_deref() != Some(key) {
        return None;
    }
    let in_core = drives
        .drives
        .iter()
        .filter(|d| d.key.as_deref() == Some(key))
        .count();
    let in_service = service
        .drives
        .iter()
        .filter(|d| d.key.as_deref() == Some(key))
        .count();
    (in_core == 1 && in_service == 1).then_some(wire)
}

/// Whether this client accepts the service as a source for the disk: not
/// with the storage module off, not a disk switched off, and a default-off
/// disk only when switched on. Another client's choice does not count.
pub(crate) fn source_accepted(entry: &DriveEntry, request: &SourceRequest) -> bool {
    let storage_off = request.disabled_modules.iter().any(|m| m == "storage");
    let switched_off = request.smart_disabled_drives.contains(&entry.device_id);
    let switched_on = request.smart_enabled_drives.contains(&entry.device_id);
    !storage_off && !switched_off && (!entry.smart_default_off || switched_on)
}

/// What the service says about one core disk, from an immutable view of the feed.
/// `Absent` when the schema or the snapshot is missing, the snapshot is older than three
/// intervals, or the disk is not associated.
///
/// The temperature is the `temperature`/`drive` sensor of the service device bound onto the
/// disk, and only when this client accepts the service as a source for it
/// ([`source_accepted`]) and the snapshot has a value.
pub(crate) fn service_disk(
    entry: &DriveEntry,
    drives: &DriveIds,
    view: &FeedView,
    now: Instant,
) -> ServiceDisk {
    let (Some(schema), Some((received, snapshot))) = (&view.schema, &view.snapshot) else {
        return ServiceDisk::Absent;
    };
    if now.saturating_duration_since(*received) > view.interval * 3 {
        return ServiceDisk::Absent;
    }
    let Some(wire) = wire_drive_for(entry, drives, &schema.service) else {
        return ServiceDisk::Absent;
    };
    let temperature = source_accepted(entry, &view.request)
        .then(|| main_temperature(entry, drives, schema, snapshot))
        .flatten();
    ServiceDisk::Present {
        state: DriveState::from_wire(&wire.state),
        blocks_smart: wire.blocks_smart,
        temperature,
    }
}

/// The main temperature in `snapshot` of the one service device bound onto `entry`.
fn main_temperature(
    entry: &DriveEntry,
    drives: &DriveIds,
    schema: &WireSchema,
    snapshot: &WireSnapshot,
) -> Option<ServiceTemperature> {
    let mut bound = schema.devices.iter().filter(|device| match &device.hint {
        Some(IdentityHint::Storage {
            physical_drive,
            model,
            serial,
        }) => storage_binding(model, serial, *physical_drive, drives) == Some(entry),
        _ => false,
    });
    let device = bound.next().filter(|_| bound.next().is_none())?;
    let index = schema.sensors.iter().position(|sensor| {
        sensor.device_id == device.id && (sensor.kind.as_str(), sensor.name.as_str()) == MAIN
    })?;
    let value = snapshot.values.get(index).copied().flatten()?;
    value.is_finite().then(|| ServiceTemperature {
        value,
        held: snapshot.held.get(index).copied().unwrap_or(false),
    })
}

/// The drive keys of `request`, switched off and switched on, translated with `drives`: each
/// list without repeats and cut to [`MAX_DRIVE_KEYS`]. The service refuses a key in both lists,
/// and two core ids can have the same key: a key switched off is never also sent as switched
/// on, even when it falls beyond the cut of its own list.
pub(crate) fn request_keys(
    request: &SourceRequest,
    drives: &DriveIds,
) -> (Vec<String>, Vec<String>) {
    let mut disabled = drive_keys_for(&request.smart_disabled_drives, &drives.drives);
    let mut enabled = drive_keys_for(&request.smart_enabled_drives, &drives.drives);
    enabled.retain(|key| !disabled.contains(key));
    disabled.truncate(MAX_DRIVE_KEYS);
    enabled.truncate(MAX_DRIVE_KEYS);
    (disabled, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: u32, serial: &str) -> DriveEntry {
        DriveEntry::new(
            index,
            format!("storage/device-{index}"),
            Some("Model".to_owned()),
            Some(serial.to_owned()),
        )
    }

    fn key(serial: &str) -> Option<String> {
        oma_ipc::drive_key("Model", serial)
    }

    fn wire_drive(physical_drive: u32, key: Option<String>) -> WireDrive {
        WireDrive {
            physical_drive,
            key,
            model: Some("Model".to_owned()),
            state: "active".to_owned(),
            blocks_smart: false,
        }
    }

    fn table(drives: Vec<DriveEntry>) -> DriveIds {
        DriveIds {
            generation: 1,
            drives,
        }
    }

    fn service(drives: Vec<WireDrive>) -> WireServiceState {
        WireServiceState {
            drives,
            ..WireServiceState::default()
        }
    }

    #[test]
    fn a_drive_is_associated_by_number_and_unique_key() {
        let drives = table(vec![entry(0, "SN-0"), entry(1, "SN-1")]);
        let service = service(vec![
            wire_drive(0, key("SN-0")),
            wire_drive(1, key("SN-1")),
            wire_drive(2, None),
        ]);
        for index in [0, 1] {
            let found = wire_drive_for(&drives.drives[index], &drives, &service);
            assert_eq!(found, Some(&service.drives[index]), "disk {index}");
        }
    }

    #[test]
    fn a_reused_drive_number_with_another_key_is_not_associated() {
        let drives = table(vec![entry(0, "SN-0")]);
        // Another disk took the number after a hot-plug.
        let reused = service(vec![wire_drive(0, key("SN-other"))]);
        assert_eq!(wire_drive_for(&drives.drives[0], &drives, &reused), None);
        // The same disk under another number is not associated by key alone.
        let moved = service(vec![wire_drive(3, key("SN-0"))]);
        assert_eq!(wire_drive_for(&drives.drives[0], &drives, &moved), None);
    }

    #[test]
    fn a_missing_or_duplicated_key_is_not_associated() {
        // The right number and key associate; every case below spoils one condition.
        let drives = table(vec![entry(0, "SN-0")]);
        let good = service(vec![wire_drive(0, key("SN-0"))]);
        assert!(wire_drive_for(&drives.drives[0], &drives, &good).is_some());

        // No key on the service's side.
        let no_wire_key = service(vec![wire_drive(0, None)]);
        assert_eq!(
            wire_drive_for(&drives.drives[0], &drives, &no_wire_key),
            None
        );

        // No key on either side: `None == None` is never identity.
        let keyless = table(vec![DriveEntry::new(
            0,
            "storage/device-0".to_owned(),
            Some("Model".to_owned()),
            None,
        )]);
        assert_eq!(
            wire_drive_for(&keyless.drives[0], &keyless, &no_wire_key),
            None
        );
        assert_eq!(wire_drive_for(&keyless.drives[0], &keyless, &good), None);

        // The key twice in the service's table.
        let twice_on_the_wire =
            service(vec![wire_drive(0, key("SN-0")), wire_drive(1, key("SN-0"))]);
        assert_eq!(
            wire_drive_for(&drives.drives[0], &drives, &twice_on_the_wire),
            None
        );

        // The key twice in the core's table.
        let twins = table(vec![entry(0, "SN-0"), entry(1, "SN-0")]);
        assert_eq!(wire_drive_for(&twins.drives[0], &twins, &good), None);
    }

    #[test]
    fn a_default_off_disk_is_accepted_only_when_enabled() {
        let id = "storage/device-0".to_owned();
        let usb = DriveEntry {
            smart_default_off: true,
            ..entry(0, "SN-0")
        };
        let enabled = SourceRequest {
            smart_enabled_drives: vec![id.clone()],
            ..SourceRequest::default()
        };
        assert!(!source_accepted(&usb, &SourceRequest::default()));
        assert!(source_accepted(&usb, &enabled));
        // Switched off wins over switched on.
        let both = SourceRequest {
            smart_disabled_drives: vec![id.clone()],
            ..enabled.clone()
        };
        assert!(!source_accepted(&usb, &both));
        // With the storage module off nothing is accepted.
        let no_storage = SourceRequest {
            disabled_modules: vec!["storage".to_owned()],
            ..enabled.clone()
        };
        assert!(!source_accepted(&usb, &no_storage));

        // A disk that is on by default needs no request, and can be switched off.
        let sata = entry(0, "SN-0");
        assert!(source_accepted(&sata, &SourceRequest::default()));
        assert!(source_accepted(&sata, &enabled));
        assert!(!source_accepted(&sata, &both));
        assert!(!source_accepted(&sata, &no_storage));
    }
}
