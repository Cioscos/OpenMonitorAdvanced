//! The service's feed as seen by the storage provider.

use super::disk_gate::tests::*;
use super::*;
use std::time::Duration;
use DiskClass::{NonRotational, RotationalOrUnknown};

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
            drives_generation: 0,
            carried: None,
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
        drives_generation: 0,
        carried: None,
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

/// One poll of disk 0, the step the provider runs for every disk, with
/// Windows reporting the disk on. `rates` is `None` on the warm-up poll
/// after a discovery. Returns whether it asks for a rediscovery.
fn poll_disk(
    gates: &mut HashMap<u32, DiskGate>,
    rates: Option<(Option<f64>, Option<f64>)>,
    drives: &DriveIds,
    view: &FeedView,
    now: &Stamp,
) -> bool {
    let entry = drives.drives.first();
    let gate = gates.get_mut(&0).unwrap();
    gate.step(rates, Some(true), entry, drives, view, now)
}

/// A disk with the main temperature (35) and an additional sensor (30),
/// both read locally at `read_at`.
fn two_sensors(read_at: Instant) -> DiskGate {
    let mut disk = gate(RotationalOrUnknown, 35.0, read_at);
    disk.temperatures.positions = vec![0, 2];
    disk.temperatures.values = vec![Some(35.0), Some(30.0)];
    disk.temperatures.report = Some(report(&[(0, 35.0), (2, 30.0)]));
    disk
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
    let mut gates = HashMap::from([(0, two_sensors(start))]);
    let now = stamp(start, 31);
    let view = wire("active", Some(41.0)).view(now.mono);
    assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &view, &now));
    assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
    // The additional sensor stays the core's own.
    assert_eq!(gates[&0].temperatures.positions, vec![0, 2]);
    assert_eq!(gates[&0].temperatures.values, vec![Some(41.0), Some(30.0)]);

    // A disk that declared only an additional sensor gets the main one.
    let mut disk = gate(RotationalOrUnknown, 30.0, start);
    disk.temperatures.positions = vec![2];
    disk.temperatures.report = None;
    let mut gates = HashMap::from([(0, disk)]);
    assert!(poll_disk(&mut gates, Some(QUIET), &drives, &view, &now));
    let gates = rediscover(gates, now.mono);
    assert_eq!(gates[&0].temperatures.positions, vec![0, 2]);
    assert_eq!(gates[&0].temperatures.values, vec![Some(41.0), Some(30.0)]);
}

#[test]
fn additional_sensors_of_a_service_disk_are_read_locally_only_with_recent_activity() {
    let start = Instant::now();
    let drives = table();
    let active = |second: u64| {
        Wire {
            seq: second,
            ..wire("active", Some(41.0))
        }
        .view(stamp(start, second).mono)
    };
    let mut gates = HashMap::from([(0, two_sensors(start))]);
    // Due, but the disk does not work: not asked.
    let quiet = stamp(start, 31);
    assert!(!poll_disk(
        &mut gates,
        Some(QUIET),
        &drives,
        &active(31),
        &quiet
    ));
    assert_eq!(state(&gates), (measured(41.0, false), DiskPower::Active));
    assert_eq!(refresh_one(&mut gates, quiet.mono, never), (None, false));
    assert_eq!(gates[&0].temperatures.read_at, Some(start), "still due");

    // It works: the overdue read starts, for the additional sensor.
    let busy = stamp(start, 32);
    assert!(!poll_disk(
        &mut gates,
        Some(BUSY),
        &drives,
        &active(32),
        &busy
    ));
    let outcome = refresh_one(&mut gates, busy.mono, |index| {
        assert_eq!(index, 0);
        Some(report(&[(0, 99.0), (2, 31.0)]))
    });
    assert_eq!(outcome, (Some(0), false));
    assert_eq!(gates[&0].temperatures.values[1], Some(31.0));
    assert_eq!(gates[&0].temperatures.read_at, Some(busy.mono));
    // Not due again before a period, however busy.
    let next = stamp(start, 33);
    assert!(!poll_disk(
        &mut gates,
        Some(BUSY),
        &drives,
        &active(33),
        &next
    ));
    assert_eq!(refresh_one(&mut gates, next.mono, never), (None, false));

    // In every other state of the service nothing is read, due and busy.
    for (wire_state, blocks_smart) in [
        ("standby", false),
        ("idle", false),
        ("noMedia", false),
        ("active", true),
        ("unknown", true),
    ] {
        let mut gates = HashMap::from([(0, two_sensors(start))]);
        let view = Wire {
            blocks_smart,
            ..wire(wire_state, Some(41.0))
        }
        .view(busy.mono);
        assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &busy));
        assert_eq!(state(&gates).0, Plan::Wait, "{wire_state}");
        assert_eq!(refresh_one(&mut gates, busy.mono, never), (None, false));
    }
    // Nor when Windows switched the disk off.
    let mut off = two_sensors(start);
    let entry = drives.drives.first();
    assert!(!off.step(Some(BUSY), Some(false), entry, &drives, &active(32), &busy));
    let mut gates = HashMap::from([(0, off)]);
    assert_eq!(state(&gates), (Plan::Wait, DiskPower::Standby));
    assert_eq!(refresh_one(&mut gates, busy.mono, never), (None, false));
}

