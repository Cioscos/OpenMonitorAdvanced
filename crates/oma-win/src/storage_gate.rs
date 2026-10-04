//! When the core may read a disk's temperature (spec M6b §5.1, §5.2). The
//! temperature query reaches a SATA hard disk: it wakes a disk in standby and
//! resets Windows' idle timer on an awake one. A rotational disk, or one of
//! unknown class, is therefore read only after recent real I/O. Pure
//! decisions: no disk is touched here.

use oma_ipc::DriveState;
use std::time::Duration;

use windows::Win32::Storage::FileSystem::{
    BusTypeFileBackedVirtual, BusTypeNvme, BusTypeSpaces, BusTypeVirtual,
};

use crate::storage_health::Stamp;

/// Whether reading a disk's temperature can spin it up or keep it spinning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskClass {
    NonRotational,
    RotationalOrUnknown,
}

/// NonRotational: bus 14, 15, 16 (virtual, file-backed virtual, Storage Spaces), 17 (NVMe),
/// or `seek_penalty == Some(false)`.
pub fn disk_class(bus_type: Option<i32>, seek_penalty: Option<bool>) -> DiskClass {
    let solid_bus = [
        BusTypeVirtual,
        BusTypeFileBackedVirtual,
        BusTypeSpaces,
        BusTypeNvme,
    ]
    .iter()
    .any(|bus| bus_type == Some(bus.0));
    if solid_bus || seek_penalty == Some(false) {
        DiskClass::NonRotational
    } else {
        DiskClass::RotationalOrUnknown
    }
}

/// How long a positive I/O rate authorizes a temperature read.
pub const ACTIVITY_WINDOW: Duration = Duration::from_secs(10);

/// The recent I/O of one disk, from the PDH rates of each poll.
#[derive(Debug, Clone, Copy, Default)]
pub struct Activity {
    /// The last poll with a positive rate, while it still counts.
    active: Option<Stamp>,
    /// The last poll, whatever it carried.
    polled: Option<Stamp>,
}

impl Activity {
    /// `read`/`write`: the PDH rates of this poll; `None` when missing. The
    /// warm-up poll after a discovery is no sample and is not observed at all.
    ///
    /// A sample is valid when both rates are finite and not negative. A valid
    /// positive one opens the window; a valid zero leaves an open window
    /// alone; anything else closes it. After a gap longer than the window on
    /// either clock (a stalled sampler, a suspend, a clock change: see
    /// [`Stamp::elapsed_until`]) the earlier activity is forgotten and this
    /// sample, whose rate spans the gap, is only a warm-up.
    pub fn observe(&mut self, read: Option<f64>, write: Option<f64>, at: &Stamp) {
        let gap = self
            .polled
            .is_some_and(|polled| polled.elapsed_until(at) > ACTIVITY_WINDOW);
        self.polled = Some(*at);
        let valid = |rate: Option<f64>| rate.filter(|r| r.is_finite() && *r >= 0.0);
        match (valid(read), valid(write)) {
            (Some(read), Some(write)) if !gap => {
                if read > 0.0 || write > 0.0 {
                    self.active = Some(*at);
                }
            }
            _ => self.active = None,
        }
    }

    /// True when a poll of the last [`ACTIVITY_WINDOW`] saw the disk working.
    pub fn recent(&self, at: &Stamp) -> bool {
        self.active
            .is_some_and(|active| active.elapsed_until(at) <= ACTIVITY_WINDOW)
    }
}

/// What a disk is doing, as far as the core can tell without touching it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiskPower {
    Active,
    Idle,
    Standby,
    Unknown,
}

/// The main temperature of a disk as the service measured it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServiceTemperature {
    pub value: f64,
    pub held: bool,
}

/// What the service says about a disk bound to a core disk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ServiceDisk {
    /// Not connected, or no entry bound to this disk.
    Absent,
    Present {
        state: DriveState,
        blocks_smart: bool,
        temperature: Option<ServiceTemperature>,
    },
}

