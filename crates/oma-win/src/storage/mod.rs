//! Physical disk throughput and activity (PDH), disk temperatures, the NVMe
//! health log and volume usage.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError, Quality};
use oma_ipc::DriveState;
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{
    BusTypeAta, BusTypeFileBackedVirtual, BusTypeNvme, BusTypeSata, BusTypeUsb, BusTypeVirtual,
    GetDiskFreeSpaceExW,
};

use crate::memory::used_pct;
use crate::pdh::{Counter, Query};
use crate::storage_gate::{
    disk_class, local_read, plan, power, Activity, LocalRead, Plan, ServiceDisk, ServiceTemperature,
};
pub use crate::storage_gate::{DiskClass, DiskPower};
use crate::storage_health::{
    bus_type, health_properties, health_sensors, next_health_refresh, read_health, DiskHealth,
    HealthRefresh, Stamp,
};
use crate::storage_identity::{
    assign_disk_ids, descriptor_texts, disk_identity_candidates, volume_identity,
    DiskIdentityCandidates,
};
use crate::storage_ioctl::PhysicalDrive;
use crate::storage_temperature::{
    declared_positions, declared_values, may_query, next_refresh, query_temperatures, sensor_label,
    sensor_name, temperature_properties, TemperatureReport,
};
use crate::svc::drives::service_disk;
use crate::svc::feed::{FeedView, SvcFeed};

mod disk_gate;
#[cfg(test)]
mod feed_tests;
mod tables;
mod temperatures;

use disk_gate::{
    disk_states, gates_by_id, powered_on, read_temperatures, refresh_one, seek_penalty, DiskGate,
    Reading,
};
pub(crate) use tables::disk_properties;
pub use tables::{
    core_id_for_key, drive_keys_for, DiskStateTable, DriveEntry, DriveIdTable, DriveIds,
    SMART_DEFAULT, SMART_SELECTABLE,
};
use tables::{owns_main_temperature, smart_default_off};
use temperatures::{snapshot_id, DiskTemperatures};

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

/// `(total, free)` bytes of a volume such as "C:"; `None` if unavailable.
fn volume_space(volume: &str) -> Option<(u64, u64)> {
    let root = HSTRING::from(format!("{volume}\\"));
    let (mut total, mut free) = (0u64, 0u64);
    // SAFETY: valid root path and out-pointers.
    unsafe { GetDiskFreeSpaceExW(&root, None, Some(&mut total), Some(&mut free)) }.ok()?;
    Some((total, free))
}

struct Counters {
    query: Query,
    read: Counter,
    write: Counter,
    idle: Counter,
}

pub struct StorageProvider {
    drives: DriveIdTable,
    disk_states: DiskStateTable,
    /// The service's feed: the state of each disk and its main temperature.
    feed: SvcFeed,
    counters: Option<Counters>,
    disks: Vec<DiskInstance>,
    disk_ids: HashMap<u32, String>,
    volume_ids: HashMap<String, String>,
    /// Every identified disk, including those waiting for temperature
    /// support, wake or activity.
    gates: HashMap<u32, DiskGate>,
    /// NVMe disks whose health log is read, including those waiting for a retry.
    health: HashMap<u32, DiskHealth>,
    /// Set by `discover`; consumed by the next `poll`. See `take_fresh`.
    fresh: bool,
    /// The quality of each value of the last successful poll.
    quality: Option<Vec<Quality>>,
}

impl Default for StorageProvider {
    fn default() -> Self {
        Self::new(
            DriveIdTable::default(),
            DiskStateTable::default(),
            SvcFeed::default(),
        )
    }
}

