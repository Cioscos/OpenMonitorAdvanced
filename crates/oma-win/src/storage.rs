//! Physical disk throughput and activity (PDH), disk temperatures, the NVMe
//! health log and volume usage.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError, Quality};
use oma_ipc::DriveState;
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{BusTypeUsb, GetDiskFreeSpaceExW};

use crate::pdh::{Counter, Query};
pub use crate::storage_gate::DiskPower;
use crate::storage_gate::{
    disk_class, plan, power, Activity, DiskClass, Plan, ServiceDisk, ServiceTemperature,
};
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
#[derive(Default)]
struct DiskTemperatures {
    positions: Vec<usize>,
    values: Vec<Option<f64>>,
    /// The last attempt; `None` for a disk never queried, due at once.
    read_at: Option<Instant>,
    /// The last successful answer: the limits, and the sensors the next
    /// discovery declares without querying the disk again.
    report: Option<TemperatureReport>,
    /// The values come from a read no poll has published yet.
    unpublished: bool,
    /// The last main temperature taken from the service.
    imported: Option<Imported>,
}

/// A main temperature taken from the service: which disk it was measured on
/// (its wire key) and the snapshot that carried it.
struct Imported {
    value: f64,
    key: String,
    snapshot: SnapshotId,
    /// No poll has published it yet.
    unpublished: bool,
}

impl DiskTemperatures {
    /// The state after a read at discovery, as before the gate: the sensors
    /// are the ones this answer reports.
    fn read(report: Option<TemperatureReport>, now: Instant) -> Self {
        let positions = report.as_ref().map(declared_positions).unwrap_or_default();
        Self {
            values: declared_values(report.as_ref(), &positions),
            positions,
            read_at: Some(now),
            report,
            unpublished: true,
            imported: None,
        }
    }

    /// Refreshes values and the attempt deadline, including failed/asleep reads.
    /// New driver indices require a schema rebuild, never a value-vector resize.
    fn refresh(&mut self, report: Option<&TemperatureReport>, now: Instant) -> bool {
        self.values = declared_values(report, &self.positions);
        self.read_at = Some(now);
        self.unpublished = true;
        let Some(report) = report else {
            return false;
        };
        self.report = Some(report.clone());
        declared_positions(report)
            .iter()
            .any(|i| !self.positions.contains(i))
    }

    /// The state a discovery that does not query the disk starts from: the
    /// sensors already declared keep their values, the indices the last
    /// answer revealed are declared with the values it carried, and a
    /// temperature taken from the service declares the main one.
    fn redeclared(mut self) -> Self {
        let mut sensors: BTreeMap<usize, Option<f64>> =
            self.positions.iter().copied().zip(self.values).collect();
        if let Some(report) = &self.report {
            for position in declared_positions(report) {
                sensors
                    .entry(position)
                    .or_insert_with(|| report.sensors.get(&position).copied().flatten());
            }
        }
        if let Some(imported) = &self.imported {
            sensors.entry(MAIN).or_insert(Some(imported.value));
        }
        (self.positions, self.values) = sensors.into_iter().unzip();
        self
    }

    /// Takes `temperature`, measured by the service on the disk with this
    /// `key` and carried by `snapshot`, as the main temperature. The local
    /// deadline is not touched. Returns whether the sensor is still to be
    /// declared, by a rediscovery.
    fn import(&mut self, temperature: ServiceTemperature, key: &str, snapshot: SnapshotId) -> bool {
        let known = self
            .imported
            .as_ref()
            .is_some_and(|imported| imported.snapshot == snapshot);
        if !known {
            self.imported = Some(Imported {
                value: temperature.value,
                key: key.to_owned(),
                snapshot,
                unpublished: true,
            });
        }
        match self.positions.iter().position(|&p| p == MAIN) {
            Some(slot) => {
                self.values[slot] = Some(temperature.value);
                false
            }
            None => true,
        }
    }

    /// Forgets every value when the last one taken from the service was
    /// measured on a disk with another key: nothing is shown for an identity
    /// it was not measured on. The sensors stay declared.
    fn forget_another_disk(&mut self, key: Option<&str>) {
        let other = self
            .imported
            .as_ref()
            .is_some_and(|imported| Some(imported.key.as_str()) != key);
        if other {
            *self = Self {
                values: vec![None; self.positions.len()],
                positions: std::mem::take(&mut self.positions),
                ..Self::default()
            };
        }
    }
}

/// The driver index of a disk's main temperature (`temperature/drive`).
const MAIN: usize = 0;

/// One snapshot of the service: the generation of the feed and the `seq`
/// within it.
type SnapshotId = (u64, u64);

fn snapshot_id(view: &FeedView) -> Option<SnapshotId> {
    let (_, snapshot) = view.snapshot.as_ref()?;
    Some((view.generation, snapshot.seq))
}

/// What the gate knows about one disk. It follows the disk across
/// discoveries for as long as its device id stays the same.
struct DiskGate {
    class: DiskClass,
    activity: Activity,
    temperatures: DiskTemperatures,
    /// The decision of the last poll.
    plan: Plan,
    power: DiskPower,
}

impl DiskGate {
    /// The state after a discovery. `previous` is the same disk (same device
    /// id) before it; `class` is asked only for a disk seen for the first
    /// time. Only a non-rotational disk is `read`, as before the gate: any
    /// other keeps what it had and gets its sensors from its first
    /// authorized read, through a rediscovery.
    fn discover(
        previous: Option<DiskGate>,
        class: impl FnOnce() -> DiskClass,
        now: Instant,
        read: impl FnOnce() -> Option<TemperatureReport>,
    ) -> Self {
        // A known disk keeps its last state until the next poll; a new one
        // has none yet.
        let (class, activity, power, carried) = match previous {
            Some(gate) => (
                gate.class,
                gate.activity,
                gate.power,
                Some(gate.temperatures),
            ),
            None => (class(), Activity::default(), DiskPower::Unknown, None),
        };
        let temperatures = match class {
            DiskClass::NonRotational => DiskTemperatures::read(read(), now),
            DiskClass::RotationalOrUnknown => carried
                .map(DiskTemperatures::redeclared)
                .unwrap_or_default(),
        };
        Self {
            class,
            activity,
            temperatures,
            plan: Plan::Wait,
            power,
        }
    }

    /// Takes this poll's PDH rates (`None` when missing) and decides where
    /// the temperature comes from.
    fn observe(
        &mut self,
        read: Option<f64>,
        write: Option<f64>,
        powered_on: Option<bool>,
        service: &ServiceDisk,
        now: &Stamp,
    ) {
        self.activity.observe(read, write, now);
        self.decide(powered_on, service, now);
    }

    /// The first poll after a discovery: its PDH rates are not a sample, so
    /// the activity window is neither opened nor closed, and only ages.
    fn warm_up(&mut self, powered_on: Option<bool>, service: &ServiceDisk, now: &Stamp) {
        self.decide(powered_on, service, now);
    }

    fn decide(&mut self, powered_on: Option<bool>, service: &ServiceDisk, now: &Stamp) {
        let recent = self.activity.recent(now);
        self.plan = plan(self.class, powered_on, service, recent);
        self.power = power(self.class, powered_on, service, recent);
    }

    /// After this poll's decision: takes the main temperature the service
    /// has for this disk, whose wire key is `key`, from the snapshot
    /// `snapshot`. A measure of an active disk is taken as it is; the value
    /// the service keeps for a disk in standby or idle is a last reading. A
    /// drive without media or blocking the gate in an unknown state gives
    /// nothing. Returns whether the main sensor is still to be declared, by a
    /// rediscovery; the disk is never queried for it.
    fn adopt(
        &mut self,
        service: &ServiceDisk,
        key: Option<&str>,
        snapshot: Option<SnapshotId>,
    ) -> bool {
        self.temperatures.forget_another_disk(key);
        // A non-rotational disk is read at every discovery, as before the gate.
        if self.class == DiskClass::NonRotational {
            return false;
        }
        let temperature = match (self.plan, self.power, service) {
            (Plan::Service(temperature), _, _) => Some(temperature),
            (
                Plan::Wait,
                DiskPower::Standby | DiskPower::Idle,
                ServiceDisk::Present {
                    state: DriveState::Standby | DriveState::Idle,
                    temperature,
                    ..
                },
            ) => *temperature,
            _ => None,
        };
        match (temperature, key, snapshot) {
            (Some(temperature), Some(key), Some(snapshot)) => {
                self.temperatures.import(temperature, key, snapshot)
            }
            _ => false,
        }
    }

