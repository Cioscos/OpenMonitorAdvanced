//! What the gate knows about one disk, and the poll that drives it.

use super::temperatures::{SnapshotId, MAIN_POSITION};
use super::*;

/// What the gate knows about one disk. It follows the disk across
/// discoveries for as long as its device id stays the same.
pub(super) struct DiskGate {
    pub(super) class: DiskClass,
    pub(super) activity: Activity,
    pub(super) temperatures: DiskTemperatures,
    /// The decision of the last poll.
    pub(super) plan: Plan,
    pub(super) power: DiskPower,
    /// The disk worked recently, as of the last poll.
    pub(super) recent: bool,
}

impl DiskGate {
    /// The state after a discovery. `previous` is the same disk (same device
    /// id) before it; `class` is asked only for a disk seen for the first
    /// time. Only a non-rotational disk is `read`, as before the gate: any
    /// other keeps what it had and gets its sensors from its first
    /// authorized read, through a rediscovery.
    pub(super) fn discover(
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
            recent: false,
        }
    }

    /// One poll of this disk: decides where its temperature comes from, out
    /// of this poll's PDH rates (`None` on the warm-up poll after a
    /// discovery, whose rates are no sample) and of what the service's feed
    /// says about the disk `entry`, then takes the service's temperature.
    /// Returns whether the main sensor is still to be declared, by a
    /// rediscovery.
    pub(super) fn step(
        &mut self,
        rates: Option<(Option<f64>, Option<f64>)>,
        powered_on: Option<bool>,
        entry: Option<&DriveEntry>,
        drives: &DriveIds,
        view: &FeedView,
        now: &Stamp,
    ) -> bool {
        let service = entry.map_or(ServiceDisk::Absent, |entry| {
            service_disk(entry, drives, view, now.mono)
        });
        match rates {
            Some((read, write)) => self.observe(read, write, powered_on, &service, now),
            None => self.warm_up(powered_on, &service, now),
        }
        let key = entry.and_then(|entry| entry.key.as_deref());
        self.adopt(&service, key, snapshot_id(view))
    }

    /// What a local query of this disk may refresh, as of the last poll.
    pub(super) fn local_read(&self) -> LocalRead {
        let additional = self
            .temperatures
            .positions
            .iter()
            .any(|&position| position != MAIN_POSITION);
        local_read(self.plan, self.recent, additional)
    }

    /// Takes this poll's PDH rates (`None` when missing) and decides where
    /// the temperature comes from.
    pub(super) fn observe(
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
    pub(super) fn warm_up(&mut self, powered_on: Option<bool>, service: &ServiceDisk, now: &Stamp) {
        self.decide(powered_on, service, now);
    }

    pub(super) fn decide(&mut self, powered_on: Option<bool>, service: &ServiceDisk, now: &Stamp) {
        let recent = self.activity.recent(now);
        self.plan = plan(self.class, powered_on, service, recent);
        self.power = power(self.class, powered_on, service, recent);
        self.recent = recent;
    }

    /// After this poll's decision: takes the main temperature the service
    /// has for this disk, whose wire key is `key`, from the snapshot
    /// `snapshot`. A measure of an active disk is taken as it is; the value
    /// the service keeps for a disk in standby or idle is a last reading. A
    /// drive without media or blocking the gate in an unknown state gives
    /// nothing. Returns whether the main sensor is still to be declared, by a
    /// rediscovery; the disk is never queried for it.
    pub(super) fn adopt(
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
    /// core's alone and follow the local rule: read while the disk works
    /// (`Fresh`, then `Held` until the next read), last readings otherwise.
    pub(super) fn published(&mut self) -> Vec<(Option<f64>, Quality)> {
        let read_locally = self.local_read() != LocalRead::Nothing;
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
        let local = |value: Option<f64>| match value {
            Some(_) if !measured => (value, Quality::Held),
            _ => (value, Quality::Fresh),
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
                    if position != MAIN_POSITION && read_locally {
                        local(value)
                    } else if position != MAIN_POSITION {
                        last_reading(value)
                    } else if temperature.held || !adopted {
                        (Some(temperature.value), Quality::Held)
                    } else {
                        (Some(temperature.value), Quality::Fresh)
                    }
                })
                .collect(),
            (Plan::Local, _) => values.map(local).collect(),
        }
    }
}