/// Where this poll's temperature of a disk comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Plan {
    /// The core may query the disk.
    Local,
    /// The service's measurement is used; the disk is not queried.
    Service(ServiceTemperature),
    /// Nobody reads: the disk is not queried.
    Wait,
}

/// The decision table of spec §5.2, in evaluation order. `powered_on` is
/// `GetDevicePowerState`; `recent` is [`Activity::recent`].
fn decide(
    class: DiskClass,
    powered_on: Option<bool>,
    service: &ServiceDisk,
    recent: bool,
) -> (Plan, DiskPower) {
    if powered_on == Some(false) {
        return (Plan::Wait, DiskPower::Standby);
    }
    if let ServiceDisk::Present {
        state: DriveState::NoMedia,
        ..
    } = service
    {
        return (Plan::Wait, DiskPower::Unknown);
    }
    if class == DiskClass::NonRotational {
        return (Plan::Local, DiskPower::Active);
    }
    if let ServiceDisk::Present {
        state,
        blocks_smart,
        temperature,
    } = *service
    {
        let standby = state == DriveState::Standby;
        if blocks_smart {
            // The gate veto: this disk keeps SMART closed for every disk.
            let power = if standby {
                DiskPower::Standby
            } else {
                DiskPower::Unknown
            };
            return (Plan::Wait, power);
        }
        if standby {
            return (Plan::Wait, DiskPower::Standby);
        }
        if state == DriveState::Idle {
            // The service owns the source and sends this disk nothing while
            // it sees no activity: the core does not query it either.
            return (Plan::Wait, DiskPower::Idle);
        }
        if let (DriveState::Active, Some(temperature)) = (state, temperature) {
            return (Plan::Service(temperature), DiskPower::Active);
        }
    }
    if recent {
        (Plan::Local, DiskPower::Active)
    } else {
        (Plan::Wait, DiskPower::Idle)
    }
}

/// What a local query of a disk may refresh on this poll. It is the only
/// authorization to query: a disk whose answer is `Nothing` is not touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalRead {
    /// Every temperature of the disk.
    All,
    /// The additional sensors only: the main temperature is the service's.
    Additional,
    Nothing,
}

/// `recent` is [`Activity::recent`]; `additional` says whether the disk has
/// temperature sensors besides the main one. Those are the core's alone: on
/// a disk whose main temperature the service measures they follow the local
/// activity rule on their own. A disk that waits is never asked.
pub fn local_read(plan: Plan, recent: bool, additional: bool) -> LocalRead {
    match plan {
        Plan::Local => LocalRead::All,
        Plan::Service(_) if recent && additional => LocalRead::Additional,
        Plan::Service(_) | Plan::Wait => LocalRead::Nothing,
    }
}

/// Where the temperature of a disk comes from on this poll.
pub fn plan(
    class: DiskClass,
    powered_on: Option<bool>,
    service: &ServiceDisk,
    recent: bool,
) -> Plan {
    decide(class, powered_on, service, recent).0
}