    /// The temperature values of this poll with their quality. A read
    /// suspended on purpose (idle, standby) repeats the last values as
    /// `Suspended`; a disk that waits for any other reason has no value. A
    /// value measured by an earlier poll is `Held`. A value that is absent
    /// (never measured, or a failed read) is just absent, whatever the state.
    ///
    /// The service's measure is the main temperature: `Fresh` on the first
    /// poll that takes a new one, `Held` when the service itself kept it or
    /// the same snapshot is read again. The additional sensors are the
    /// core's alone and nobody reads them meanwhile: last readings.
    fn published(&mut self) -> Vec<(Option<f64>, Quality)> {
        let measured = std::mem::take(&mut self.temperatures.unpublished);
        let adopted = self
            .temperatures
            .imported
            .as_mut()
            .is_some_and(|imported| std::mem::take(&mut imported.unpublished));
        let last_reading = |value: Option<f64>| match value {
            Some(_) => (value, Quality::Suspended),
            None => (None, Quality::Fresh),
        };
        let positions = self.temperatures.positions.iter().copied();
        let values = self.temperatures.values.iter().copied();
        match (self.plan, self.power) {
            (Plan::Wait, DiskPower::Idle | DiskPower::Standby) => {
                values.map(last_reading).collect()
            }
            (Plan::Wait, _) => values.map(|_| (None, Quality::Fresh)).collect(),
            (Plan::Service(temperature), _) => positions
                .zip(values)
                .map(|(position, value)| {
                    if position != MAIN {
                        last_reading(value)
                    } else if temperature.held || !adopted {
                        (Some(temperature.value), Quality::Held)
                    } else {
                        (Some(temperature.value), Quality::Fresh)
                    }
                })
                .collect(),
            (Plan::Local, _) => values
                .map(|value| match value {
                    Some(_) if !measured => (value, Quality::Held),
                    _ => (value, Quality::Fresh),
                })
                .collect(),
        }
    }
}

/// What `DiskStateTable` publishes: the state of every identified disk, in
/// disk order.
fn disk_states(
    disks: &[DiskInstance],
    ids: &HashMap<u32, String>,
    gates: &HashMap<u32, DiskGate>,
) -> Vec<(String, DiskPower)> {
    disks
        .iter()
        .filter_map(|disk| {
            let id = ids.get(&disk.index)?;
            Some((id.clone(), gates.get(&disk.index)?.power))
        })
        .collect()
}

/// The gate states of a discovery by device id, for the next one to pick up.
fn gates_by_id(
    gates: HashMap<u32, DiskGate>,
    ids: &HashMap<u32, String>,
) -> HashMap<String, DiskGate> {
    gates
        .into_iter()
        .filter_map(|(index, gate)| Some((ids.get(&index)?.clone(), gate)))
        .collect()
}

/// Reads the temperatures of at most one disk: among those whose plan is a
/// local read, the one never read (lowest index first), else the oldest due
/// read. A disk that waits is skipped without renewing its deadline, so it
/// stays due and does not hold back the others. Returns the disk read and
/// whether its answer asks for a rediscovery.
fn refresh_one(
    gates: &mut HashMap<u32, DiskGate>,
    now: Instant,
    read: impl FnOnce(u32) -> Option<TemperatureReport>,
) -> (Option<u32>, bool) {
    let authorized = || {
        gates
            .iter()
            .filter(|(_, gate)| gate.plan == Plan::Local)
            .map(|(&index, gate)| (index, gate.temperatures.read_at))
    };
    let never_read = authorized()
        .filter(|(_, read_at)| read_at.is_none())
        .map(|(index, _)| index)
        .min();
    let picked = never_read.or_else(|| {
        let reads = authorized().filter_map(|(index, read_at)| Some((index, read_at?)));
        next_refresh(reads, now)
    });
    let Some(gate) = picked.and_then(|index| gates.get_mut(&index)) else {
        return (None, false);
    };
    let report = picked.and_then(read);
    (picked, gate.temperatures.refresh(report.as_ref(), now))
}

/// The values of a poll with the quality of each.
#[derive(Default)]
struct Reading {
    values: Vec<Option<f64>>,
    quality: Vec<Quality>,
}

impl Reading {
    /// Values measured by this poll (or absent).
    fn fresh(&mut self, values: impl IntoIterator<Item = Option<f64>>) {
        for value in values {
            self.values.push(value);
            self.quality.push(Quality::Fresh);
        }
    }

    fn temperatures(&mut self, gate: &mut DiskGate) {
        for (value, quality) in gate.published() {
            self.values.push(value);
            self.quality.push(quality);
        }
    }
}

/// Temperatures of disk `index`. A disk known to be spun down is not queried,
/// because the query could wake it up: `None`, as for an unsupported disk.
fn read_temperatures(index: u32) -> Option<TemperatureReport> {
    PhysicalDrive::open(index)
        .filter(|drive| may_query(drive.powered_on()))
        .and_then(|drive| query_temperatures(&drive))
}

/// Whether disk `index` incurs a seek penalty; `None` when it does not say.
fn seek_penalty(index: u32) -> Option<bool> {
    PhysicalDrive::open(index)?.seek_penalty()
}

/// `Some(false)` when Windows switched disk `index` off. Asking does not
/// reach the disk.
fn powered_on(index: u32) -> Option<bool> {
    PhysicalDrive::open(index)?.powered_on()
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
    /// The service leaves this disk's SMART off unless a client asks for it:
    /// a disk on the USB bus (spec M6b §4.2).
    pub smart_default_off: bool,
}

