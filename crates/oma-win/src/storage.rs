//! Physical disk throughput and activity (PDH), disk temperatures and volume usage.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

use crate::pdh::{Counter, Query};
use crate::storage_identity::{
    assign_disk_ids, descriptor_texts, disk_identity_candidates, volume_identity,
    DiskIdentityCandidates,
};
use crate::storage_ioctl::PhysicalDrive;
use crate::storage_temperature::{
    declared_positions, declared_values, may_query, next_refresh, query_temperatures, sensor_label,
    sensor_name, temperature_properties, TemperatureReport,
};

const READ: &str = r"\PhysicalDisk(*)\Disk Read Bytes/sec";
const WRITE: &str = r"\PhysicalDisk(*)\Disk Write Bytes/sec";
const IDLE: &str = r"\PhysicalDisk(*)\% Idle Time";

/// A "PhysicalDisk" instance such as "2 C: D:" (disk 2 holding C: and D:).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiskInstance {
    pub instance: String,
    pub index: u32,
    pub volumes: Vec<String>,
}

/// `None` for "_Total".
pub(crate) fn parse_disk_instance(name: &str) -> Option<DiskInstance> {
    let mut parts = name.split_whitespace();
    let index = parts.next()?.parse().ok()?;
    let volumes = parts
        .filter(|p| p.len() == 2 && p.ends_with(':') && p.as_bytes()[0].is_ascii_alphabetic())
        .map(|p| p.to_ascii_uppercase())
        .collect();
    Some(DiskInstance {
        instance: name.to_owned(),
        index,
        volumes,
    })
}

pub(crate) fn disk_instances(names: &[String]) -> Vec<DiskInstance> {
    let mut disks: Vec<_> = names
        .iter()
        .filter_map(|n| parse_disk_instance(n))
        .collect();
    disks.sort_by_key(|d| d.index);
    disks
}

pub(crate) fn disks_changed(known: &[DiskInstance], names: &[String]) -> bool {
    disk_instances(names) != known
}

/// True when a volume's identity no longer matches the one recorded at discovery
/// (including when the volume no longer resolves at all): the letter was reused
/// by different hardware or a recreated partition, so history must not carry over.
pub(crate) fn volume_identity_changed(recorded: &str, current: Option<&str>) -> bool {
    current != Some(recorded)
}

pub(crate) fn disk_name(disk: &DiskInstance) -> String {
    if disk.volumes.is_empty() {
        format!("Disk {}", disk.index)
    } else {
        format!("Disk {} ({})", disk.index, disk.volumes.join(", "))
    }
}

pub(crate) fn active_pct(idle: f64) -> Option<f64> {
    idle.is_finite().then(|| (100.0 - idle).clamp(0.0, 100.0))
}

pub(crate) fn used_pct(total: u64, free: u64) -> Option<f64> {
    (total > 0).then(|| total.saturating_sub(free) as f64 * 100.0 / total as f64)
}

/// `(total, free)` bytes of a volume such as "C:"; `None` if unavailable.
fn volume_space(volume: &str) -> Option<(u64, u64)> {
    let root = HSTRING::from(format!("{volume}\\"));
    let (mut total, mut free) = (0u64, 0u64);
    // SAFETY: valid root path and out-pointers.
    unsafe { GetDiskFreeSpaceExW(&root, None, Some(&mut total), Some(&mut free)) }.ok()?;
    Some((total, free))
}

/// Temperature sensors of one disk: the driver indices declared at
/// discovery, their latest values (repeated between refreshes) and when they
/// were read.
struct DiskTemperatures {
    positions: Vec<usize>,
    values: Vec<Option<f64>>,
    read_at: Instant,
}

impl DiskTemperatures {
    /// Refreshes values and the attempt deadline, including failed/asleep reads.
    /// New driver indices require a schema rebuild, never a value-vector resize.
    fn refresh(&mut self, report: Option<&TemperatureReport>, now: Instant) -> bool {
        self.values = declared_values(report, &self.positions);
        self.read_at = now;
        report.is_some_and(|r| {
            declared_positions(r)
                .iter()
                .any(|i| !self.positions.contains(i))
        })
    }
}