/// The state of the disk that goes with [`plan`].
pub fn power(
    class: DiskClass,
    powered_on: Option<bool>,
    service: &ServiceDisk,
    recent: bool,
) -> DiskPower {
    decide(class, powered_on, service, recent).1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime};

    use DiskClass::{NonRotational, RotationalOrUnknown};

    const TEMPERATURE: ServiceTemperature = ServiceTemperature {
        value: 37.0,
        held: false,
    };

    fn present(
        state: DriveState,
        blocks_smart: bool,
        temperature: Option<ServiceTemperature>,
    ) -> ServiceDisk {
        ServiceDisk::Present {
            state,
            blocks_smart,
            temperature,
        }
    }

    /// Both public answers, so the tests cover `plan` and `power` themselves.
    fn decide(
        class: DiskClass,
        powered_on: Option<bool>,
        service: &ServiceDisk,
        recent: bool,
    ) -> (Plan, DiskPower) {
        (
            plan(class, powered_on, service, recent),
            power(class, powered_on, service, recent),
        )
    }

    /// `mono_s` and `wall_s` seconds after a common origin on each clock.
    fn stamp(start: Instant, mono_s: u64, wall_s: u64) -> Stamp {
        Stamp {
            mono: start + Duration::from_secs(mono_s),
            wall: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + wall_s),
        }
    }

    #[test]
    fn the_decision_table_matches_the_spec() {
        let absent = ServiceDisk::Absent;
        let active = present(DriveState::Active, false, Some(TEMPERATURE));

        // A disk Windows switched off is never read, whatever else is known.
        for class in [NonRotational, RotationalOrUnknown] {
            for service in [&absent, &active] {
                assert_eq!(
                    decide(class, Some(false), service, true),
                    (Plan::Wait, DiskPower::Standby)
                );
            }
        }
        // No media: nothing to read, and no evidence of a standby.
        let no_media = present(DriveState::NoMedia, false, Some(TEMPERATURE));
        for class in [NonRotational, RotationalOrUnknown] {
            assert_eq!(
                decide(class, Some(true), &no_media, true),
                (Plan::Wait, DiskPower::Unknown)
            );
        }
        // A non-rotational disk is read as before the gate, in every other state.
        let blocking_standby = present(DriveState::Standby, true, None);
        for service in [&absent, &active, &blocking_standby] {
            for (powered_on, recent) in [(Some(true), false), (None, false), (None, true)] {
                assert_eq!(
                    decide(NonRotational, powered_on, service, recent),
                    (Plan::Local, DiskPower::Active)
                );
            }
        }
        // The gate veto: a disk holding the SMART gate closed is not read
        // locally either, even with recent activity.
        assert_eq!(
            decide(RotationalOrUnknown, Some(true), &blocking_standby, true),
            (Plan::Wait, DiskPower::Standby)
        );
        for state in [
            DriveState::Unknown,
            DriveState::SmartOff,
            DriveState::Active,
            DriveState::Idle,
        ] {
            assert_eq!(
                decide(
                    RotationalOrUnknown,
                    Some(true),
                    &present(state, true, Some(TEMPERATURE)),
                    true
                ),
                (Plan::Wait, DiskPower::Unknown),
                "{state:?}"
            );
        }
        // A confirmed standby, even when the service has a temperature.
        for temperature in [None, Some(TEMPERATURE)] {
            assert_eq!(
                decide(
                    RotationalOrUnknown,
                    Some(true),
                    &present(DriveState::Standby, false, temperature),
                    true
                ),
                (Plan::Wait, DiskPower::Standby)
            );
        }
        // An idle disk is the service's to read, and the service sends it
        // nothing: no local query either, even while the disk works.
        for temperature in [None, Some(TEMPERATURE)] {
            for recent in [false, true] {
                assert_eq!(
                    decide(
                        RotationalOrUnknown,
                        Some(true),
                        &present(DriveState::Idle, false, temperature),
                        recent
                    ),
                    (Plan::Wait, DiskPower::Idle)
                );
            }
        }
        // The service's temperature of an active disk replaces the local query.
        for recent in [false, true] {
            assert_eq!(
                decide(RotationalOrUnknown, Some(true), &active, recent),
                (Plan::Service(TEMPERATURE), DiskPower::Active)
            );
        }
        // Everything else follows the recent activity.
        let others = [
            absent,
            present(DriveState::Active, false, None),
            present(DriveState::SmartOff, false, None),
            present(DriveState::SmartOff, false, Some(TEMPERATURE)),
            present(DriveState::Unknown, false, None),
            present(DriveState::Unknown, false, Some(TEMPERATURE)),
        ];
        for service in &others {
            for powered_on in [Some(true), None] {
                assert_eq!(
                    decide(RotationalOrUnknown, powered_on, service, true),
                    (Plan::Local, DiskPower::Active),
                    "{service:?}"
                );
                assert_eq!(
                    decide(RotationalOrUnknown, powered_on, service, false),
                    (Plan::Wait, DiskPower::Idle),
                    "{service:?}"
                );
            }
        }

        // The three cases the plan spells out.
        assert_eq!(
            plan(
                RotationalOrUnknown,
                Some(true),
                &present(DriveState::SmartOff, false, None),
                true
            ),
            Plan::Local
        );
        assert_eq!(
            decide(
                RotationalOrUnknown,
                Some(true),
                &present(DriveState::Active, false, None),
                false
            ),
            (Plan::Wait, DiskPower::Idle)
        );
        assert_eq!(
            decide(
                RotationalOrUnknown,
                Some(true),
                &present(DriveState::Standby, false, None),
                true
            ),
            (Plan::Wait, DiskPower::Standby)
        );
    }

    #[test]
    fn a_local_read_needs_a_local_plan_or_additional_sensors_with_activity() {
        // The core's own read refreshes everything, whatever else is known:
        // `plan` already weighed the activity.
        for recent in [false, true] {
            for additional in [false, true] {
                assert_eq!(local_read(Plan::Local, recent, additional), LocalRead::All);
                assert_eq!(
                    local_read(Plan::Wait, recent, additional),
                    LocalRead::Nothing
                );
            }
        }
        // The service measures the main temperature: the disk is asked only
        // for sensors the service does not have, and only while it works.
        let service = Plan::Service(TEMPERATURE);
        assert_eq!(local_read(service, true, true), LocalRead::Additional);
        assert_eq!(local_read(service, false, true), LocalRead::Nothing);
        assert_eq!(local_read(service, true, false), LocalRead::Nothing);
        assert_eq!(local_read(service, false, false), LocalRead::Nothing);
    }

    #[test]
    fn nvme_and_virtual_buses_are_non_rotational() {
        for bus in [14, 15, 16, 17] {
            for seek_penalty in [None, Some(true), Some(false)] {
                assert_eq!(disk_class(Some(bus), seek_penalty), NonRotational, "{bus}");
            }
        }
        // SATA (11) and USB (7) follow the seek penalty; no answer is not a "no".
        assert_eq!(disk_class(Some(11), Some(true)), RotationalOrUnknown);
        assert_eq!(disk_class(Some(11), None), RotationalOrUnknown);
        assert_eq!(disk_class(Some(7), None), RotationalOrUnknown);
        assert_eq!(disk_class(None, None), RotationalOrUnknown);
        assert_eq!(disk_class(None, Some(true)), RotationalOrUnknown);
        assert_eq!(disk_class(Some(11), Some(false)), NonRotational);
        assert_eq!(disk_class(None, Some(false)), NonRotational);
    }

    #[test]
    fn activity_needs_a_positive_finite_rate_within_ten_seconds() {
        let start = Instant::now();
        let mut activity = Activity::default();
        assert!(!activity.recent(&stamp(start, 0, 0)), "nothing observed");
        activity.observe(Some(0.0), Some(4096.0), &stamp(start, 0, 0));
        assert!(activity.recent(&stamp(start, 0, 0)));
        assert!(activity.recent(&stamp(start, 10, 10)));
        assert!(!activity.recent(&stamp(start, 11, 11)));

        // A read counts as much as a write.
        let mut reading = Activity::default();
        reading.observe(Some(512.0), Some(0.0), &stamp(start, 0, 0));
        assert!(reading.recent(&stamp(start, 1, 1)));

        // A valid zero does not renew the window, and does not close it early.
        activity.observe(Some(0.0), Some(0.0), &stamp(start, 5, 5));
        assert!(activity.recent(&stamp(start, 10, 10)));
        assert!(!activity.recent(&stamp(start, 11, 11)));
        // A later positive sample does renew it.
        activity.observe(Some(1.0), Some(0.0), &stamp(start, 9, 9));
        assert!(activity.recent(&stamp(start, 19, 19)));
        assert!(!activity.recent(&stamp(start, 20, 20)));
    }

    #[test]
    fn a_warm_up_or_missing_counter_is_not_activity() {
        let start = Instant::now();
        let samples = [
            (None, None),
            (Some(f64::NAN), None),
            (Some(0.0), Some(0.0)),
            (Some(f64::INFINITY), Some(0.0)),
            (Some(f64::NAN), Some(4096.0)),
            (None, Some(4096.0)),
            (Some(-1.0), Some(4096.0)),
        ];
        let mut activity = Activity::default();
        for (second, (read, write)) in samples.into_iter().enumerate() {
            let at = stamp(start, second as u64, second as u64);
            activity.observe(read, write, &at);
            assert!(!activity.recent(&at), "{read:?} {write:?}");
        }
    }

    #[test]
    fn activity_before_a_suspend_does_not_count() {
        let start = Instant::now();
        let mut activity = Activity::default();
        activity.observe(Some(4096.0), Some(4096.0), &stamp(start, 0, 0));
        // An hour of suspend the monotonic clock did not count: the old
        // activity must not authorize a read on resume, even before the next
        // sample arrives.
        assert!(!activity.recent(&stamp(start, 2, 3_600)));
        activity.observe(Some(0.0), Some(0.0), &stamp(start, 2, 3_600));
        assert!(!activity.recent(&stamp(start, 2, 3_600)));
        assert!(!activity.recent(&stamp(start, 3, 3_601)));

        // A wall clock set back is not trusted either.
        let mut set_back = Activity::default();
        let later = stamp(start, 0, 7_200);
        set_back.observe(Some(4096.0), Some(0.0), &later);
        let earlier = stamp(start, 1, 0);
        assert!(!set_back.recent(&earlier));
        set_back.observe(Some(0.0), Some(0.0), &earlier);
        assert!(!set_back.recent(&earlier));
    }

    #[test]
    fn a_sampling_gap_invalidates_the_activity_window() {
        let start = Instant::now();
        let busy = (Some(4096.0), Some(4096.0));

        // A gap longer than the window: the first sample after it spans the
        // gap (warm-up), so even a positive rate does not authorize a read.
        let mut activity = Activity::default();
        activity.observe(busy.0, busy.1, &stamp(start, 0, 0));
        activity.observe(busy.0, busy.1, &stamp(start, 11, 11));
        assert!(!activity.recent(&stamp(start, 11, 11)));
        // Only the next valid positive sample does.
        activity.observe(busy.0, busy.1, &stamp(start, 12, 12));
        assert!(activity.recent(&stamp(start, 12, 12)));

        // The same when only the wall clock shows the gap (a suspend).
        let mut suspended = Activity::default();
        suspended.observe(busy.0, busy.1, &stamp(start, 0, 0));
        suspended.observe(busy.0, busy.1, &stamp(start, 1, 600));
        assert!(!suspended.recent(&stamp(start, 1, 600)));
        suspended.observe(busy.0, busy.1, &stamp(start, 2, 601));
        assert!(suspended.recent(&stamp(start, 2, 601)));

        // Exactly the window is not a gap.
        let mut slow = Activity::default();
        slow.observe(Some(0.0), Some(0.0), &stamp(start, 0, 0));
        slow.observe(busy.0, busy.1, &stamp(start, 10, 10));
        assert!(slow.recent(&stamp(start, 10, 10)));

        // Missing counters on the current poll withdraw the authorization at
        // once; the next valid positive sample gives it back.
        let mut missing = Activity::default();
        missing.observe(busy.0, busy.1, &stamp(start, 0, 0));
        missing.observe(None, None, &stamp(start, 1, 1));
        assert!(!missing.recent(&stamp(start, 1, 1)));
        missing.observe(Some(0.0), Some(0.0), &stamp(start, 2, 2));
        assert!(!missing.recent(&stamp(start, 2, 2)), "nothing to keep");
        missing.observe(busy.0, busy.1, &stamp(start, 3, 3));
        assert!(missing.recent(&stamp(start, 3, 3)));
    }
}
