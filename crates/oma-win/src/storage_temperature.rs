//! Disk temperatures from `StorageDeviceTemperatureProperty`: no administrator
//! rights needed; support depends on the drive and its driver (a drive without
//! support answers ERROR_INVALID_FUNCTION and simply has no sensors).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use oma_core::model::Label;
use windows::Win32::System::Ioctl::{
    StorageDeviceTemperatureProperty, STORAGE_TEMPERATURE_DATA_DESCRIPTOR,
    STORAGE_TEMPERATURE_INFO, STORAGE_TEMPERATURE_VALUE_NOT_REPORTED,
};

use crate::storage_ioctl::{le_i16, le_u16, PhysicalDrive};

/// STORAGE_TEMPERATURE_DATA_DESCRIPTOR field offsets.
const CRITICAL: usize = 8;
const WARNING: usize = 10;
const INFO_COUNT: usize = 12;
const INFO: usize = 24;
/// STORAGE_TEMPERATURE_INFO size and `Temperature` offset.
const INFO_SIZE: usize = 16;
const INFO_TEMPERATURE: usize = 2;

const _: () = assert!(
    std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, CriticalTemperature) == CRITICAL
);
const _: () = assert!(
    std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, WarningTemperature) == WARNING
);
const _: () =
    assert!(std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, InfoCount) == INFO_COUNT);
const _: () =
    assert!(std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, TemperatureInfo) == INFO);
const _: () = assert!(size_of::<STORAGE_TEMPERATURE_INFO>() == INFO_SIZE);
const _: () =
    assert!(std::mem::offset_of!(STORAGE_TEMPERATURE_INFO, Temperature) == INFO_TEMPERATURE);

/// Spec §4.1: disk health data is refreshed every 30 s; the last values are
/// repeated in between.
pub(crate) const TEMPERATURE_PERIOD: Duration = Duration::from_secs(30);

/// One `StorageDeviceTemperatureProperty` answer, in °C.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TemperatureReport {
    /// Driver Index -> value (0 = composite); independent of descriptor order.
    /// `None` = not reported. Duplicate indices invalidate the report.
    pub sensors: BTreeMap<usize, Option<f64>>,
    pub warning_c: Option<i16>,
    pub critical_c: Option<i16>,
}

fn reported(raw: i16) -> Option<i16> {
    (u32::from(raw as u16) != STORAGE_TEMPERATURE_VALUE_NOT_REPORTED).then_some(raw)
}

pub(crate) fn parse_temperatures(bytes: &[u8]) -> Option<TemperatureReport> {
    let count = usize::from(le_u16(bytes, INFO_COUNT)?);
    let mut sensors = BTreeMap::new();
    for i in 0..count {
        let at = INFO + i * INFO_SIZE;
        let Some(record) = bytes.get(at..at + INFO_SIZE) else {
            break;
        };
        let index = usize::from(le_u16(record, 0)?);
        let value = reported(le_i16(record, INFO_TEMPERATURE)?).map(f64::from);
        if sensors.insert(index, value).is_some() {
            return None;
        }
    }
    let threshold = |offset| le_i16(bytes, offset).and_then(reported).filter(|&t| t > 0);
    Some(TemperatureReport {
        sensors,
        warning_c: threshold(WARNING),
        critical_c: threshold(CRITICAL),
    })
}

/// Queries the drive; `None` if the drive does not support the property.
pub(crate) fn query_temperatures(drive: &PhysicalDrive) -> Option<TemperatureReport> {
    parse_temperatures(&drive.query_property(StorageDeviceTemperatureProperty, 4096)?)
}

/// Driver indices that get a sensor: those reported at discovery, sorted by Index.
pub(crate) fn declared_positions(report: &TemperatureReport) -> Vec<usize> {
    report
        .sensors
        .iter()
        .filter_map(|(&index, value)| value.map(|_| index))
        .collect()
}

/// Sensor id segment: `drive` for driver Index 0, `sensor-<n>` otherwise.
pub(crate) fn sensor_name(position: usize) -> String {
    if position == 0 {
        "drive".to_owned()
    } else {
        format!("sensor-{position}")
    }
}

pub(crate) fn sensor_label(position: usize) -> Label {
    if position == 0 {
        Label::new("storage.temperature")
    } else {
        Label::with_arg("storage.temperatureSensor", position.to_string())
    }
}

/// Values in declared driver-index order; a missing report or index is `None`.
pub(crate) fn declared_values(
    report: Option<&TemperatureReport>,
    positions: &[usize],
) -> Vec<Option<f64>> {
    positions
        .iter()
        .map(|&p| report.and_then(|r| r.sensors.get(&p).copied().flatten()))
        .collect()
}