impl DriveEntry {
    /// An entry whose `key` follows from `model` and `serial`, with SMART on
    /// by default.
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
fn smart_default_off(bus_type: Option<i32>) -> bool {
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

/// The current state of every identified disk, by core device id. Shared
/// handle: written by `StorageProvider::poll`, read by the shell. Cheap to
/// clone.
#[derive(Clone, Default)]
pub struct DiskStateTable(Arc<Mutex<DiskStates>>);

#[derive(Default)]
struct DiskStates {
    generation: u64,
    states: Vec<(String, DiskPower)>,
}

impl DiskStateTable {
    fn lock(&self) -> std::sync::MutexGuard<'_, DiskStates> {
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
            let entry = DriveEntry {
                smart_default_off: smart_default_off(bus),
                ..DriveEntry::new(disk.index, id.clone(), model, serial)
            };
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
        let snapshot = snapshot_id(&view);
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
            let service = entry.map_or(ServiceDisk::Absent, |entry| {
                service_disk(entry, &drives, &view, now.mono)
            });
            let powered_on = powered_on(disk.index);
            if fresh {
                gate.warm_up(powered_on, &service, &now);
            } else {
                gate.observe(
                    finite(&read, &disk.instance),
                    finite(&write, &disk.instance),
                    powered_on,
                    &service,
                    &now,
                );
            }
            let key = entry.and_then(|entry| entry.key.as_deref());
            undeclared |= gate.adopt(&service, key, snapshot);
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

    use crate::storage_temperature::TEMPERATURE_PERIOD;
    use std::time::{Duration, SystemTime};
    use DiskClass::{NonRotational, RotationalOrUnknown};

    const BUSY: (Option<f64>, Option<f64>) = (Some(4096.0), Some(0.0));
    const QUIET: (Option<f64>, Option<f64>) = (Some(0.0), Some(0.0));

    /// `seconds` after `start` on both clocks.
    fn stamp(start: Instant, seconds: u64) -> Stamp {
        Stamp {
            mono: start + Duration::from_secs(seconds),
            wall: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + seconds),
        }
    }

    fn report(sensors: &[(usize, f64)]) -> TemperatureReport {
        TemperatureReport {
            sensors: sensors.iter().map(|&(i, c)| (i, Some(c))).collect(),
            warning_c: Some(60),
            critical_c: None,
        }
    }

    /// A disk whose only temperature sensor read `celsius` at `read_at`,
    /// published since.
    fn gate(class: DiskClass, celsius: f64, read_at: Instant) -> DiskGate {
        DiskGate {
            class,
            activity: Activity::default(),
            temperatures: DiskTemperatures {
                positions: vec![0],
                values: vec![Some(celsius)],
                read_at: Some(read_at),
                report: Some(report(&[(0, celsius)])),
                unpublished: false,
                imported: None,
            },
            plan: Plan::Wait,
            power: DiskPower::Unknown,
        }
    }

    /// One poll of `gate` without the service: `rates` are its PDH rates.
    fn observe(gate: &mut DiskGate, rates: (Option<f64>, Option<f64>), now: &Stamp) {
        gate.observe(rates.0, rates.1, Some(true), &ServiceDisk::Absent, now);
    }

    #[test]
    fn an_idle_hdd_is_not_picked_and_does_not_starve_other_disks() {
        let start = Instant::now();
        // Both due; the idle one has the older read and would be first in line.
        let mut gates = HashMap::from([
            (0, gate(RotationalOrUnknown, 35.0, start)),
            (
                2,
                gate(RotationalOrUnknown, 41.0, start + Duration::from_secs(5)),
            ),
        ]);
        let now = stamp(start, 40);
        observe(gates.get_mut(&0).unwrap(), QUIET, &now);
        observe(gates.get_mut(&2).unwrap(), BUSY, &now);
        assert_eq!(gates[&0].plan, Plan::Wait);
        assert_eq!(gates[&2].plan, Plan::Local);

        let outcome = refresh_one(&mut gates, now.mono, |index| {
            assert_eq!(index, 2, "the idle disk must not be queried");
            Some(report(&[(0, 43.0)]))
        });
        assert_eq!(outcome, (Some(2), false));
        assert_eq!(gates[&2].temperatures.values, vec![Some(43.0)]);
        assert_eq!(gates[&2].temperatures.read_at, Some(now.mono));
        assert_eq!(gates[&0].temperatures.values, vec![Some(35.0)]);
        assert_eq!(gates[&0].temperatures.read_at, Some(start));

        // The busy disk is not due again and the idle one still waits: no query.
        let later = stamp(start, 41);
        observe(gates.get_mut(&0).unwrap(), QUIET, &later);
        observe(gates.get_mut(&2).unwrap(), BUSY, &later);
        let outcome = refresh_one(&mut gates, later.mono, |index| {
            panic!("disk {index} queried")
        });
        assert_eq!(outcome, (None, false));
    }

    #[test]
    fn an_idle_or_standby_disk_keeps_its_last_values_and_stays_due() {
        let start = Instant::now();
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        let no_query = |index: u32| -> Option<TemperatureReport> { panic!("disk {index} queried") };
        let absent = ServiceDisk::Absent;

        // Idle: no I/O, as far as Windows knows the disk is on.
        let idle = stamp(start, 31);
        let disk = gates.get_mut(&0).unwrap();
        disk.observe(QUIET.0, QUIET.1, Some(true), &absent, &idle);
        assert_eq!(disk.power, DiskPower::Idle);
        assert_eq!(refresh_one(&mut gates, idle.mono, no_query), (None, false));
        assert_eq!(
            gates.get_mut(&0).unwrap().published(),
            vec![(Some(35.0), Quality::Suspended)]
        );

        // Standby: Windows switched the disk off, even while I/O is reported.
        let standby = stamp(start, 32);
        let disk = gates.get_mut(&0).unwrap();
        disk.observe(BUSY.0, BUSY.1, Some(false), &absent, &standby);
        assert_eq!(disk.power, DiskPower::Standby);
        assert_eq!(
            refresh_one(&mut gates, standby.mono, no_query),
            (None, false)
        );
        assert_eq!(
            gates.get_mut(&0).unwrap().published(),
            vec![(Some(35.0), Quality::Suspended)]
        );
        assert_eq!(gates[&0].temperatures.values, vec![Some(35.0)]);
        assert_eq!(gates[&0].temperatures.read_at, Some(start), "still due");

        // Waiting for a reason that is not an expected suspension (no media, a
        // disk blocking the SMART gate in an unknown state): no value at all,
        // and not passed off as suspended. The cache itself is untouched.
        let unknown = stamp(start, 33);
        for service in [
            ServiceDisk::Present {
                state: DriveState::NoMedia,
                blocks_smart: false,
                temperature: None,
            },
            ServiceDisk::Present {
                state: DriveState::Unknown,
                blocks_smart: true,
                temperature: None,
            },
        ] {
            let disk = gates.get_mut(&0).unwrap();
            disk.observe(BUSY.0, BUSY.1, Some(true), &service, &unknown);
            assert_eq!((disk.plan, disk.power), (Plan::Wait, DiskPower::Unknown));
            assert_eq!(disk.published(), vec![(None, Quality::Fresh)]);
            assert_eq!(disk.temperatures.values, vec![Some(35.0)]);
            assert_eq!(disk.temperatures.read_at, Some(start));
        }
        assert_eq!(
            refresh_one(&mut gates, unknown.mono, no_query),
            (None, false)
        );

        // The overdue read starts on the first poll that authorizes it.
        let busy = stamp(start, 34);
        observe(gates.get_mut(&0).unwrap(), BUSY, &busy);
        let outcome = refresh_one(&mut gates, busy.mono, |_| Some(report(&[(0, 36.0)])));
        assert_eq!(outcome, (Some(0), false));
        assert_eq!(
            gates.get_mut(&0).unwrap().published(),
            vec![(Some(36.0), Quality::Fresh)]
        );
        assert_eq!(gates[&0].temperatures.read_at, Some(busy.mono));
    }

    #[test]
    fn a_failed_authorized_read_gives_absent_values() {
        let start = Instant::now();
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        let now = stamp(start, 30);
        observe(gates.get_mut(&0).unwrap(), BUSY, &now);
        assert_eq!(gates[&0].plan, Plan::Local);
        assert_eq!(
            refresh_one(&mut gates, now.mono, |_| None),
            (Some(0), false)
        );
        assert_eq!(gates[&0].temperatures.values, vec![None]);
        assert_eq!(gates[&0].temperatures.read_at, Some(now.mono));
        // An error is an absent value, not a suspension and not a held one.
        assert_eq!(
            gates.get_mut(&0).unwrap().published(),
            vec![(None, Quality::Fresh)]
        );
        // The failed attempt keeps the normal interval before the next one.
        let next = stamp(start, 31);
        observe(gates.get_mut(&0).unwrap(), BUSY, &next);
        let outcome = refresh_one(&mut gates, next.mono, |index| {
            panic!("disk {index} queried")
        });
        assert_eq!(outcome, (None, false));
        assert_eq!(
            gates.get_mut(&0).unwrap().published(),
            vec![(None, Quality::Fresh)]
        );
    }

    #[test]
    fn a_new_device_id_starts_without_cache() {
        let start = Instant::now();
        let mut known = gate(RotationalOrUnknown, 35.0, start);
        observe(&mut known, BUSY, &stamp(start, 1));
        let ids = HashMap::from([(0, "storage/a".to_owned())]);
        let mut previous = gates_by_id(HashMap::from([(0, known)]), &ids);
        let now = start + Duration::from_secs(2);
        let no_query = || -> Option<TemperatureReport> { panic!("queried at discovery") };

        // Another disk took index 0: nothing of the old one is inherited, and
        // a rotational disk is not queried at discovery.
        let other = DiskGate::discover(
            previous.remove("storage/b"),
            || RotationalOrUnknown,
            now,
            no_query,
        );
        assert!(other.temperatures.positions.is_empty());
        assert!(other.temperatures.values.is_empty());
        assert_eq!(other.temperatures.read_at, None);
        assert!(other.temperatures.report.is_none());
        assert!(!other.activity.recent(&stamp(start, 2)));

        // The same disk, now at another index, keeps everything; its class is
        // not asked again.
        let same = DiskGate::discover(
            previous.remove("storage/a"),
            || panic!("class asked again"),
            now,
            no_query,
        );
        assert_eq!(same.class, RotationalOrUnknown);
        assert_eq!(same.temperatures.positions, vec![0]);
        assert_eq!(same.temperatures.values, vec![Some(35.0)]);
        assert_eq!(same.temperatures.read_at, Some(start));
        assert!(same.activity.recent(&stamp(start, 2)));
        assert!(previous.is_empty());

        // A disk without an id is not carried over.
        let orphan = HashMap::from([(7, gate(RotationalOrUnknown, 35.0, start))]);
        assert!(gates_by_id(orphan, &ids).is_empty());
    }

    #[test]
    fn a_non_rotational_disk_is_read_at_discovery() {
        let start = Instant::now();
        let fresh = DiskGate::discover(
            None,
            || NonRotational,
            start,
            || Some(report(&[(0, 48.0), (2, 39.0)])),
        );
        assert_eq!(fresh.temperatures.positions, vec![0, 2]);
        assert_eq!(fresh.temperatures.values, vec![Some(48.0), Some(39.0)]);
        assert_eq!(fresh.temperatures.read_at, Some(start));
        // As before the gate, a rediscovery reads again and declares what it finds.
        let later = start + Duration::from_secs(3);
        let mut again = DiskGate::discover(
            Some(fresh),
            || panic!("class asked again"),
            later,
            || Some(report(&[(0, 49.0)])),
        );
        assert_eq!(again.temperatures.positions, vec![0]);
        assert_eq!(again.temperatures.read_at, Some(later));
        // Never waiting on activity.
        observe(&mut again, QUIET, &stamp(start, 4));
        assert_eq!((again.plan, again.power), (Plan::Local, DiskPower::Active));
        assert_eq!(again.published(), vec![(Some(49.0), Quality::Fresh)]);
    }

    #[test]
    fn a_first_read_declares_the_sensors_at_the_next_discovery() {
        let start = Instant::now();
        let no_query = || -> Option<TemperatureReport> { panic!("queried at discovery") };
        let new = DiskGate::discover(None, || RotationalOrUnknown, start, no_query);
        let mut gates = HashMap::from([(0, new)]);

        // Never read: due as soon as the disk works, without waiting a period.
        let warm_up = stamp(start, 1);
        let disk = gates.get_mut(&0).unwrap();
        disk.warm_up(Some(true), &ServiceDisk::Absent, &warm_up);
        let outcome = refresh_one(&mut gates, warm_up.mono, |index| {
            panic!("disk {index} queried")
        });
        assert_eq!(outcome, (None, false));
        let busy = stamp(start, 2);
        observe(gates.get_mut(&0).unwrap(), BUSY, &busy);
        let outcome = refresh_one(&mut gates, busy.mono, |_| Some(report(&[(0, 39.0)])));
        assert_eq!(outcome, (Some(0), true), "undeclared sensor: rediscover");
        // The poll ends there: nothing is published before the rediscovery.
        assert!(gates[&0].temperatures.values.is_empty());

        // The rediscovery declares the sensor from that report, with no query.
        let ids = HashMap::from([(0, "storage/a".to_owned())]);
        let mut declared = DiskGate::discover(
            gates_by_id(gates, &ids).remove("storage/a"),
            || panic!("class asked again"),
            busy.mono,
            no_query,
        );
        assert_eq!(declared.temperatures.positions, vec![0]);
        assert_eq!(
            declared.temperatures.report.as_ref().unwrap().warning_c,
            Some(60)
        );
        // The PDH warm-up poll that follows the rediscovery is no sample: the
        // disk still counts as working, and the measurement no poll has
        // published yet goes out as a new one.
        let after = stamp(start, 3);
        declared.warm_up(Some(true), &ServiceDisk::Absent, &after);
        assert_eq!(
            (declared.plan, declared.power),
            (Plan::Local, DiskPower::Active)
        );
        assert_eq!(declared.published(), vec![(Some(39.0), Quality::Fresh)]);
        // From the next poll on it is the same measurement.
        observe(&mut declared, BUSY, &stamp(start, 4));
        assert_eq!(declared.published(), vec![(Some(39.0), Quality::Held)]);

        // A failed read keeps the sensor declared, without resurrecting the value.
        let mut failed = gate(RotationalOrUnknown, 35.0, start);
        assert!(!failed.temperatures.refresh(None, start));
        let mut failed = DiskGate::discover(
            Some(failed),
            || panic!("class asked again"),
            start,
            no_query,
        );
        assert_eq!(failed.temperatures.positions, vec![0]);
        assert_eq!(failed.temperatures.values, vec![None]);
        // Going idle does not turn the missing value into a suspended one.
        observe(&mut failed, QUIET, &stamp(start, 1));
        assert_eq!((failed.plan, failed.power), (Plan::Wait, DiskPower::Idle));
        assert_eq!(failed.published(), vec![(None, Quality::Fresh)]);
    }

    #[test]
    fn an_absent_value_is_never_suspended() {
        let start = Instant::now();
        let absent = ServiceDisk::Absent;
        // An authorized read fails, then the disk goes idle, then to standby.
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        let now = stamp(start, 30);
        observe(gates.get_mut(&0).unwrap(), BUSY, &now);
        assert_eq!(
            refresh_one(&mut gates, now.mono, |_| None),
            (Some(0), false)
        );
        let disk = gates.get_mut(&0).unwrap();
        for (second, powered_on, power) in [
            (41, Some(true), DiskPower::Idle),
            (42, Some(false), DiskPower::Standby),
        ] {
            disk.observe(QUIET.0, QUIET.1, powered_on, &absent, &stamp(start, second));
            assert_eq!((disk.plan, disk.power), (Plan::Wait, power));
            assert_eq!(disk.published(), vec![(None, Quality::Fresh)], "{power:?}");
        }

        // A sensor declared from a report that carried it, next to one whose
        // last read failed: only the measured slot is a suspended reading.
        let mut mixed = gate(RotationalOrUnknown, 35.0, start);
        assert!(!mixed.temperatures.refresh(None, start));
        mixed.temperatures.report = Some(report(&[(0, 36.0), (1, 30.0)]));
        let mut mixed = DiskGate::discover(
            Some(mixed),
            || panic!("class asked again"),
            start,
            || panic!("queried at discovery"),
        );
        assert_eq!(mixed.temperatures.values, vec![None, Some(30.0)]);
        observe(&mut mixed, QUIET, &stamp(start, 1));
        assert_eq!(
            mixed.published(),
            vec![(None, Quality::Fresh), (Some(30.0), Quality::Suspended)]
        );
    }

    #[test]
    fn a_warm_up_poll_keeps_an_open_window_for_the_same_id() {
        let start = Instant::now();
        let absent = ServiceDisk::Absent;
        let mut disk = gate(RotationalOrUnknown, 35.0, start);
        observe(&mut disk, BUSY, &stamp(start, 1));
        // A rediscovery, then its warm-up poll: the window opened under this
        // id is neither closed nor renewed.
        let ids = HashMap::from([(0, "storage/a".to_owned())]);
        let mut disk = DiskGate::discover(
            gates_by_id(HashMap::from([(0, disk)]), &ids).remove("storage/a"),
            || panic!("class asked again"),
            start,
            || panic!("queried at discovery"),
        );
        assert_eq!(disk.power, DiskPower::Active, "no flap to unknown");
        disk.warm_up(Some(true), &absent, &stamp(start, 2));
        assert_eq!((disk.plan, disk.power), (Plan::Local, DiskPower::Active));
        // It still ages from the last real sample, on both clocks.
        disk.warm_up(Some(true), &absent, &stamp(start, 11));
        assert_eq!(disk.plan, Plan::Local);
        disk.warm_up(Some(true), &absent, &stamp(start, 12));
        assert_eq!((disk.plan, disk.power), (Plan::Wait, DiskPower::Idle));
        let mut suspended = gate(RotationalOrUnknown, 35.0, start);
        observe(&mut suspended, BUSY, &stamp(start, 1));
        let resumed = Stamp {
            wall: stamp(start, 3_600).wall,
            ..stamp(start, 2)
        };
        suspended.warm_up(Some(true), &absent, &resumed);
        assert_eq!(suspended.plan, Plan::Wait);

        // The power state is still read on a warm-up poll.
        let mut off = gate(RotationalOrUnknown, 35.0, start);
        observe(&mut off, BUSY, &stamp(start, 1));
        off.warm_up(Some(false), &absent, &stamp(start, 2));
        assert_eq!((off.plan, off.power), (Plan::Wait, DiskPower::Standby));

        // A regular poll with missing counters does close the window, and a
        // warm-up after a real gap does not bridge it.
        let mut missing = gate(RotationalOrUnknown, 35.0, start);
        observe(&mut missing, BUSY, &stamp(start, 1));
        missing.observe(None, None, Some(true), &absent, &stamp(start, 2));
        assert_eq!(missing.plan, Plan::Wait);
        let mut gap = gate(RotationalOrUnknown, 35.0, start);
        observe(&mut gap, BUSY, &stamp(start, 1));
        gap.warm_up(Some(true), &absent, &stamp(start, 2));
        observe(&mut gap, BUSY, &stamp(start, 13));
        assert_eq!(gap.plan, Plan::Wait, "the sample after a gap is a warm-up");
    }

    #[test]
    fn a_warm_up_poll_alone_never_authorizes_a_read() {
        let start = Instant::now();
        let no_query = |index: u32| -> Option<TemperatureReport> { panic!("disk {index} queried") };
        let new = DiskGate::discover(
            None,
            || RotationalOrUnknown,
            start,
            || panic!("queried at discovery"),
        );
        assert_eq!(new.power, DiskPower::Unknown);
        let mut gates = HashMap::from([
            (0, new),
            // Known, due, and quiet before the rediscovery.
            (1, gate(RotationalOrUnknown, 35.0, start)),
        ]);
        observe(gates.get_mut(&1).unwrap(), QUIET, &stamp(start, 40));
        for second in [41, 42] {
            let now = stamp(start, second);
            for disk in gates.values_mut() {
                disk.warm_up(Some(true), &ServiceDisk::Absent, &now);
                assert_eq!((disk.plan, disk.power), (Plan::Wait, DiskPower::Idle));
            }
            assert_eq!(refresh_one(&mut gates, now.mono, no_query), (None, false));
        }
    }

    #[test]
    fn disk_states_follow_the_discovered_disks() {
        let start = Instant::now();
        let disks = disk_instances(&["3 E:".into(), "0 C:".into(), "5".into()]);
        let ids = HashMap::from([
            (0, "storage/a".to_owned()),
            (3, "storage/b".to_owned()),
            // Disk 5 has no identity: no gate, no state.
        ]);
        let mut known = gate(RotationalOrUnknown, 35.0, start);
        observe(&mut known, QUIET, &stamp(start, 1));
        let new = DiskGate::discover(
            None,
            || RotationalOrUnknown,
            start,
            || panic!("queried at discovery"),
        );
        let gates = HashMap::from([(0, known), (3, new)]);
        // Disk order; a disk seen for the first time is unknown until its
        // first poll.
        let states = disk_states(&disks, &ids, &gates);
        assert_eq!(
            states,
            vec![
                ("storage/a".to_owned(), DiskPower::Idle),
                ("storage/b".to_owned(), DiskPower::Unknown),
            ]
        );
        // The next discovery no longer finds disk 0: its entry goes with it.
        let table = DiskStateTable::default();
        table.publish(states);
        let remaining = disk_instances(&["3 E:".into()]);
        table.publish(disk_states(&remaining, &ids, &gates));
        assert_eq!(
            table.get(),
            (2, vec![("storage/b".to_owned(), DiskPower::Unknown)])
        );
    }

    #[test]
    fn suspended_quality_covers_only_the_temperature_sensors() {
        let start = Instant::now();
        let mut disk = gate(RotationalOrUnknown, 35.0, start);
        let absent = ServiceDisk::Absent;
        for (second, powered_on, power) in [
            (1, Some(true), DiskPower::Idle),
            (2, None, DiskPower::Idle),
            (3, Some(false), DiskPower::Standby),
        ] {
            disk.observe(QUIET.0, QUIET.1, powered_on, &absent, &stamp(start, second));
            assert_eq!((disk.plan, disk.power), (Plan::Wait, power));
            // The sensors of one disk in schema order: read, write, active,
            // drive temperature, volume used and free.
            let mut reading = Reading::default();
            reading.fresh([Some(0.0), Some(0.0), Some(0.0)]);
            reading.temperatures(&mut disk);
            reading.fresh([Some(75.0), Some(1e9)]);
            assert_eq!(
                reading.values,
                vec![
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                    Some(35.0),
                    Some(75.0),
                    Some(1e9)
                ]
            );
            assert_eq!(
                reading.quality,
                vec![
                    Quality::Fresh,
                    Quality::Fresh,
                    Quality::Fresh,
                    Quality::Suspended,
                    Quality::Fresh,
                    Quality::Fresh
                ],
                "{power:?}"
            );
        }
    }

    #[test]
    fn cached_temperature_between_reads_is_held_while_io_is_fresh() {
        let start = Instant::now();
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        let mut poll = |second: u64, celsius: f64| {
            let now = stamp(start, second);
            observe(gates.get_mut(&0).unwrap(), BUSY, &now);
            let (picked, _) = refresh_one(&mut gates, now.mono, |_| Some(report(&[(0, celsius)])));
            let mut reading = Reading::default();
            reading.fresh([Some(0.0), Some(4096.0), Some(3.0)]);
            reading.temperatures(gates.get_mut(&0).unwrap());
            (picked, reading)
        };
        // Read at t = 30 s: a new measurement.
        let (picked, reading) = poll(TEMPERATURE_PERIOD.as_secs(), 36.0);
        assert_eq!(picked, Some(0));
        assert_eq!(reading.values[3], Some(36.0));
        assert_eq!(reading.quality, vec![Quality::Fresh; 4]);
        // One second later the disk still works, but the temperature is the
        // one already published: held, while the I/O of this poll is fresh.
        for second in [31, 35, 39] {
            let (picked, reading) = poll(second, 99.0);
            assert_eq!(picked, None);
            assert_eq!(reading.values[3], Some(36.0));
            assert_eq!(
                reading.quality,
                vec![
                    Quality::Fresh,
                    Quality::Fresh,
                    Quality::Fresh,
                    Quality::Held
                ]
            );
        }
        // The next read, a period after the first: fresh again.
        for second in [43, 47, 51, 55, 59] {
            assert_eq!(poll(second, 99.0).0, None);
        }
        let (picked, reading) = poll(60, 37.0);
        assert_eq!(picked, Some(0));
        assert_eq!(reading.values[3], Some(37.0));
        assert_eq!(reading.quality, vec![Quality::Fresh; 4]);
    }

    #[test]
    fn disk_states_bump_the_generation_only_on_change() {
        let table = DiskStateTable::default();
        assert_eq!(table.get(), (0, Vec::new()));
        let idle = vec![
            ("storage/a".to_owned(), DiskPower::Active),
            ("storage/b".to_owned(), DiskPower::Idle),
        ];
        table.publish(idle.clone());
        assert_eq!(table.get(), (1, idle.clone()));
        // Published on every poll: the same states are not a change.
        table.publish(idle.clone());
        assert_eq!(table.get().0, 1);
        let standby = vec![
            ("storage/a".to_owned(), DiskPower::Active),
            ("storage/b".to_owned(), DiskPower::Standby),
        ];
        table.publish(standby.clone());
        assert_eq!(table.clone().get(), (2, standby));
    }

    fn entry(index: u32) -> DriveEntry {
        DriveEntry::new(
            index,
            format!("storage/device-{index}"),
            Some("Model".to_owned()),
            Some(format!("SN{index}")),
        )
    }

    // ---- the service's feed ----

    use crate::svc::feed::SourceRequest;
    use oma_ipc::{IdentityHint, WireDevice, WireDrive, WireSchema, WireSensor, WireSnapshot};

    const INTERVAL: Duration = Duration::from_secs(1);
    const ID: &str = "storage/device-0";

    /// The core's table with the one disk of these tests.
    fn table() -> DriveIds {
        DriveIds {
            generation: 1,
            drives: vec![entry(0)],
        }
    }

    /// What the service sends about physical drive 0.
    #[derive(Clone)]
    struct Wire {
        state: &'static str,
        blocks_smart: bool,
        /// The main temperature in the snapshot, and its `held` flag.
        celsius: Option<f64>,
        held: bool,
        seq: u64,
        generation: u64,
        serial: &'static str,
    }

    /// A drive in `state` that blocks nothing, measured in snapshot 1.
    fn wire(state: &'static str, celsius: Option<f64>) -> Wire {
        Wire {
            state,
            blocks_smart: false,
            celsius,
            held: false,
            seq: 1,
            generation: 1,
            serial: "SN0",
        }
    }

    impl Wire {
        /// The feed as the provider sees it, the snapshot received at
        /// `received`: a SMART percentage, then the main temperature.
        fn view(&self, received: Instant) -> FeedView {
            let sensor = |kind: &str, name: &str, unit: &str| WireSensor {
                device_id: "svc-0".to_owned(),
                kind: kind.to_owned(),
                name: name.to_owned(),
                unit: unit.to_owned(),
                label_key: "lhm.raw".to_owned(),
                label_arg: None,
                category: kind.to_owned(),
            };
            let mut schema = WireSchema {
                service: Default::default(),
                devices: vec![WireDevice {
                    id: "svc-0".to_owned(),
                    kind: "storage".to_owned(),
                    name: "Disk".to_owned(),
                    vendor: None,
                    properties: Default::default(),
                    hint: Some(IdentityHint::Storage {
                        physical_drive: 0,
                        model: Some("Model".to_owned()),
                        serial: Some(self.serial.to_owned()),
                    }),
                }],
                sensors: vec![
                    sensor("percent", "life", "percent"),
                    sensor("temperature", "drive", "celsius"),
                ],
            };
            schema.service.drives = vec![WireDrive {
                physical_drive: 0,
                key: oma_ipc::drive_key("Model", self.serial),
                model: Some("Model".to_owned()),
                state: self.state.to_owned(),
                blocks_smart: self.blocks_smart,
            }];
            FeedView {
                generation: self.generation,
                schema: Some(Arc::new(schema)),
                snapshot: Some((
                    received,
                    WireSnapshot {
                        seq: self.seq,
                        timestamp_ms: 0,
                        values: vec![Some(97.0), self.celsius],
                        held: vec![true, self.held],
                    },
                )),
                interval: INTERVAL,
                request: Arc::default(),
            }
        }
    }

    /// The feed once the link is lost.
    fn disconnected() -> FeedView {
        FeedView {
            generation: 99,
            schema: None,
            snapshot: None,
            interval: INTERVAL,
            request: Arc::default(),
        }
    }

    fn never(index: u32) -> Option<TemperatureReport> {
        panic!("disk {index} queried")
    }

    /// A disk seen for the first time, not queried at discovery.
    fn new_disk(start: Instant) -> DiskGate {
        DiskGate::discover(
            None,
            || RotationalOrUnknown,
            start,
            || panic!("queried at discovery"),
        )
    }

    /// One poll of disk 0 as the provider runs it: the gate decides from this
    /// poll's rates (`None` on the warm-up poll after a discovery) and from
    /// what `view` says about the disk, then takes the service's temperature.
    /// Returns whether that asks for a rediscovery.
    fn poll_disk(
        gates: &mut HashMap<u32, DiskGate>,
        rates: Option<(Option<f64>, Option<f64>)>,
        drives: &DriveIds,
        view: &FeedView,
        now: &Stamp,
    ) -> bool {
        let entry = &drives.drives[0];
        let service = service_disk(entry, drives, view, now.mono);
        let gate = gates.get_mut(&0).unwrap();
        match rates {
            Some((read, write)) => gate.observe(read, write, Some(true), &service, now),
            None => gate.warm_up(Some(true), &service, now),
        }
        gate.adopt(&service, entry.key.as_deref(), snapshot_id(view))
    }

    /// The rediscovery a poll asked for: no class asked, no disk queried.
    fn rediscover(gates: HashMap<u32, DiskGate>, now: Instant) -> HashMap<u32, DiskGate> {
        let ids = HashMap::from([(0, ID.to_owned())]);
        let gate = DiskGate::discover(
            gates_by_id(gates, &ids).remove(ID),
            || panic!("class asked again"),
            now,
            || panic!("queried at discovery"),
        );
        HashMap::from([(0, gate)])
    }

    fn published(gates: &mut HashMap<u32, DiskGate>) -> Vec<(Option<f64>, Quality)> {
        gates.get_mut(&0).unwrap().published()
    }

    fn state(gates: &HashMap<u32, DiskGate>) -> (Plan, DiskPower) {
        (gates[&0].plan, gates[&0].power)
    }

    fn measured(celsius: f64, held: bool) -> Plan {
        Plan::Service(ServiceTemperature {
            value: celsius,
            held,
        })
    }

    #[test]
    fn a_stale_feed_has_no_authority() {
        let now = Instant::now() + Duration::from_secs(10);
        let drives = table();
        let disk = &drives.drives[0];
        let current = wire("active", Some(41.0)).view(now - INTERVAL * 3);
        assert_eq!(
            service_disk(disk, &drives, &current, now),
            ServiceDisk::Present {
                state: DriveState::Active,
                blocks_smart: false,
                temperature: Some(ServiceTemperature {
                    value: 41.0,
                    held: false
                }),
            }
        );
        let stale = wire("active", Some(41.0)).view(now - INTERVAL * 3 - Duration::from_millis(1));
        assert_eq!(
            service_disk(disk, &drives, &stale, now),
            ServiceDisk::Absent
        );
        // A standby goes with its feed.
        let asleep = wire("standby", None).view(now - INTERVAL * 4);
        assert_eq!(
            service_disk(disk, &drives, &asleep, now),
            ServiceDisk::Absent
        );

        // No snapshot for this schema yet, or no schema at all.
        let unpaired = FeedView {
            snapshot: None,
            ..current
        };
        assert_eq!(
            service_disk(disk, &drives, &unpaired, now),
            ServiceDisk::Absent
        );
        assert_eq!(
            service_disk(disk, &drives, &disconnected(), now),
            ServiceDisk::Absent
        );
    }

    #[test]
    fn a_disk_asleep_at_startup_is_never_queried() {
        let start = Instant::now();
        let drives = table();
        let mut gates = HashMap::from([(0, new_disk(start))]);
        // The service never measured it; the I/O Windows reports changes nothing.
        for second in [1, 2, 31, 62] {
            let now = stamp(start, second);
            let view = Wire {
                seq: second,
                ..wire("standby", None)
            }
            .view(now.mono);
            assert_eq!(
                service_disk(&drives.drives[0], &drives, &view, now.mono),
                ServiceDisk::Present {
                    state: DriveState::Standby,
                    blocks_smart: false,
                    temperature: None,
                }
            );
            assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
            assert_eq!(state(&gates), (Plan::Wait, DiskPower::Standby));
            assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
            assert!(published(&mut gates).is_empty(), "no sensor declared");
        }
        assert!(gates[&0].temperatures.positions.is_empty());
        assert_eq!(gates[&0].temperatures.read_at, None);
    }

    #[test]
    fn the_first_service_measure_declares_the_sensor_without_a_local_query() {
        let start = Instant::now();
        let drives = table();
        let mut gates = HashMap::from([(0, new_disk(start))]);
        let first = stamp(start, 1);
        let view = wire("active", Some(41.0)).view(first.mono);
        assert!(
            poll_disk(&mut gates, Some(QUIET), &drives, &view, &first),
            "undeclared sensor: rediscover"
        );
        assert_eq!(state(&gates), (measured(41.0, false), DiskPower::Active));

        let mut gates = rediscover(gates, first.mono);
        assert_eq!(gates[&0].temperatures.positions, vec![0]);
        assert_eq!(gates[&0].temperatures.values, vec![Some(41.0)]);
        // The warm-up poll publishes the measure no poll has published yet.
        let after = stamp(start, 2);
        assert!(!poll_disk(&mut gates, None, &drives, &view, &after));
        assert_eq!(refresh_one(&mut gates, after.mono, never), (None, false));
        assert_eq!(published(&mut gates), vec![(Some(41.0), Quality::Fresh)]);
        // The disk was never asked, and its local read is still to come.
        assert_eq!(gates[&0].temperatures.read_at, None);
        assert!(gates[&0].temperatures.report.is_none());
    }

    #[test]
    fn the_service_temperature_replaces_the_local_read() {
        let start = Instant::now();
        let drives = table();
        // The local read is due and the disk works: the service still wins.
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        let now = stamp(start, 31);
        let view = wire("active", Some(41.0)).view(now.mono);
        assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
        assert_eq!(state(&gates), (measured(41.0, false), DiskPower::Active));
        assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
        assert_eq!(published(&mut gates), vec![(Some(41.0), Quality::Fresh)]);
    }

    #[test]
    fn the_service_gives_only_the_main_temperature() {
        let start = Instant::now();
        let drives = table();
        // A disk with an additional sensor of its own, read locally before.
        let mut disk = gate(RotationalOrUnknown, 35.0, start);
        disk.temperatures.positions = vec![0, 2];
        disk.temperatures.values = vec![Some(35.0), Some(30.0)];
        let mut gates = HashMap::from([(0, disk)]);
        let now = stamp(start, 31);
        let view = wire("active", Some(41.0)).view(now.mono);
        assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
        assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
        // Nobody reads the additional sensor meanwhile: a last reading.
        assert_eq!(
            published(&mut gates),
            vec![
                (Some(41.0), Quality::Fresh),
                (Some(30.0), Quality::Suspended)
            ]
        );
        assert_eq!(gates[&0].temperatures.positions, vec![0, 2]);

        // A disk that declared only an additional sensor gets the main one.
        let mut disk = gate(RotationalOrUnknown, 30.0, start);
        disk.temperatures.positions = vec![2];
        disk.temperatures.report = None;
        let mut gates = HashMap::from([(0, disk)]);
        assert!(poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
        let gates = rediscover(gates, now.mono);
        assert_eq!(gates[&0].temperatures.positions, vec![0, 2]);
        assert_eq!(gates[&0].temperatures.values, vec![Some(41.0), Some(30.0)]);
    }

    #[test]
    fn a_non_rotational_disk_takes_nothing_from_the_service() {
        let start = Instant::now();
        let drives = table();
        let key = drives.drives[0].key.as_deref();
        let now = stamp(start, 1);
        // Read at every discovery, as before the gate: an imported value
        // would not survive one, and asking for it would never end.
        let mut disk = DiskGate::discover(None, || NonRotational, start, || None);
        for (state, powered_on) in [("standby", Some(false)), ("active", Some(true))] {
            let view = wire(state, Some(41.0)).view(now.mono);
            let service = service_disk(&drives.drives[0], &drives, &view, now.mono);
            disk.observe(QUIET.0, QUIET.1, powered_on, &service, &now);
            assert!(!disk.adopt(&service, key, snapshot_id(&view)), "{state}");
            assert!(disk.temperatures.imported.is_none());
            assert!(disk.published().is_empty());
        }
    }

    #[test]
    fn a_held_service_temperature_is_held() {
        let start = Instant::now();
        let drives = table();
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        // A new snapshot that repeats the measure of an earlier round.
        for seq in [1, 2] {
            let now = stamp(start, seq);
            let view = Wire {
                held: true,
                seq,
                ..wire("active", Some(41.0))
            }
            .view(now.mono);
            assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &view, &now));
            assert_eq!(state(&gates), (measured(41.0, true), DiskPower::Active));
            assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
            assert_eq!(published(&mut gates), vec![(Some(41.0), Quality::Held)]);
        }
    }