impl StorageProvider {
    pub fn new(drives: DriveIdTable, disk_states: DiskStateTable, feed: SvcFeed) -> Self {
        Self {
            drives,
            disk_states,
            feed,
            counters: None,
            disks: Vec::new(),
            disk_ids: HashMap::new(),
            volume_ids: HashMap::new(),
            gates: HashMap::new(),
            health: HashMap::new(),
            fresh: false,
            quality: None,
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
        // From here on nothing fails: the gate states move to the new discovery.
        let mut previous = gates_by_id(std::mem::take(&mut self.gates), &self.disk_ids);
        let mut gates = HashMap::new();
        let mut health = HashMap::new();
        let mut drive_entries = Vec::new();
        for disk in &disks {
            // assign_disk_ids has already logged why a disk has no identity.
            let Some(id) = disk_ids.get(&disk.index).cloned() else {
                continue;
            };
            let (model, serial) = descriptor_texts(disk.index);
            let bus = bus_type(disk.index);
            // A disk that may be rotational is not queried here (the query
            // wakes it up): it keeps what it had under the same id.
            // Unknown/asleep/idle disks remain scheduled; a later successful
            // probe requests rediscovery when it reveals undeclared sensor
            // indices.
            let gate = DiskGate::discover(
                previous.remove(&id),
                || disk_class(bus, seek_penalty(disk.index)),
                Instant::now(),
                || read_temperatures(disk.index),
            );
            let entry = DriveEntry {
                smart_default_off: smart_default_off(bus),
                owns_main_temperature: owns_main_temperature(gate.class),
                ..DriveEntry::new(disk.index, id.clone(), model, serial)
            };
            let report = gate.temperatures.report.as_ref();
            // Only an NVMe disk is sent the health log query; a transient
            // failure keeps it scheduled, and its first successful read
            // requests the rediscovery that declares the sensors.
            let disk_health = DiskHealth::discover(bus, || read_health(disk.index), Stamp::now());
            let declared_health = disk_health.as_ref().filter(|h| h.declared());
            let mut properties = disk_properties(report, &entry);
            properties.extend(health_properties(
                declared_health.and_then(DiskHealth::cached),
            ));
            devices.push(Device {
                id: id.clone(),
                kind: DeviceKind::Storage,
                name: disk_name(disk),
                vendor: None,
                properties,
            });
            drive_entries.push(entry);
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
            for &position in &gate.temperatures.positions {
                sensors.push(Sensor::new(
                    &id,
                    SensorKind::Temperature,
                    &sensor_name(position),
                    Unit::Celsius,
                    sensor_label(position),
                    Source::Win32,
                ));
            }
            if declared_health.is_some() {
                sensors.extend(health_sensors(&id, bus));
            }
            if let Some(disk_health) = disk_health {
                health.insert(disk.index, disk_health);
            }
            gates.insert(disk.index, gate);
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
        self.gates = gates;
        self.health = health;
        self.fresh = true;
        self.quality = None;
        self.drives.publish(drive_entries);
        // A removed disk leaves the table now, not at the next good poll.
        self.disk_states
            .publish(disk_states(&self.disks, &self.disk_ids, &self.gates));
        Ok(Inventory { devices, sensors })
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        self.quality = None;
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
        let now = Stamp::now();
        // One view of the feed for the whole poll, whatever `svc` reads on
        // its own tick.
        let view = self.feed.view();
        let drives = self.drives.get();
        // The gate: what the service says and this poll's I/O decide which
        // disks may be read at all. The rates of the first poll after a
        // discovery are no sample.
        let mut undeclared = false;
        for disk in &self.disks {
            let Some(gate) = self.gates.get_mut(&disk.index) else {
                continue;
            };
            let entry = drives.drives.iter().find(|e| e.index == disk.index);
            let rates = (!fresh).then(|| {
                (
                    finite(&read, &disk.instance),
                    finite(&write, &disk.instance),
                )
            });
            undeclared |= gate.step(rates, powered_on(disk.index), entry, &drives, &view, &now);
        }
        self.disk_states
            .publish(disk_states(&self.disks, &self.disk_ids, &self.gates));
        // The service's first measure of a disk declares its main sensor.
        if undeclared {
            return Err(ProviderError::Rediscover);
        }
        let (picked, rediscover) = refresh_one(&mut self.gates, now.mono, read_temperatures);
        if rediscover {
            return Err(ProviderError::Rediscover);
        }
        // Same rotation: an NVMe disk's health log is read with its
        // temperatures, while the disk is awake anyway. Only a poll without a
        // temperature refresh reads a health log on its own (after a suspend
        // the monotonic clock did not count), so a poll never queries two disks.
        let health_pick = match picked {
            Some(index) => self.health.contains_key(&index).then_some(index),
            None => next_health_refresh(self.health.iter().map(|(&i, h)| (i, h)), &now),
        };
        if let Some(index) = health_pick {
            if let Some(disk) = self.health.get_mut(&index) {
                match disk.refresh(read_health(index), Stamp::now()) {
                    HealthRefresh::Keep => {}
                    HealthRefresh::Rediscover => return Err(ProviderError::Rediscover),
                    HealthRefresh::Forget => {
                        self.health.remove(&index);
                    }
                }
            }
        }
        let now = Stamp::now();

        let mut reading = Reading::default();
        for disk in &self.disks {
            if !self.disk_ids.contains_key(&disk.index) {
                continue;
            }
            if fresh {
                // Prime the PDH rate counters but report no value yet.
                reading.fresh([None, None, None]);
            } else {
                reading.fresh([
                    finite(&read, &disk.instance),
                    finite(&write, &disk.instance),
                    finite(&idle, &disk.instance).and_then(active_pct),
                ]);
            }
            // Not a rate: the last read is valid on the first poll too.
            if let Some(gate) = self.gates.get_mut(&disk.index) {
                reading.temperatures(gate);
            }
            if let Some(health) = self.health.get(&disk.index).and_then(|h| h.values(&now)) {
                reading.fresh(health);
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
                        reading.fresh([used_pct(total, free), Some(free as f64)]);
                    }
                    None => reading.fresh([None, None]),
                }
            }
        }
        self.quality = Some(reading.quality);
        Ok(reading.values)
    }