/// Device properties `tempWarningC` / `tempCriticalC`, when the drive reports them.
pub(crate) fn temperature_properties(
    report: Option<&TemperatureReport>,
) -> BTreeMap<String, String> {
    let mut properties = BTreeMap::new();
    if let Some(report) = report {
        if let Some(warning) = report.warning_c {
            properties.insert("tempWarningC".to_owned(), warning.to_string());
        }
        if let Some(critical) = report.critical_c {
            properties.insert("tempCriticalC".to_owned(), critical.to_string());
        }
    }
    properties
}

/// True when a read is at least `TEMPERATURE_PERIOD` old.
pub(crate) fn refresh_due(read_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(read_at) >= TEMPERATURE_PERIOD
}

/// The disk to refresh on this poll: the one with the oldest due read. One
/// disk per poll bounds the cost of a tick (an HDD answers in about 20 ms, an
/// NVMe drive leaving a low-power state in up to about 140 ms).
pub(crate) fn next_refresh(
    reads: impl IntoIterator<Item = (u32, Instant)>,
    now: Instant,
) -> Option<u32> {
    reads
        .into_iter()
        .filter(|&(_, read_at)| refresh_due(read_at, now))
        .min_by_key(|&(index, read_at)| (read_at, index))
        .map(|(index, _)| index)
}