/// Temperatures of disk `index`. A disk known to be spun down is not queried,
/// because the query could wake it up: `None`, as for an unsupported disk.
fn read_temperatures(index: u32) -> Option<TemperatureReport> {
    PhysicalDrive::open(index)
        .filter(|drive| may_query(drive.powered_on()))
        .and_then(|drive| query_temperatures(&drive))
}

struct Counters {
    query: Query,
    read: Counter,
    write: Counter,
    idle: Counter,
}

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
}

impl DriveEntry {
    /// An entry whose `key` follows from `model` and `serial`.
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
        }
    }
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
    fn lock(&self) -> std::sync::MutexGuard<'_, DriveIds> {
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

pub struct StorageProvider {
    drives: DriveIdTable,
    counters: Option<Counters>,
    disks: Vec<DiskInstance>,
    disk_ids: HashMap<u32, String>,
    volume_ids: HashMap<String, String>,
    /// Every identified disk, including those waiting for temperature support/wake.
    temperatures: HashMap<u32, DiskTemperatures>,
    /// Set by `discover`; consumed by the next `poll`. See `take_fresh`.
    fresh: bool,
}

impl Default for StorageProvider {
    fn default() -> Self {
        Self::new(DriveIdTable::default())
    }
}

impl StorageProvider {
    pub fn new(drives: DriveIdTable) -> Self {
        Self {
            drives,
            counters: None,
            disks: Vec::new(),
            disk_ids: HashMap::new(),
            volume_ids: HashMap::new(),
            temperatures: HashMap::new(),
            fresh: false,
        }
    }
}

/// `true` only for the first call after a discover: PDH rate counters were
/// just added, so the collect a few milliseconds later has too short an
/// interval to yield a meaningful rate (noisy throughput/active time),
/// mirroring the network provider's first-sample rule. Resets the flag as a
/// side effect.
fn take_fresh(fresh: &mut bool) -> bool {
    std::mem::replace(fresh, false)
}

impl Provider for StorageProvider {
    fn name(&self) -> &'static str {
        "storage"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        let mut query = Query::open()?;
        let read = query.add_english(READ)?;
        let write = query.add_english(WRITE)?;
        let idle = query.add_english(IDLE)?;
        query.collect()?;
        let disks = disk_instances(&query.instances(read)?);