/// What `DiskStateTable` publishes: the state of every identified disk, in
/// disk order.
pub(super) fn disk_states(
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
pub(super) fn gates_by_id(
    gates: HashMap<u32, DiskGate>,
    ids: &HashMap<u32, String>,
) -> HashMap<String, DiskGate> {
    gates
        .into_iter()
        .filter_map(|(index, gate)| Some((ids.get(&index)?.clone(), gate)))
        .collect()
}

/// Reads the temperatures of at most one disk: among those authorized for a
/// local read ([`LocalRead`]), the one never read (lowest index first), else
/// the oldest due read. A disk that waits is skipped without renewing its
/// deadline, so it stays due and does not hold back the others. A disk whose
/// main temperature is the service's keeps it. Returns the disk read and
/// whether its answer asks for a rediscovery.
pub(super) fn refresh_one(
    gates: &mut HashMap<u32, DiskGate>,
    now: Instant,
    read: impl FnOnce(u32) -> Option<TemperatureReport>,
) -> (Option<u32>, bool) {
    let authorized = || {
        gates
            .iter()
            .filter(|(_, gate)| gate.local_read() != LocalRead::Nothing)
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
    let scope = gate.local_read();
    let report = picked.and_then(read);
    let rediscover = match scope {
        LocalRead::Additional => gate.temperatures.refresh_additional(report.as_ref(), now),
        LocalRead::All | LocalRead::Nothing => gate.temperatures.refresh(report.as_ref(), now),
    };
    (picked, rediscover)
}

/// The values of a poll with the quality of each.
#[derive(Default)]
pub(super) struct Reading {
    pub(super) values: Vec<Option<f64>>,
    pub(super) quality: Vec<Quality>,
}

impl Reading {
    /// Values measured by this poll (or absent).
    pub(super) fn fresh(&mut self, values: impl IntoIterator<Item = Option<f64>>) {
        for value in values {
            self.values.push(value);
            self.quality.push(Quality::Fresh);
        }
    }

    pub(super) fn temperatures(&mut self, gate: &mut DiskGate) {
        for (value, quality) in gate.published() {
            self.values.push(value);
            self.quality.push(quality);
        }
    }
}

/// Temperatures of disk `index`. A disk known to be spun down is not queried,
/// because the query could wake it up: `None`, as for an unsupported disk.
pub(super) fn read_temperatures(index: u32) -> Option<TemperatureReport> {
    PhysicalDrive::open(index)
        .filter(|drive| may_query(drive.powered_on()))
        .and_then(|drive| query_temperatures(&drive))
}

/// Whether disk `index` incurs a seek penalty; `None` when it does not say.
pub(super) fn seek_penalty(index: u32) -> Option<bool> {
    PhysicalDrive::open(index)?.seek_penalty()
}

/// `Some(false)` when Windows switched disk `index` off. Asking does not
/// reach the disk.
pub(super) fn powered_on(index: u32) -> Option<bool> {
    PhysicalDrive::open(index)?.powered_on()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    use crate::storage_temperature::TEMPERATURE_PERIOD;
    use std::time::{Duration, SystemTime};
    use DiskClass::{NonRotational, RotationalOrUnknown};

    pub(in crate::storage) const BUSY: (Option<f64>, Option<f64>) = (Some(4096.0), Some(0.0));
    pub(in crate::storage) const QUIET: (Option<f64>, Option<f64>) = (Some(0.0), Some(0.0));

    /// `seconds` after `start` on both clocks.
    pub(in crate::storage) fn stamp(start: Instant, seconds: u64) -> Stamp {
        Stamp {
            mono: start + Duration::from_secs(seconds),
            wall: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + seconds),
        }
    }

    pub(in crate::storage) fn report(sensors: &[(usize, f64)]) -> TemperatureReport {
        TemperatureReport {
            sensors: sensors.iter().map(|&(i, c)| (i, Some(c))).collect(),
            warning_c: Some(60),
            critical_c: None,
        }
    }

    /// A disk whose only temperature sensor read `celsius` at `read_at`,
    /// published since.
    pub(in crate::storage) fn gate(class: DiskClass, celsius: f64, read_at: Instant) -> DiskGate {
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
            recent: false,
        }
    }

    /// One poll of `gate` without the service: `rates` are its PDH rates.
    pub(in crate::storage) fn observe(
        gate: &mut DiskGate,
        rates: (Option<f64>, Option<f64>),
        now: &Stamp,
    ) {
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

    pub(in crate::storage) fn entry(index: u32) -> DriveEntry {
        DriveEntry::new(
            index,
            format!("storage/device-{index}"),
            Some("Model".to_owned()),
            Some(format!("SN{index}")),
        )
    }
}