#[test]
fn additional_sensors_are_held_between_reads_and_suspended_when_idle() {
    let start = Instant::now();
    let drives = table();
    let mut gates = HashMap::from([(0, two_sensors(start))]);
    let mut poll = |second: u64, rates, read: Option<f64>| {
        let now = stamp(start, second);
        let view = Wire {
            seq: second,
            ..wire("active", Some(41.0))
        }
        .view(now.mono);
        assert!(!poll_disk(&mut gates, Some(rates), &drives, &view, &now));
        let outcome = refresh_one(&mut gates, now.mono, |_| {
            let celsius = read.expect("disk queried");
            Some(report(&[(0, 99.0), (2, celsius)]))
        });
        assert_eq!(outcome, (read.map(|_| 0), false), "second {second}");
        published(&mut gates)
    };
    use Quality::{Fresh, Held, Suspended};
    // The disk works and its read is not due: the value of the last one.
    assert_eq!(
        poll(25, BUSY, None),
        vec![(Some(41.0), Fresh), (Some(30.0), Held)]
    );
    // The read, then the same value again until the next one.
    assert_eq!(
        poll(30, BUSY, Some(31.0)),
        vec![(Some(41.0), Fresh), (Some(31.0), Fresh)]
    );
    for second in [31, 40, 49] {
        assert_eq!(
            poll(second, BUSY, None),
            vec![(Some(41.0), Fresh), (Some(31.0), Held)],
            "second {second}"
        );
    }
    // No activity for longer than the window: a last reading, also once
    // the read is due.
    for second in [60, 61, 70, 80, 90] {
        assert_eq!(
            poll(second, QUIET, None),
            vec![(Some(41.0), Fresh), (Some(31.0), Suspended)],
            "second {second}"
        );
    }
    // Work again: the overdue read, fresh.
    assert_eq!(
        poll(91, BUSY, Some(33.0)),
        vec![(Some(41.0), Fresh), (Some(33.0), Fresh)]
    );
}

#[test]
fn a_service_disk_without_additional_sensors_is_never_queried() {
    let start = Instant::now();
    let drives = table();
    // Busy all along, and its local read due from the first poll on.
    let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
    for second in [31, 32, 40, 61, 62, 95] {
        let now = stamp(start, second);
        let view = Wire {
            seq: second,
            ..wire("active", Some(41.0))
        }
        .view(now.mono);
        assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
        assert_eq!(gates[&0].local_read(), LocalRead::Nothing);
        assert_eq!(refresh_one(&mut gates, now.mono, never), (None, false));
        assert_eq!(published(&mut gates), vec![(Some(41.0), Quality::Fresh)]);
    }
    assert_eq!(gates[&0].temperatures.read_at, Some(start));
}

#[test]
fn the_main_temperature_stays_the_services_after_a_local_read_of_the_additional_ones() {
    let start = Instant::now();
    let drives = table();
    let mut gates = HashMap::from([(0, two_sensors(start))]);
    let now = stamp(start, 31);
    let view = wire("active", Some(41.0)).view(now.mono);
    assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
    assert_eq!(gates[&0].local_read(), LocalRead::Additional);
    // The disk answers with a main temperature of its own: not taken.
    let outcome = refresh_one(&mut gates, now.mono, |_| {
        Some(report(&[(0, 99.0), (2, 31.0)]))
    });
    assert_eq!(outcome, (Some(0), false));
    assert_eq!(gates[&0].temperatures.values, vec![Some(41.0), Some(31.0)]);
    assert_eq!(
        published(&mut gates),
        vec![(Some(41.0), Quality::Fresh), (Some(31.0), Quality::Fresh)]
    );
    // The same snapshot read again: the main one is held as before, and
    // the local read did not make it fresh.
    let again = stamp(start, 32);
    assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &again));
    assert_eq!(refresh_one(&mut gates, again.mono, never), (None, false));
    assert_eq!(
        published(&mut gates),
        vec![(Some(41.0), Quality::Held), (Some(31.0), Quality::Held)]
    );
    // A failed read loses the additional value only.
    let mut gates = HashMap::from([(0, two_sensors(start))]);
    assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
    assert_eq!(
        refresh_one(&mut gates, now.mono, |_| None),
        (Some(0), false)
    );
    assert_eq!(
        published(&mut gates),
        vec![(Some(41.0), Quality::Fresh), (None, Quality::Fresh)]
    );
    // The service gone and the disk idle: the service's value is the
    // last reading of the main sensor, not the disk's 99.
    let mut gates = HashMap::from([(0, two_sensors(start))]);
    assert!(!poll_disk(&mut gates, Some(BUSY), &drives, &view, &now));
    let read = |_| Some(report(&[(0, 99.0), (2, 31.0)]));
    assert_eq!(refresh_one(&mut gates, now.mono, read), (Some(0), false));
    let lost = stamp(start, 50);
    assert!(!poll_disk(
        &mut gates,
        Some(QUIET),
        &drives,
        &disconnected(),
        &lost
    ));
    assert_eq!(
        published(&mut gates),
        vec![
            (Some(41.0), Quality::Suspended),
            (Some(31.0), Quality::Suspended)
        ]
    );
}