        // Resolve stable identities only during discovery; never persist PDH indices.
        // Fallback chain and ambiguity rule: storage_identity::assign_disk_ids.
        let candidates: BTreeMap<u32, DiskIdentityCandidates> = disks
            .iter()
            .map(|d| (d.index, disk_identity_candidates(d.index)))
            .collect();
        let disk_ids: HashMap<u32, String> = assign_disk_ids(&candidates)
            .into_iter()
            .map(|(index, (id, _tier))| (index, id))
            .collect();
        let volume_ids: HashMap<String, String> = disks
            .iter()
            .flat_map(|d| &d.volumes)
            .filter_map(|v| volume_identity(v).map(|id| (v.clone(), id)))
            .collect();
        let mut devices = Vec::new();
        let mut sensors = Vec::new();
        let mut temperatures = HashMap::new();
        let mut drive_entries = Vec::new();
        for disk in &disks {
            // assign_disk_ids has already logged why a disk has no identity.
            let Some(id) = disk_ids.get(&disk.index).cloned() else {
                continue;
            };
            let (model, serial) = descriptor_texts(disk.index);
            drive_entries.push(DriveEntry::new(disk.index, id.clone(), model, serial));
            // Unknown/asleep disks remain scheduled; a later successful probe
            // requests rediscovery when it reveals undeclared sensor indices.
            let report = read_temperatures(disk.index);
            devices.push(Device {
                id: id.clone(),
                kind: DeviceKind::Storage,
                name: disk_name(disk),
                vendor: None,
                properties: temperature_properties(report.as_ref()),
            });
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "read",
                Unit::BytesPerSecond,
                Label::new("storage.read"),
                Source::Pdh,
            ));
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "write",
                Unit::BytesPerSecond,
                Label::new("storage.write"),
                Source::Pdh,
            ));
            sensors.push(Sensor::new(
                &id,
                SensorKind::Load,
                "active",
                Unit::Percent,
                Label::new("storage.active"),
                Source::Pdh,
            ));
            let positions = report.as_ref().map(declared_positions).unwrap_or_default();
            for &position in &positions {
                sensors.push(Sensor::new(
                    &id,
                    SensorKind::Temperature,
                    &sensor_name(position),
                    Unit::Celsius,
                    sensor_label(position),
                    Source::Win32,
                ));
            }
            temperatures.insert(
                disk.index,
                DiskTemperatures {
                    values: declared_values(report.as_ref(), &positions),
                    positions,
                    read_at: Instant::now(),
                },
            );
            for volume in &disk.volumes {
                let Some(volume_id) = volume_ids.get(volume) else {
                    continue;
                };
                sensors.push(Sensor::new(
                    &id,
                    SensorKind::Percent,
                    &format!("volume-{}", volume_id),
                    Unit::Percent,
                    Label::with_arg("storage.volumeUsed", volume.clone()),
                    Source::Win32,
                ));
                sensors.push(Sensor::new(
                    &id,
                    SensorKind::Data,
                    &format!("volume-{}-free", volume_id),
                    Unit::Bytes,
                    Label::with_arg("storage.volumeFree", volume.clone()),
                    Source::Win32,
                ));
            }
        }
        self.counters = Some(Counters {
            query,
            read,
            write,
            idle,
        });
        self.disks = disks;
        self.disk_ids = disk_ids;
        self.volume_ids = volume_ids;
        self.temperatures = temperatures;
        self.fresh = true;
        self.drives.publish(drive_entries);
        Ok(Inventory { devices, sensors })
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let fresh = take_fresh(&mut self.fresh);
        let counters = self.counters.as_mut().ok_or(ProviderError::Rediscover)?;
        counters.query.collect()?;
        let read = counters.query.array(counters.read)?;
        // Raw instances distinguish a missing disk from rate-counter warm-up.
        if disks_changed(&self.disks, &counters.query.instances(counters.read)?) {
            return Err(ProviderError::Rediscover);
        }
        let read: HashMap<String, f64> = read.into_iter().collect();
        let write: HashMap<String, f64> =
            counters.query.array(counters.write)?.into_iter().collect();
        let idle: HashMap<String, f64> = counters.query.array(counters.idle)?.into_iter().collect();
        let finite =
            |map: &HashMap<String, f64>, key: &str| map.get(key).copied().filter(|v| v.is_finite());
        let reads = self.temperatures.iter().map(|(&i, t)| (i, t.read_at));
        if let Some(index) = next_refresh(reads, Instant::now()) {
            if let Some(disk) = self.temperatures.get_mut(&index) {
                let report = read_temperatures(index);
                if disk.refresh(report.as_ref(), Instant::now()) {
                    return Err(ProviderError::Rediscover);
                }
            }
        }

        let mut values = Vec::new();
        for disk in &self.disks {
            if !self.disk_ids.contains_key(&disk.index) {
                continue;
            }
            if fresh {
                // Prime the PDH rate counters but report no value yet.
                values.extend([None, None, None]);
            } else {
                values.push(finite(&read, &disk.instance));
                values.push(finite(&write, &disk.instance));
                values.push(finite(&idle, &disk.instance).and_then(active_pct));
            }
            // Not a rate: the last read is valid on the first poll too.
            if let Some(temperatures) = self.temperatures.get(&disk.index) {
                values.extend(temperatures.values.iter().copied());
            }
            for volume in &disk.volumes {
                let Some(recorded_id) = self.volume_ids.get(volume) else {
                    continue;
                };
                // The letter alone cannot tell a swapped disk or recreated partition
                // from the one seen at discovery; the volume GUID can.
                if volume_identity_changed(recorded_id, volume_identity(volume).as_deref()) {
                    return Err(ProviderError::Rediscover);
                }
                match volume_space(volume) {
                    Some((total, free)) => {
                        values.push(used_pct(total, free));
                        values.push(Some(free as f64));
                    }
                    None => values.extend([None, None]),
                }
            }
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_disk_with_one_volume() {
        let d = parse_disk_instance("2 C:").unwrap();
        assert_eq!(d.index, 2);
        assert_eq!(d.volumes, vec!["C:".to_string()]);
    }

    #[test]
    fn parses_disk_with_several_or_no_volumes() {
        assert_eq!(
            parse_disk_instance("0 C: D:").unwrap().volumes,
            vec!["C:", "D:"]
        );
        assert!(parse_disk_instance("1").unwrap().volumes.is_empty());
        assert_eq!(parse_disk_instance("3 e:").unwrap().volumes, vec!["E:"]);
    }

    #[test]
    fn total_instance_is_not_a_disk() {
        assert_eq!(parse_disk_instance("_Total"), None);
    }

    #[test]
    fn disks_are_sorted_by_index() {
        let disks = disk_instances(&["2 C:".into(), "_Total".into(), "0 D:".into()]);
        assert_eq!(
            disks.iter().map(|d| d.index).collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn detects_disk_set_changes() {
        let known = disk_instances(&["0 C:".into()]);
        assert!(!disks_changed(&known, &["0 C:".into(), "_Total".into()]));
        assert!(disks_changed(
            &known,
            &["0 C:".into(), "1 E:".into(), "_Total".into()]
        ));
    }

    #[test]
    fn active_time_is_the_complement_of_idle() {
        assert!((active_pct(99.9).unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(active_pct(120.0), Some(0.0));
        assert_eq!(active_pct(f64::NAN), None);
    }

    #[test]
    fn volume_usage() {
        assert_eq!(used_pct(200, 50), Some(75.0));
        assert_eq!(used_pct(0, 0), None);
    }

    #[test]
    fn volume_identity_change_forces_rediscover() {
        assert!(!volume_identity_changed("guid-a", Some("guid-a")));
        assert!(volume_identity_changed("guid-a", Some("guid-b")));
        assert!(volume_identity_changed("guid-a", None));
    }

    #[test]
    fn fresh_flag_is_consumed_by_the_first_poll_only() {
        let mut fresh = true;
        assert!(take_fresh(&mut fresh));
        assert!(!take_fresh(&mut fresh));
        assert!(!take_fresh(&mut fresh));
    }

    #[test]
    fn sleeping_disk_is_retried_and_new_indices_request_discovery() {
        use crate::storage_temperature::TEMPERATURE_PERIOD;
        use std::time::Duration;
        let start = Instant::now();
        let mut disk = DiskTemperatures {
            positions: vec![],
            values: vec![],
            read_at: start,
        };
        let first = start + TEMPERATURE_PERIOD;
        assert_eq!(next_refresh([(0, disk.read_at)], first), Some(0));
        assert!(!disk.refresh(None, first)); // still asleep / transient failure
        assert_eq!(
            next_refresh([(0, disk.read_at)], first + Duration::from_secs(1)),
            None
        );
        let awake = TemperatureReport {
            sensors: BTreeMap::from([(0, Some(42.0))]),
            warning_c: None,
            critical_c: None,
        };
        assert_eq!(
            next_refresh([(0, disk.read_at)], first + TEMPERATURE_PERIOD),
            Some(0)
        );
        assert!(disk.refresh(Some(&awake), first + TEMPERATURE_PERIOD));
        assert!(disk.values.is_empty(), "schema changes only in discover");
        disk.positions = vec![0]; // subsequent discovery declares the new sensor
        assert!(!disk.refresh(Some(&awake), first + TEMPERATURE_PERIOD));
        assert_eq!(disk.values, vec![Some(42.0)]);
        assert!(!disk.refresh(None, first + TEMPERATURE_PERIOD));
        assert_eq!(disk.values, vec![None]);
    }

    fn entry(index: u32) -> DriveEntry {
        DriveEntry::new(
            index,
            format!("storage/device-{index}"),
            Some("Model".to_owned()),
            Some(format!("SN{index}")),
        )
    }

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

    #[test]
    fn disk_names_list_volumes() {
        assert_eq!(
            disk_name(&parse_disk_instance("0 C: D:").unwrap()),
            "Disk 0 (C:, D:)"
        );
        assert_eq!(disk_name(&parse_disk_instance("1").unwrap()), "Disk 1");
    }
}