    fn quality(&self) -> Option<Vec<Quality>> {
        self.quality.clone()
    }
}

/// Kind of a physical disk for the benchmark estimate and warnings (DC14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskKind {
    Nvme,
    SataSsd,
    Hdd,
    Usb,
    Virtual,
    Other,
}

/// Pure mapping from the storage bus type and the seek-penalty flag.
pub fn disk_kind(bus: Option<i32>, seek_penalty: Option<bool>) -> DiskKind {
    match bus {
        Some(b) if b == BusTypeNvme.0 => DiskKind::Nvme,
        Some(b) if b == BusTypeUsb.0 => DiskKind::Usb,
        Some(b) if b == BusTypeVirtual.0 || b == BusTypeFileBackedVirtual.0 => DiskKind::Virtual,
        _ if seek_penalty == Some(true) => DiskKind::Hdd,
        Some(b) if (b == BusTypeAta.0 || b == BusTypeSata.0) && seek_penalty != Some(true) => {
            DiskKind::SataSsd
        }
        _ => DiskKind::Other,
    }
}

/// What disk `index` is, from two metadata queries (no data read, no SMART).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskTraits {
    pub bus: Option<i32>,
    pub class: DiskClass,
    pub kind: DiskKind,
}

pub fn disk_traits(index: u32) -> DiskTraits {
    let bus = bus_type(index);
    let penalty = seek_penalty(index);
    DiskTraits {
        bus,
        class: disk_class(bus, penalty),
        kind: disk_kind(bus, penalty),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_kinds_follow_bus_and_seek_penalty() {
        use DiskKind::*;
        assert_eq!(disk_kind(Some(17), Some(false)), Nvme);
        assert_eq!(disk_kind(Some(17), None), Nvme);
        assert_eq!(disk_kind(Some(7), Some(true)), Usb);
        assert_eq!(disk_kind(Some(14), None), Virtual);
        assert_eq!(disk_kind(Some(15), Some(true)), Virtual);
        assert_eq!(disk_kind(Some(3), Some(true)), Hdd);
        assert_eq!(disk_kind(Some(11), Some(true)), Hdd);
        assert_eq!(disk_kind(Some(3), Some(false)), SataSsd);
        assert_eq!(disk_kind(Some(11), None), SataSsd);
        assert_eq!(disk_kind(None, Some(true)), Hdd);
        assert_eq!(disk_kind(None, Some(false)), Other);
        assert_eq!(disk_kind(None, None), Other);
        assert_eq!(disk_kind(Some(8), Some(false)), Other);
        assert_eq!(serde_json::to_string(&SataSsd).unwrap(), "\"sata_ssd\"");
    }

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
            read_at: Some(start),
            ..Default::default()
        };
        let first = start + TEMPERATURE_PERIOD;
        assert_eq!(next_refresh([(0, start)], first), Some(0));
        assert!(!disk.refresh(None, first)); // still asleep / transient failure
        assert_eq!(disk.read_at, Some(first));
        assert_eq!(
            next_refresh([(0, first)], first + Duration::from_secs(1)),
            None
        );
        let awake = TemperatureReport {
            sensors: BTreeMap::from([(0, Some(42.0))]),
            warning_c: None,
            critical_c: None,
        };
        assert_eq!(
            next_refresh([(0, first)], first + TEMPERATURE_PERIOD),
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

    #[test]
    fn disk_names_list_volumes() {
        assert_eq!(
            disk_name(&parse_disk_instance("0 C: D:").unwrap()),
            "Disk 0 (C:, D:)"
        );
        assert_eq!(disk_name(&parse_disk_instance("1").unwrap()), "Disk 1");
    }
}