#[test]
fn a_changed_request_does_not_make_the_same_measure_fresh_again() {
    let start = Instant::now();
    let drives = table();
    let mut gates = HashMap::from([(0, gate(RotationalOrUnknown, 35.0, start))]);
    let first = stamp(start, 1);
    let view = wire("active", Some(41.0)).view(first.mono);
    assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &view, &first));
    assert_eq!(published(&mut gates), vec![(Some(41.0), Quality::Fresh)]);
    // The user switches another disk off: the feed's generation moves,
    // the snapshot is the same one.
    let changed = FeedView {
        generation: view.generation + 1,
        request: Arc::new(SourceRequest {
            smart_disabled_drives: vec!["storage/another".to_owned()],
            ..SourceRequest::default()
        }),
        ..view.clone()
    };
    let next = stamp(start, 2);
    assert!(!poll_disk(
        &mut gates,
        Some(QUIET),
        &drives,
        &changed,
        &next
    ));
    assert_eq!(published(&mut gates), vec![(Some(41.0), Quality::Held)]);
}

#[test]
fn a_change_of_the_services_drive_table_asks_for_no_rediscovery() {
    let start = Instant::now();
    let drives = table();
    let mut gates = HashMap::from([(0, two_sensors(start))]);
    let first = stamp(start, 1);
    let view = wire("active", Some(41.0)).view(first.mono);
    assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &view, &first));
    assert_eq!(state(&gates), (measured(41.0, false), DiskPower::Active));
    published(&mut gates);

    // The service's table changes (this disk went idle): until the next
    // snapshot the feed carries the old values without authority.
    let changed = FeedView {
        drives_generation: view.drives_generation + 1,
        snapshot: None,
        carried: view.snapshot.clone(),
        ..wire("idle", Some(41.0)).view(first.mono)
    };
    let between = stamp(start, 2);
    assert_eq!(
        service_disk(&drives.drives[0], &drives, &changed, between.mono),
        ServiceDisk::Absent,
        "a carried snapshot gives no state and no temperature"
    );
    assert!(
        !poll_disk(&mut gates, Some(QUIET), &drives, &changed, &between),
        "no rediscovery"
    );
    // Nothing is queried, the sensors stay declared and keep their
    // values as last readings.
    assert_eq!(state(&gates), (Plan::Wait, DiskPower::Idle));
    assert_eq!(refresh_one(&mut gates, between.mono, never), (None, false));
    assert_eq!(gates[&0].temperatures.positions, vec![0, 2]);
    assert_eq!(
        published(&mut gates),
        vec![
            (Some(41.0), Quality::Suspended),
            (Some(30.0), Quality::Suspended)
        ]
    );

    // The next snapshot gives the new table its authority.
    let after = stamp(start, 3);
    let idle = Wire {
        seq: 2,
        held: true,
        ..wire("idle", Some(41.0))
    }
    .view(after.mono);
    assert!(!poll_disk(&mut gates, Some(QUIET), &drives, &idle, &after));
    assert_eq!(state(&gates), (Plan::Wait, DiskPower::Idle));
    assert_eq!(refresh_one(&mut gates, after.mono, never), (None, false));
    assert_eq!(gates[&0].temperatures.positions, vec![0, 2]);
}

#[test]
fn only_a_disk_that_may_be_rotational_has_its_main_temperature_owned() {
    // A non-rotational disk imports nothing from the service, so the
    // service's sensor must stay available to fill the gap.
    assert!(owns_main_temperature(RotationalOrUnknown));
    assert!(!owns_main_temperature(NonRotational));
    // An entry built without a class is the cautious one.
    assert!(entry(0).owns_main_temperature);
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

    // A disk with a local reading shows the one the service kept, when
    // there is one; without it, its own.
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
    let hint = (Some("Model".to_owned()), Some("SN0".to_owned()));
    assert_eq!(
        crate::svc::drives::storage_binding(&hint.0, &hint.1, 0, &drives),
        Some(&drives.drives[0]),
        "the premise: the hint of that view binds"
    );
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
