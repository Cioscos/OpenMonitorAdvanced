//! The drive table published at discovery and the disk power table.

use super::*;

/// One physical disk's stable id, model and serial (descriptor texts,
/// trimmed), published at discovery for the `svc` provider to bind service
/// storage devices onto the same core disk (spec §M4, D3).
#[derive(Clone, Debug, PartialEq)]
pub struct DriveEntry {
    pub index: u32,
    pub device_id: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    /// The wire key of the disk ([`oma_ipc::drive_key`]): what the service
    /// and this app call the disk when they talk about its SMART. `None` when
    /// the descriptor has no model or no serial.
    pub key: Option<String>,
    /// The service leaves this disk's SMART off unless a client asks for it:
    /// a disk on the USB bus (spec M6b §4.2).
    pub smart_default_off: bool,
    /// The storage provider owns this disk's main temperature: it takes the
    /// service's measure itself (a disk that may be rotational, spec M6b
    /// §5.3), so the `svc` provider leaves that sensor out. A non-rotational
    /// disk is read locally and imports nothing: there the service's sensor
    /// stays, and fills in when the local query yields none.
    pub owns_main_temperature: bool,
}

/// Whether the storage provider owns the main temperature of a disk of this
/// class ([`DriveEntry::owns_main_temperature`]).
pub(super) fn owns_main_temperature(class: DiskClass) -> bool {
    class == DiskClass::RotationalOrUnknown
}

impl DriveEntry {
    /// An entry whose `key` follows from `model` and `serial`, with SMART on
    /// by default and the main temperature owned, as for a disk of unknown
    /// class.
    pub fn new(
        index: u32,
        device_id: String,
        model: Option<String>,
        serial: Option<String>,
    ) -> Self {
        let key = match (model.as_deref(), serial.as_deref()) {
            (Some(model), Some(serial)) => oma_ipc::drive_key(model, serial),
            _ => None,
        };
        Self {
            index,
            device_id,
            model,
            serial,
            key,
            smart_default_off: false,
            owns_main_temperature: true,
        }
    }
}

/// Device property telling the UI whether this disk's SMART can be switched
/// off on its own (`"true"`/`"false"`): only a disk whose descriptor has a
/// model and a serial has a drive key (spec M5 §2.8).
pub const SMART_SELECTABLE: &str = "smartSelectable";

/// Device property of a disk whose SMART is off unless the user switches it
/// on (`"off"`); absent for every other disk.
pub const SMART_DEFAULT: &str = "smartDefault";

/// Whether a disk on this bus has its SMART off by default.
pub(super) fn smart_default_off(bus_type: Option<i32>) -> bool {
    bus_type == Some(BusTypeUsb.0)
}

/// Properties of a disk device: its temperature limits, [`SMART_SELECTABLE`]
/// and, for a default-off disk, [`SMART_DEFAULT`].
pub(crate) fn disk_properties(
    report: Option<&TemperatureReport>,
    entry: &DriveEntry,
) -> BTreeMap<String, String> {
    let mut properties = temperature_properties(report);
    properties.insert(SMART_SELECTABLE.to_owned(), entry.key.is_some().to_string());
    if entry.smart_default_off {
        properties.insert(SMART_DEFAULT.to_owned(), "off".to_owned());
    }
    properties
}

/// The wire keys of the disks named by core id in `request`, in request
/// order and without repeats. A disk that is not in `drives` (unplugged, not
/// identified yet) or has no key is dropped.
pub fn drive_keys_for(request: &[String], drives: &[DriveEntry]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for id in request {
        let key = drives
            .iter()
            .find(|d| &d.device_id == id)
            .and_then(|d| d.key.as_ref());
        if let Some(key) = key {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    keys
}

/// The core id of the disk with this wire `key`.
pub fn core_id_for_key<'a>(key: &str, drives: &'a [DriveEntry]) -> Option<&'a str> {
    drives
        .iter()
        .find(|d| d.key.as_deref() == Some(key))
        .map(|d| d.device_id.as_str())
}

/// Snapshot of every identified disk; `generation` bumps only when the set
/// of drives (or their model/serial texts) actually changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DriveIds {
    pub generation: u64,
    pub drives: Vec<DriveEntry>,
}

/// Shared handle: written by `StorageProvider::discover`, read by the `svc`
/// provider on its own tick. Cheap to clone.
#[derive(Clone, Default)]
pub struct DriveIdTable(Arc<Mutex<DriveIds>>);

impl std::fmt::Debug for DriveIdTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriveIdTable")
            .field("generation", &self.generation())
            .finish()
    }
}

impl DriveIdTable {
    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, DriveIds> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the drive list; the generation bumps only when it differs
    /// from the one already published, so a provider comparing generations
    /// does not rediscover on every tick.
    pub fn publish(&self, drives: Vec<DriveEntry>) {
        let mut inner = self.lock();
        if inner.drives != drives {
            inner.drives = drives;
            inner.generation += 1;
        }
    }

    pub fn get(&self) -> DriveIds {
        self.lock().clone()
    }

    /// The generation alone, without copying the list.
    pub fn generation(&self) -> u64 {
        self.lock().generation
    }
}