/// A disk known to be spun down is not queried: the query could wake it up.
pub(crate) fn may_query(powered_on: Option<bool>) -> bool {
    powered_on != Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOT_REPORTED: i16 = i16::MIN; // 0x8000

    fn descriptor(critical: i16, warning: i16, temperatures: &[i16]) -> Vec<u8> {
        let mut bytes = vec![0u8; INFO + INFO_SIZE * temperatures.len().max(1)];
        bytes[CRITICAL..CRITICAL + 2].copy_from_slice(&critical.to_le_bytes());
        bytes[WARNING..WARNING + 2].copy_from_slice(&warning.to_le_bytes());
        bytes[INFO_COUNT..INFO_COUNT + 2]
            .copy_from_slice(&(temperatures.len() as u16).to_le_bytes());
        for (i, t) in temperatures.iter().enumerate() {
            let at = INFO + i * INFO_SIZE;
            bytes[at..at + 2].copy_from_slice(&(i as u16).to_le_bytes());
            bytes[at + INFO_TEMPERATURE..at + INFO_TEMPERATURE + 2]
                .copy_from_slice(&t.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn parses_an_nvme_report() {
        // Disk 2 of the development machine: composite plus two sensors.
        let report = parse_temperatures(&descriptor(95, 90, &[48, 48, 39])).unwrap();
        assert_eq!(
            report.sensors,
            BTreeMap::from([(0, Some(48.0)), (1, Some(48.0)), (2, Some(39.0))])
        );
        assert_eq!(report.warning_c, Some(90));
        assert_eq!(report.critical_c, Some(95));
    }

    #[test]
    fn not_reported_values_and_thresholds_are_absent() {
        // Disk 0 of the development machine (SATA HDD): no critical threshold.
        let report =
            parse_temperatures(&descriptor(NOT_REPORTED, 60, &[39, NOT_REPORTED])).unwrap();
        assert_eq!(report.sensors, BTreeMap::from([(0, Some(39.0)), (1, None)]));
        assert_eq!(report.warning_c, Some(60));
        assert_eq!(report.critical_c, None);
        let zero = parse_temperatures(&descriptor(0, -5, &[30])).unwrap();
        assert_eq!((zero.warning_c, zero.critical_c), (None, None));
    }

    #[test]
    fn truncated_descriptors_are_safe() {
        let mut bytes = descriptor(95, 90, &[48, 48, 39]);
        bytes.truncate(INFO + INFO_SIZE + 3); // only the first entry fits
        assert_eq!(
            parse_temperatures(&bytes).unwrap().sensors,
            BTreeMap::from([(0, Some(48.0))])
        );
        assert_eq!(parse_temperatures(&bytes[..12]), None);
    }

    #[test]
    fn sensors_are_declared_only_where_reported() {
        let report = parse_temperatures(&descriptor(95, 90, &[NOT_REPORTED, 41, 48])).unwrap();
        let positions = declared_positions(&report);
        assert_eq!(positions, vec![1, 2]);
        assert_eq!(sensor_name(0), "drive");
        assert_eq!(sensor_name(2), "sensor-2");
        assert_eq!(sensor_label(0), Label::new("storage.temperature"));
        assert_eq!(
            sensor_label(1),
            Label::with_arg("storage.temperatureSensor", "1")
        );
        assert_eq!(
            declared_values(Some(&report), &positions),
            vec![Some(41.0), Some(48.0)]
        );
    }

    #[test]
    fn declared_values_stay_aligned_when_a_refresh_changes() {
        let fewer = parse_temperatures(&descriptor(95, 90, &[47])).unwrap();
        assert_eq!(
            declared_values(Some(&fewer), &[0, 2]),
            vec![Some(47.0), None]
        );
        assert_eq!(declared_values(None, &[0, 2]), vec![None, None]);
    }

    #[test]
    fn sparse_reordered_indices_keep_their_identity() {
        let mut bytes = descriptor(95, 90, &[48, 39]);
        bytes[INFO..INFO + 2].copy_from_slice(&7u16.to_le_bytes());
        bytes[INFO + INFO_SIZE..INFO + INFO_SIZE + 2].copy_from_slice(&0u16.to_le_bytes());
        let report = parse_temperatures(&bytes).unwrap();
        assert_eq!(declared_positions(&report), vec![0, 7]);
        assert_eq!(
            declared_values(Some(&report), &[0, 7, 1]),
            vec![Some(39.0), Some(48.0), None]
        );
        let mut reversed = bytes.clone();
        reversed[INFO..INFO + INFO_SIZE]
            .copy_from_slice(&bytes[INFO + INFO_SIZE..INFO + 2 * INFO_SIZE]);
        reversed[INFO + INFO_SIZE..INFO + 2 * INFO_SIZE]
            .copy_from_slice(&bytes[INFO..INFO + INFO_SIZE]);
        assert_eq!(parse_temperatures(&reversed), Some(report));
        bytes[INFO..INFO + 2].copy_from_slice(&0u16.to_le_bytes());
        assert!(
            parse_temperatures(&bytes).is_none(),
            "duplicate sensor identity"
        );
    }

    #[test]
    fn thresholds_become_device_properties() {
        let nvme = parse_temperatures(&descriptor(87, 86, &[47])).unwrap();
        let properties = temperature_properties(Some(&nvme));
        assert_eq!(properties["tempWarningC"], "86");
        assert_eq!(properties["tempCriticalC"], "87");
        let hdd = parse_temperatures(&descriptor(NOT_REPORTED, 60, &[39])).unwrap();
        assert!(!temperature_properties(Some(&hdd)).contains_key("tempCriticalC"));
        assert!(temperature_properties(None).is_empty());
    }

    #[test]
    fn refresh_every_thirty_seconds() {
        let start = Instant::now();
        assert!(!refresh_due(start, start + Duration::from_secs(29)));
        assert!(refresh_due(start, start + TEMPERATURE_PERIOD));
        // A read stamped after `now` is never due.
        assert!(!refresh_due(start + Duration::from_secs(5), start));
    }

    #[test]
    fn one_disk_per_poll_oldest_first() {
        let start = Instant::now();
        let reads = [
            (0, start + Duration::from_secs(2)),
            (2, start),
            (3, start + Duration::from_secs(40)),
        ];
        let now = start + Duration::from_secs(33);
        assert_eq!(next_refresh(reads, now), Some(2));
        assert_eq!(
            next_refresh([(0, start + Duration::from_secs(2))], now),
            Some(0)
        );
        assert_eq!(next_refresh(reads, start + Duration::from_secs(10)), None);
        assert_eq!(next_refresh([], now), None);
        // Equal read times: the lowest disk index first, deterministically.
        assert_eq!(next_refresh([(3, start), (1, start)], now), Some(1));
    }

    #[test]
    fn sleeping_disks_are_not_queried() {
        assert!(may_query(Some(true)));
        assert!(may_query(None));
        assert!(!may_query(Some(false)));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn reads_disk_temperatures_on_this_machine() {
        let mut with_temperature = 0;
        for index in 0..16 {
            let Some(drive) = PhysicalDrive::open(index) else {
                continue;
            };
            let powered = drive.powered_on();
            if !may_query(powered) {
                println!("disk {index}: spun down, not queried");
                continue;
            }
            let started = Instant::now();
            let report = query_temperatures(&drive);
            let elapsed = started.elapsed();
            println!("disk {index}: powered {powered:?}, {elapsed:?}, {report:?}");
            // A single disk must fit in the 200 ms tick deadline.
            assert!(
                elapsed < Duration::from_millis(200),
                "disk {index}: {elapsed:?}"
            );
            if let Some(Some(celsius)) = report.as_ref().and_then(|r| r.sensors.get(&0)) {
                assert!((5.0..=90.0).contains(celsius), "disk {index}: {celsius} °C");
                with_temperature += 1;
            }
        }
        assert!(with_temperature >= 1, "no disk reports a temperature");
    }
}