    #[test]
    fn rereading_the_same_service_snapshot_is_held() {
        let start = Instant::now();
        let drives = table();
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        let mut poll = |second: u64, wire: &Wire, received: u64| {
            let now = stamp(start, second);
            let view = wire.view(stamp(start, received).mono);
            assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &view, &now));
            published(&mut gates)
        };
        let first = wire("active", Some(41.0));
        assert_eq!(poll(1, &first, 1), vec![(Some(41.0), Quality::Fresh)]);
        // The core polls faster than the service publishes.
        assert_eq!(poll(2, &first, 1), vec![(Some(41.0), Quality::Held)]);
        assert_eq!(poll(3, &first, 1), vec![(Some(41.0), Quality::Held)]);
        // The next snapshot is a new measure, even with the same value.
        let second = Wire {
            seq: 2,
            ..first.clone()
        };
        assert_eq!(poll(4, &second, 4), vec![(Some(41.0), Quality::Fresh)]);
        assert_eq!(poll(5, &second, 4), vec![(Some(41.0), Quality::Held)]);
        // A restarted service counts from the same number: another feed
        // generation is another measure.
        let restarted = Wire {
            generation: 2,
            ..second
        };
        assert_eq!(poll(6, &restarted, 6), vec![(Some(41.0), Quality::Fresh)]);
        assert_eq!(poll(7, &restarted, 6), vec![(Some(41.0), Quality::Held)]);
    }

    #[test]
    fn a_new_service_measure_is_adopted_before_the_local_deadline() {
        let start = Instant::now();
        let drives = table();
        // Read locally at `start`: the next local read is 30 s away.
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        for (second, celsius) in [(5, 41.0), (6, 42.0)] {
            let now = stamp(start, second);
            let view = Wire {
                seq: second,
                ..wire("active", Some(celsius))
            }
            .view(now.mono);
            assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
            assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
            assert_eq!(published(&mut gates), vec![(Some(celsius), Quality::Fresh)]);
        }
        // The local budget is untouched: the deadline is still the old one.
        assert_eq!(gates[&0].temperatures.read_at, Some(start));
    }

    #[test]
    fn a_standby_service_value_is_historical_and_suspended() {
        let start = Instant::now();
        let drives = table();
        // Asleep since before the app started, with a value the service kept.
        let mut gates = HashMap::from([(0, new_disk(start))]);
        let asleep = Wire {
            held: true,
            ..wire("standby", Some(33.0))
        };
        let first = stamp(start, 1);
        let view = asleep.view(first.mono);
        assert!(poll_disk(&mut gates, Some(BUSY), &drives, &view, &first));
        assert_eq!(state(&gates), (Plan::Wait, DiskPower::Standby));
        let mut gates = rediscover(gates, first.mono);
        let after = stamp(start, 2);
        assert!(!poll_disk(&mut gates, None, &drives, &view, &after));
        assert_eq!(refresh_one(&mut gates, after.mono, never), (None, false));
        assert_eq!(
            published(&mut gates),
            vec![(Some(33.0), Quality::Suspended)]
        );

        // A disk with a local reading takes the service's, more recent one;
        // without one from the service it keeps its own.
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        let unmeasured = wire("standby", None).view(first.mono);
        assert!(!poll_disk(
            &mut gates,
            Some(BUSY),
            &drives,
            &unmeasured,
            &first
        ));
        assert_eq!(
            published(&mut gates),
            vec![(Some(35.0), Quality::Suspended)]
        );
        assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &after));
        assert_eq!(refresh_one(&mut gates, after.mono, never), (None, false));
        assert_eq!(
            published(&mut gates),
            vec![(Some(33.0), Quality::Suspended)]
        );
    }

    #[test]
    fn an_idle_service_disk_is_not_queried_and_shows_its_last_reading() {
        let start = Instant::now();
        let drives = table();
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        // The local read is due and Windows reports I/O: the service owns
        // the source and sends the disk nothing, so neither does the core.
        let unmeasured = stamp(start, 31);
        let view = wire("idle", None).view(unmeasured.mono);
        assert_eq!(
            service_disk(&drives.drives[0], &drives, &view, unmeasured.mono),
            ServiceDisk::Present {
                state: DriveState::Idle,
                blocks_smart: false,
                temperature: None,
            }
        );
        assert!(!poll_disk(
            &mut gates,
            Some(BUSY),
            &drives,
            &view,
            &unmeasured
        ));
        assert_eq!(state(&gates), (Plan::Wait, DiskPower::Idle));
        assert_eq!(
            refresh_one(&mut gates, unmeasured.mono, never),
            (None, false)
        );
        assert_eq!(
            published(&mut gates),
            vec![(Some(35.0), Quality::Suspended)]
        );

        // The value the service kept is a last reading, not a measure.
        for (second, held) in [(32, true), (33, false)] {
            let now = stamp(start, second);
            let view = Wire {
                held,
                seq: second,
                ..wire("idle", Some(33.0))
            }
            .view(now.mono);
            assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
            assert_eq!(state(&gates), (Plan::Wait, DiskPower::Idle));
            assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
            assert_eq!(
                published(&mut gates),
                vec![(Some(33.0), Quality::Suspended)]
            );
        }
        assert_eq!(gates[&0].temperatures.read_at, Some(start), "still due");
    }

    #[test]
    fn rediscovery_keeps_the_imported_temperature_without_a_local_query() {
        let start = Instant::now();
        let drives = table();
        let mut gates = HashMap::from([(0, new_disk(start))]);
        let first = stamp(start, 1);
        let view = wire("active", Some(41.0)).view(first.mono);
        assert!(poll_disk(&mut gates, Some(QUIET), &drives, &view, &first));
        let mut gates = rediscover(gates, first.mono);
        let warm_up = stamp(start, 2);
        assert!(!poll_disk(&mut gates, None, &drives, &view, &warm_up));
        assert_eq!(published(&mut gates), vec![(Some(41.0), Quality::Fresh)]);

        // Another rediscovery (a disk plugged in): position, value and the
        // memory of the snapshot follow the disk, so it is not a new measure.
        let mut gates = rediscover(gates, warm_up.mono);
        assert_eq!(gates[&0].temperatures.positions, vec![0]);
        assert_eq!(gates[&0].temperatures.values, vec![Some(41.0)]);
        let again = stamp(start, 3);
        assert!(!poll_disk(&mut gates, None, &drives, &view, &again));
        assert_eq!(refresh_one(&mut gates, again.mono, never), (None, false));
        assert_eq!(published(&mut gates), vec![(Some(41.0), Quality::Held)]);
        assert_eq!(gates[&0].temperatures.read_at, None);
    }

    #[test]
    fn a_refused_source_falls_back_to_the_activity_rule() {
        let start = Instant::now();
        let drives = table();
        // Another client keeps this disk's SMART on; this one switched it off.
        let refused = |received: Instant| FeedView {
            request: Arc::new(SourceRequest {
                smart_disabled_drives: vec![ID.to_owned()],
                ..SourceRequest::default()
            }),
            ..wire("active", Some(41.0)).view(received)
        };
        let unmeasured = ServiceDisk::Present {
            state: DriveState::Active,
            blocks_smart: false,
            temperature: None,
        };
        assert_eq!(
            service_disk(&drives.drives[0], &drives, &refused(start), start),
            unmeasured
        );
        // A default-off disk this client did not switch on is refused too.
        let usb = DriveIds {
            generation: 1,
            drives: vec![DriveEntry {
                smart_default_off: true,
                ..entry(0)
            }],
        };
        let sent = wire("active", Some(41.0)).view(start);
        assert_eq!(service_disk(&usb.drives[0], &usb, &sent, start), unmeasured);

        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        let quiet = stamp(start, 31);
        let view = refused(quiet.mono);
        assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &view, &quiet));
        assert_eq!(state(&gates), (Plan::Wait, DiskPower::Idle));
        assert_eq!(refresh_one(&mut gates, quiet.mono, never), (None, false));
        assert_eq!(
            published(&mut gates),
            vec![(Some(35.0), Quality::Suspended)]
        );
        // Only recent activity authorizes the local read.
        let busy = stamp(start, 32);
        let view = refused(busy.mono);
        assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &busy));
        assert_eq!(state(&gates), (Plan::Local, DiskPower::Active));
        let outcome = refresh_one(&mut gates, busy.mono, |_| Some(report(&[(0, 36.0)])));
        assert_eq!(outcome, (Some(0), false));
        assert_eq!(published(&mut gates), vec![(Some(36.0), Quality::Fresh)]);
    }

    #[test]
    fn a_blocking_drive_is_never_queried_locally() {
        let start = Instant::now();
        let drives = table();
        let now = stamp(start, 31);
        // The local read is due and the disk works. Only a confirmed standby
        // is an expected suspension; the other states show no value.
        for (wire_state, power, shown) in [
            ("unknown", DiskPower::Unknown, (None, Quality::Fresh)),
            ("smartOff", DiskPower::Unknown, (None, Quality::Fresh)),
            ("active", DiskPower::Unknown, (None, Quality::Fresh)),
            ("idle", DiskPower::Unknown, (None, Quality::Fresh)),
            (
                "standby",
                DiskPower::Standby,
                (Some(35.0), Quality::Suspended),
            ),
        ] {
            let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
            let view = Wire {
                blocks_smart: true,
                ..wire(wire_state, None)
            }
            .view(now.mono);
            assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
            assert_eq!(state(&gates), (Plan::Wait, power), "{wire_state}");
            assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
            assert_eq!(published(&mut gates), vec![shown], "{wire_state}");
            assert_eq!(gates[&0].temperatures.read_at, Some(start), "still due");
        }

        // Nothing is imported from a drive without media, or from one
        // blocking in an unknown state, whatever the snapshot carries.
        for (wire_state, blocks_smart) in [("noMedia", false), ("unknown", true)] {
            let mut gates = HashMap::from([(0, new_disk(start))]);
            let view = Wire {
                blocks_smart,
                ..wire(wire_state, Some(41.0))
            }
            .view(now.mono);
            assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
            assert_eq!(state(&gates), (Plan::Wait, DiskPower::Unknown));
            assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
            assert!(gates[&0].temperatures.positions.is_empty(), "{wire_state}");
        }
    }

    #[test]
    fn losing_the_service_keeps_the_sensor_and_its_last_value() {
        let start = Instant::now();
        let drives = table();
        for lost in ["disconnected", "stale"] {
            let mut gates = HashMap::from([(0, new_disk(start))]);
            let first = stamp(start, 1);
            let view = wire("active", Some(41.0)).view(first.mono);
            assert!(poll_disk(&mut gates, Some(QUIET), &drives, &view, &first));
            let mut gates = rediscover(gates, first.mono);
            let warm_up = stamp(start, 2);
            assert!(!poll_disk(&mut gates, None, &drives, &view, &warm_up));
            assert_eq!(published(&mut gates), vec![(Some(41.0), Quality::Fresh)]);

            // The next poll, the disk idle: the same snapshot, now too old,
            // or no feed at all.
            let now = stamp(start, 5);
            let view = match lost {
                "stale" => view.clone(),
                _ => disconnected(),
            };
            assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &view, &now));
            assert_eq!(state(&gates), (Plan::Wait, DiskPower::Idle), "{lost}");
            assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
            assert_eq!(gates[&0].temperatures.positions, vec![0], "{lost}");
            assert_eq!(
                published(&mut gates),
                vec![(Some(41.0), Quality::Suspended)],
                "{lost}"
            );
        }
    }

    #[test]
    fn standby_from_the_service_is_not_kept_after_a_disconnect() {
        let start = Instant::now();
        let drives = table();
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        let asleep = stamp(start, 1);
        let view = wire("standby", None).view(asleep.mono);
        assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &view, &asleep));
        assert_eq!(state(&gates), (Plan::Wait, DiskPower::Standby));

        // Without the service nobody confirms the standby: the disk is idle.
        let lost = stamp(start, 2);
        assert!(!poll_disk(
            &mut gates,
            Some(QUIET),
            &drives,
            &disconnected(),
            &lost
        ));
        assert_eq!(state(&gates), (Plan::Wait, DiskPower::Idle));
        assert_eq!(refresh_one(&mut gates, lost.mono, never), (None, false));
        assert_eq!(
            published(&mut gates),
            vec![(Some(35.0), Quality::Suspended)]
        );
        // And the local rule is back: activity authorizes a read.
        let busy = stamp(start, 3);
        assert!(!poll_disk(
            &mut gates,
            Some(BUSY),
            &drives,
            &disconnected(),
            &busy
        ));
        assert_eq!(state(&gates), (Plan::Local, DiskPower::Active));
    }

    #[test]
    fn reconnecting_with_another_key_drops_the_old_measure() {
        let start = Instant::now();
        let drives = table();
        let adopted = |gates: &mut HashMap<u32, DiskGate>| {
            let first = stamp(start, 1);
            let view = wire("active", Some(41.0)).view(first.mono);
            assert!(!poll_disk(gates, Some(QUIET), &drives, &view, &first));
            assert_eq!(published(gates), vec![(Some(41.0), Quality::Fresh)]);
        };

        // The service comes back describing another disk under the same
        // number: it is not associated, so nothing of that disk is taken and
        // the core's disk keeps its own last reading.
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        adopted(&mut gates);
        let now = stamp(start, 5);
        let other = Wire {
            generation: 2,
            serial: "SN-other",
            ..wire("standby", Some(50.0))
        }
        .view(now.mono);
        assert_eq!(
            service_disk(&drives.drives[0], &drives, &other, now.mono),
            ServiceDisk::Absent
        );
        assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &other, &now));
        assert_eq!(state(&gates), (Plan::Wait, DiskPower::Idle));
        assert_eq!(
            published(&mut gates),
            vec![(Some(41.0), Quality::Suspended)]
        );

        // The core's disk itself now has another key under the same device
        // id: the measure taken for the old one is not shown for it.
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        adopted(&mut gates);
        let swapped = DriveIds {
            generation: 2,
            drives: vec![DriveEntry::new(
                0,
                ID.to_owned(),
                Some("Model".to_owned()),
                Some("SN-other".to_owned()),
            )],
        };
        let idle = Wire {
            generation: 2,
            serial: "SN-other",
            ..wire("idle", None)
        }
        .view(now.mono);
        assert!(!poll_disk(&mut gates, Some(QUIET), &swapped, &idle, &now));
        assert_eq!(state(&gates), (Plan::Wait, DiskPower::Idle));
        assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
        assert_eq!(published(&mut gates), vec![(None, Quality::Fresh)]);
        // Also with the link down in between.
        let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
        adopted(&mut gates);
        assert!(!poll_disk(
            &mut gates,
            Some(QUIET),
            &swapped,
            &disconnected(),
            &now
        ));
        assert_eq!(published(&mut gates), vec![(None, Quality::Fresh)]);
        // The first measure under the new key is a new one.
        let later = stamp(start, 6);
        let measured_again = Wire {
            generation: 2,
            serial: "SN-other",
            ..wire("active", Some(29.0))
        }
        .view(later.mono);
        assert!(!poll_disk(
            &mut gates,
            Some(QUIET),
            &swapped,
            &measured_again,
            &later
        ));
        assert_eq!(published(&mut gates), vec![(Some(29.0), Quality::Fresh)]);
    }

    #[test]
    fn a_hint_bound_but_unassociated_disk_uses_the_local_rule() {
        let start = Instant::now();
        let drives = table();
        // The hint binds (so `svc` leaves the main temperature to this
        // provider), but the key is twice in the service's table.
        let duplicated = |received: Instant| {
            let mut view = wire("standby", Some(41.0)).view(received);
            let schema = Arc::make_mut(view.schema.as_mut().unwrap());
            let twin = WireDrive {
                physical_drive: 3,
                ..schema.service.drives[0].clone()
            };
            schema.service.drives.push(twin);
            view
        };
        assert_eq!(
            service_disk(&drives.drives[0], &drives, &duplicated(start), start),
            ServiceDisk::Absent
        );

        // Neither the standby nor the temperature of that entry is taken.
        let mut gates = HashMap::from([(0, new_disk(start))]);
        let quiet = stamp(start, 1);
        let view = duplicated(quiet.mono);
        assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &view, &quiet));
        assert_eq!(state(&gates), (Plan::Wait, DiskPower::Idle));
        assert_eq!(refresh_one(&mut gates, quiet.mono, never), (None, false));
        // The disk still gets a main temperature: from its own first
        // authorized read.
        let busy = stamp(start, 2);
        let view = duplicated(busy.mono);
        assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &busy));
        assert_eq!(state(&gates), (Plan::Local, DiskPower::Active));
        let outcome = refresh_one(&mut gates, busy.mono, |_| Some(report(&[(0, 39.0)])));
        assert_eq!(outcome, (Some(0), true), "undeclared sensor: rediscover");
        let mut gates = rediscover(gates, busy.mono);
        let after = stamp(start, 3);
        let view = duplicated(after.mono);
        assert!(!poll_disk(&mut gates, None, &drives, &view, &after));
        assert_eq!(published(&mut gates), vec![(Some(39.0), Quality::Fresh)]);
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

    #[test]
    fn disk_names_list_volumes() {
        assert_eq!(
            disk_name(&parse_disk_instance("0 C: D:").unwrap()),
            "Disk 0 (C:, D:)"
        );
        assert_eq!(disk_name(&parse_disk_instance("1").unwrap()), "Disk 1");
    }
}