/// The current state of every identified disk, by core device id. Shared
/// handle: written by `StorageProvider::poll`, read by the shell. Cheap to
/// clone.
#[derive(Clone, Default)]
pub struct DiskStateTable(Arc<Mutex<DiskStates>>);

#[derive(Default)]
pub(super) struct DiskStates {
    pub(super) generation: u64,
    pub(super) states: Vec<(String, DiskPower)>,
}

impl DiskStateTable {
    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, DiskStates> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the states (core device id, power); the generation bumps
    /// only when they differ from the ones already published.
    pub fn publish(&self, states: Vec<(String, DiskPower)>) {
        let mut inner = self.lock();
        if inner.states != states {
            inner.states = states;
            inner.generation += 1;
        }
    }

    /// The generation and the states of the last poll.
    pub fn get(&self) -> (u64, Vec<(String, DiskPower)>) {
        let inner = self.lock();
        (inner.generation, inner.states.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::super::disk_gate::tests::entry;
    use super::*;

    #[test]
    fn a_drive_entry_carries_the_descriptor_key() {
        let disk = entry(3);
        assert_eq!(disk.key, oma_ipc::drive_key("Model", "SN3"));
        assert!(disk.key.is_some());
        // Either text missing (or blank) leaves the disk without a key.
        let no_serial = DriveEntry::new(0, "storage/a".into(), Some("M".into()), None);
        assert_eq!(no_serial.key, None);
        let blank = DriveEntry::new(0, "storage/a".into(), Some("M".into()), Some("  ".into()));
        assert_eq!(blank.key, None);
    }

    #[test]
    fn disk_properties_say_whether_smart_is_selectable() {
        let report = TemperatureReport {
            sensors: BTreeMap::new(),
            warning_c: Some(70),
            critical_c: None,
        };
        let with_key = disk_properties(Some(&report), &entry(1));
        assert_eq!(
            with_key.get(SMART_SELECTABLE).map(String::as_str),
            Some("true")
        );
        assert_eq!(with_key.get("tempWarningC").map(String::as_str), Some("70"));
        let no_key = DriveEntry::new(2, "storage/no-key".into(), Some("M".into()), None);
        let without = disk_properties(None, &no_key);
        assert_eq!(
            without.get(SMART_SELECTABLE).map(String::as_str),
            Some("false")
        );
        assert_eq!(without.len(), 1);
    }

    #[test]
    fn disk_properties_mark_a_usb_disk_as_default_off() {
        let usb = DriveEntry {
            smart_default_off: true,
            ..entry(1)
        };
        let properties = disk_properties(None, &usb);
        assert_eq!(
            properties.get(SMART_DEFAULT).map(String::as_str),
            Some("off")
        );
        assert_eq!(
            properties.get(SMART_SELECTABLE).map(String::as_str),
            Some("true")
        );
        // Every other disk has no such property.
        assert!(!disk_properties(None, &entry(1)).contains_key(SMART_DEFAULT));

        assert!(smart_default_off(Some(BusTypeUsb.0)));
        assert!(!smart_default_off(Some(
            windows::Win32::Storage::FileSystem::BusTypeNvme.0
        )));
        assert!(!smart_default_off(None));
    }

    #[test]
    fn drive_keys_for_translates_and_drops_unknown() {
        let drives = vec![
            entry(0),
            entry(1),
            DriveEntry::new(2, "storage/no-key".into(), None, None),
        ];
        let request = vec![
            "storage/device-1".to_owned(),
            "storage/gone".to_owned(),
            "storage/no-key".to_owned(),
            "storage/device-0".to_owned(),
            "storage/device-1".to_owned(),
        ];
        assert_eq!(
            drive_keys_for(&request, &drives),
            vec![
                oma_ipc::drive_key("Model", "SN1").unwrap(),
                oma_ipc::drive_key("Model", "SN0").unwrap(),
            ],
            "request order, unknown and key-less disks dropped, no duplicates"
        );
        assert!(drive_keys_for(&[], &drives).is_empty());
        assert!(drive_keys_for(&request, &[]).is_empty());
    }

    #[test]
    fn core_id_for_key_finds_the_disk() {
        let drives = vec![entry(0), entry(1)];
        let key = oma_ipc::drive_key("Model", "SN1").unwrap();
        assert_eq!(core_id_for_key(&key, &drives), Some("storage/device-1"));
        assert_eq!(core_id_for_key("nope", &drives), None);
    }

    #[test]
    fn generation_is_readable_without_a_copy() {
        let table = DriveIdTable::default();
        assert_eq!(table.generation(), 0);
        table.publish(vec![entry(0)]);
        assert_eq!(table.generation(), 1);
    }

    #[test]
    fn publish_bumps_generation_only_on_change() {
        let table = DriveIdTable::default();
        assert_eq!(table.get(), DriveIds::default());

        table.publish(vec![entry(0)]);
        let after_first = table.get();
        assert_eq!(after_first.generation, 1);
        assert_eq!(after_first.drives, vec![entry(0)]);

        // Publishing the same list again changes nothing.
        table.publish(vec![entry(0)]);
        assert_eq!(table.get().generation, 1);

        // A real change bumps the generation again.
        table.publish(vec![entry(0), entry(1)]);
        assert_eq!(table.get().generation, 2);
    }
}
