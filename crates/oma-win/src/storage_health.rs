//! NVMe SMART/Health log (log page 02h) read without administrator rights:
//! `IOCTL_STORAGE_QUERY_PROPERTY` with `StorageDeviceProtocolSpecificProperty`
//! on a disk handle opened with zero access (spike M5 S1 §2.5). Only NVMe
//! disks (`BusTypeNvme`) are ever asked; SATA disks never get a command.

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime};

use oma_core::model::{Label, Sensor, SensorKind, Source, Unit};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED,
    WIN32_ERROR,
};
use windows::Win32::Storage::FileSystem::BusTypeNvme;
use windows::Win32::System::Ioctl::{
    NVMeDataTypeLogPage, PropertyStandardQuery, ProtocolTypeNvme, StorageDeviceProperty,
    StorageDeviceProtocolSpecificProperty, IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_PROPERTY_QUERY, STORAGE_PROTOCOL_DATA_DESCRIPTOR, STORAGE_PROTOCOL_SPECIFIC_DATA,
};

use crate::storage_ioctl::{le_u32, PhysicalDrive};
use crate::storage_temperature::{may_query, TEMPERATURE_PERIOD};

/// Critical Warning bits that mean "this disk is failing" (spike S1 §2.3):
/// spare below threshold, reliability degraded, read-only, volatile backup
/// failed, persistent memory region read-only. Bit 1 (temperature) is left
/// to the temperature rule.
pub(crate) const CRITICAL_WARNING_MASK: u8 = 0x3D;

/// The fields of the SMART/Health log the rules use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NvmeHealth {
    pub critical_warning: u8,
    pub available_spare: u8,
    pub spare_threshold: u8,
    pub percentage_used: u8,
}

/// Why a health log read failed; it decides when to try again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HealthReadError {
    /// The drive or its driver does not answer the query: until rediscovery.
    Unsupported,
    /// The driver requires rights this process lacks: until rediscovery.
    AccessDenied,
    /// Anything else (disk asleep, I/O error): at the next refresh.
    Transient,
}

/// Bytes of `STORAGE_PROPERTY_QUERY` before `AdditionalParameters`.
const QUERY_HEADER: usize = 8;
/// `STORAGE_PROTOCOL_SPECIFIC_DATA` size, and the offsets of its fields.
const SPECIFIC_SIZE: usize = 40;
const SPECIFIC_PROTOCOL_TYPE: usize = 0;
const SPECIFIC_DATA_TYPE: usize = 4;
const SPECIFIC_REQUEST_VALUE: usize = 8;
const SPECIFIC_DATA_OFFSET: usize = 16;
const SPECIFIC_DATA_LENGTH: usize = 20;
/// `STORAGE_PROTOCOL_DATA_DESCRIPTOR`: `Version`, `Size`, then the
/// `STORAGE_PROTOCOL_SPECIFIC_DATA` the data offset of the answer is relative to.
const DESCRIPTOR_SIZE: usize = 48;
const DESCRIPTOR_SPECIFIC: usize = 8;
/// `STORAGE_DEVICE_DESCRIPTOR.BusType`.
const DEVICE_BUS_TYPE: usize = 28;
/// SMART / Health Information: log identifier 02h, 512 bytes (NVMe base spec).
const HEALTH_LOG_PAGE: u32 = 0x02;
const HEALTH_LOG_SIZE: usize = 512;
/// Byte offsets inside the log.
const LOG_CRITICAL_WARNING: usize = 0;
const LOG_AVAILABLE_SPARE: usize = 3;
const LOG_SPARE_THRESHOLD: usize = 4;
const LOG_PERCENTAGE_USED: usize = 5;
/// Query header, protocol request and room for the log in one buffer (560 bytes).
const REQUEST_SIZE: usize = QUERY_HEADER + SPECIFIC_SIZE + HEALTH_LOG_SIZE;

const _: () =
    assert!(std::mem::offset_of!(STORAGE_PROPERTY_QUERY, AdditionalParameters) == QUERY_HEADER);
const _: () = assert!(size_of::<STORAGE_PROTOCOL_SPECIFIC_DATA>() == SPECIFIC_SIZE);
const _: () = assert!(
    std::mem::offset_of!(STORAGE_PROTOCOL_SPECIFIC_DATA, ProtocolType) == SPECIFIC_PROTOCOL_TYPE
);
const _: () =
    assert!(std::mem::offset_of!(STORAGE_PROTOCOL_SPECIFIC_DATA, DataType) == SPECIFIC_DATA_TYPE);
const _: () = assert!(
    std::mem::offset_of!(STORAGE_PROTOCOL_SPECIFIC_DATA, ProtocolDataRequestValue)
        == SPECIFIC_REQUEST_VALUE
);
const _: () = assert!(
    std::mem::offset_of!(STORAGE_PROTOCOL_SPECIFIC_DATA, ProtocolDataOffset)
        == SPECIFIC_DATA_OFFSET
);
const _: () = assert!(
    std::mem::offset_of!(STORAGE_PROTOCOL_SPECIFIC_DATA, ProtocolDataLength)
        == SPECIFIC_DATA_LENGTH
);
const _: () = assert!(size_of::<STORAGE_PROTOCOL_DATA_DESCRIPTOR>() == DESCRIPTOR_SIZE);
const _: () = assert!(
    std::mem::offset_of!(STORAGE_PROTOCOL_DATA_DESCRIPTOR, ProtocolSpecificData)
        == DESCRIPTOR_SPECIFIC
);
const _: () = assert!(std::mem::offset_of!(STORAGE_DEVICE_DESCRIPTOR, BusType) == DEVICE_BUS_TYPE);

/// The input of the health log query as a fully initialised byte buffer
/// (see `PhysicalDrive::query_property`): the log is asked right after the
/// protocol-specific header.
pub(crate) fn health_log_request() -> Vec<u8> {
    let mut request = vec![0u8; REQUEST_SIZE];
    let mut put = |at: usize, value: u32| request[at..at + 4].copy_from_slice(&value.to_le_bytes());
    put(0, StorageDeviceProtocolSpecificProperty.0 as u32);
    put(4, PropertyStandardQuery.0 as u32);
    let specific = QUERY_HEADER;
    put(specific + SPECIFIC_PROTOCOL_TYPE, ProtocolTypeNvme.0 as u32);
    put(specific + SPECIFIC_DATA_TYPE, NVMeDataTypeLogPage.0 as u32);
    put(specific + SPECIFIC_REQUEST_VALUE, HEALTH_LOG_PAGE);
    put(specific + SPECIFIC_DATA_OFFSET, SPECIFIC_SIZE as u32);
    put(specific + SPECIFIC_DATA_LENGTH, HEALTH_LOG_SIZE as u32);
    request
}

/// The health fields of a `STORAGE_PROTOCOL_DATA_DESCRIPTOR` answer; `None`
/// unless the descriptor has the expected version and size, its data offset
/// (relative to `ProtocolSpecificData`) starts past the protocol header, and
/// the returned bytes hold a whole 512-byte log there.
pub(crate) fn parse_health_log(bytes: &[u8]) -> Option<NvmeHealth> {
    let version = le_u32(bytes, 0)? as usize;
    let size = le_u32(bytes, 4)? as usize;
    if version != DESCRIPTOR_SIZE || size != DESCRIPTOR_SIZE {
        return None;
    }
    let offset = le_u32(bytes, DESCRIPTOR_SPECIFIC + SPECIFIC_DATA_OFFSET)? as usize;
    let length = le_u32(bytes, DESCRIPTOR_SPECIFIC + SPECIFIC_DATA_LENGTH)? as usize;
    if offset < SPECIFIC_SIZE || length < HEALTH_LOG_SIZE {
        return None;
    }
    let start = DESCRIPTOR_SPECIFIC.checked_add(offset)?;
    let log = bytes.get(start..start.checked_add(HEALTH_LOG_SIZE)?)?;
    Some(NvmeHealth {
        critical_warning: log[LOG_CRITICAL_WARNING],
        available_spare: log[LOG_AVAILABLE_SPARE],
        spare_threshold: log[LOG_SPARE_THRESHOLD],
        percentage_used: log[LOG_PERCENTAGE_USED],
    })
}

/// `…/flag/critical-warning`: 1 when a bit of [`CRITICAL_WARNING_MASK`] is set.
pub(crate) fn critical_flag(warning: u8) -> f64 {
    if warning & CRITICAL_WARNING_MASK != 0 {
        1.0
    } else {
        0.0
    }
}

/// A failed query's error: "not supported" answers wait for a rediscovery,
/// anything unknown is retried at the next refresh.
pub(crate) fn classify(error: Option<WIN32_ERROR>) -> HealthReadError {
    match error {
        Some(ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED | ERROR_INVALID_PARAMETER) => {
            HealthReadError::Unsupported
        }
        Some(ERROR_ACCESS_DENIED) => HealthReadError::AccessDenied,
        _ => HealthReadError::Transient,
    }
}

/// Reads log page 02h. An answer this module cannot parse is a driver
/// quirk rather than a passing failure: [`HealthReadError::Unsupported`].
pub(crate) fn query_health_log(drive: &PhysicalDrive) -> Result<NvmeHealth, HealthReadError> {
    let answer = drive
        .ioctl_result(
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&health_log_request()),
            REQUEST_SIZE,
        )
        .map_err(classify)?;
    parse_health_log(&answer).ok_or(HealthReadError::Unsupported)
}

/// `STORAGE_DEVICE_DESCRIPTOR.BusType`.
pub(crate) fn parse_bus_type(descriptor: &[u8]) -> Option<i32> {
    le_u32(descriptor, DEVICE_BUS_TYPE).map(|bus| bus as i32)
}

/// The bus of disk `index`; `None` when its descriptor is unavailable.
pub(crate) fn bus_type(index: u32) -> Option<i32> {
    PhysicalDrive::open(index)?
        .query_property(StorageDeviceProperty, 65_536)
        .as_deref()
        .and_then(parse_bus_type)
}

fn is_nvme(bus_type: Option<i32>) -> bool {
    bus_type == Some(BusTypeNvme.0)
}

/// Health log of disk `index`, behind the same gate as the temperatures: a
/// disk known to be spun down is not queried (a transient failure).
pub(crate) fn read_health(index: u32) -> Result<NvmeHealth, HealthReadError> {
    let drive = PhysicalDrive::open(index).ok_or(HealthReadError::Transient)?;
    if !may_query(drive.powered_on()) {
        return Err(HealthReadError::Transient);
    }
    query_health_log(&drive)
}

/// The health sensors of an NVMe disk, with the ids the service gives the
/// same data (`SchemaBuilder.cs`), so the engine keeps these (the core's
/// providers come first) and drops the service's duplicates. None for any
/// other bus.
pub(crate) fn health_sensors(device_id: &str, bus_type: Option<i32>) -> Vec<Sensor> {
    if !is_nvme(bus_type) {
        return Vec::new();
    }
    vec![
        Sensor::new(
            device_id,
            SensorKind::Flag,
            "critical-warning",
            Unit::Boolean,
            Label::new("storage.criticalWarning"),
            Source::Win32,
        ),
        Sensor::new(
            device_id,
            SensorKind::Percent,
            "wear",
            Unit::Percent,
            Label::new("storage.percentUsed"),
            Source::Win32,
        ),
        Sensor::new(
            device_id,
            SensorKind::Percent,
            "available-spare",
            Unit::Percent,
            Label::new("storage.availableSpare"),
            Source::Win32,
        ),
    ]
}

/// Values in [`health_sensors`] order.
fn health_values(health: Option<&NvmeHealth>) -> [Option<f64>; 3] {
    match health {
        Some(h) => [
            Some(critical_flag(h.critical_warning)),
            Some(f64::from(h.percentage_used)),
            Some(f64::from(h.available_spare)),
        ],
        None => [None; 3],
    }
}

/// Device property `availableSpareThresholdPct`, as the service names it.
pub(crate) fn health_properties(health: Option<&NvmeHealth>) -> BTreeMap<String, String> {
    health
        .map(|h| {
            BTreeMap::from([(
                "availableSpareThresholdPct".to_owned(),
                h.spare_threshold.to_string(),
            )])
        })
        .unwrap_or_default()
}

/// A point in time on both clocks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Stamp {
    pub mono: Instant,
    pub wall: SystemTime,
}

impl Stamp {
    pub(crate) fn now() -> Self {
        Self {
            mono: Instant::now(),
            wall: SystemTime::now(),
        }
    }

    /// Time from `self` to `later` on whichever clock moved more: a suspend
    /// the monotonic clock does not count still shows on the wall clock, and
    /// a wall clock set back counts as time passed too.
    pub(crate) fn elapsed_until(&self, later: &Stamp) -> Duration {
        let mono = later.mono.saturating_duration_since(self.mono);
        let wall = later
            .wall
            .duration_since(self.wall)
            .unwrap_or_else(|earlier| earlier.duration());
        mono.max(wall)
    }
}

/// How long a read stays valid: until its refresh, due after
/// `TEMPERATURE_PERIOD`, plus one more period for the one-disk-per-poll
/// rotation to reach the disk. A refresh that has not happened by then (a
/// suspend, a stalled rotation) turns the values into `None` instead of
/// holding them.
pub(crate) const HEALTH_TTL: Duration = Duration::from_secs(2 * TEMPERATURE_PERIOD.as_secs());

/// What a refresh asks of the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HealthRefresh {
    Keep,
    /// First successful read of a disk without sensors: declare them.
    Rediscover,
    /// Unsupported or denied before any success: stop until the next discovery.
    Forget,
}

/// Health log state of one NVMe disk.
#[derive(Debug)]
pub(crate) struct DiskHealth {
    /// Its sensors are in the inventory.
    declared: bool,
    /// The latest read; `None` after a failed one.
    cached: Option<NvmeHealth>,
    read_at: Stamp,
}

impl DiskHealth {
    /// The state after the discovery read, which `read` performs only on an
    /// NVMe disk. `None` means nothing to track until the next discovery: a
    /// disk on another bus, or one whose driver refuses the query. After a
    /// transient failure the disk is kept, without sensors, for a retry.
    pub(crate) fn discover(
        bus_type: Option<i32>,
        read: impl FnOnce() -> Result<NvmeHealth, HealthReadError>,
        now: Stamp,
    ) -> Option<Self> {
        if !is_nvme(bus_type) {
            return None;
        }
        let cached = match read() {
            Ok(health) => Some(health),
            Err(HealthReadError::Transient) => None,
            Err(HealthReadError::Unsupported | HealthReadError::AccessDenied) => return None,
        };
        Some(Self {
            declared: cached.is_some(),
            cached,
            read_at: now,
        })
    }

    pub(crate) fn declared(&self) -> bool {
        self.declared
    }

    pub(crate) fn cached(&self) -> Option<&NvmeHealth> {
        self.cached.as_ref()
    }

    /// True once the last attempt is at least `TEMPERATURE_PERIOD` old.
    pub(crate) fn due(&self, now: &Stamp) -> bool {
        self.read_at.elapsed_until(now) >= TEMPERATURE_PERIOD
    }

    /// Stores the result of a refresh attempt. A failure after the sensors
    /// were declared leaves them in place, with no value.
    pub(crate) fn refresh(
        &mut self,
        result: Result<NvmeHealth, HealthReadError>,
        now: Stamp,
    ) -> HealthRefresh {
        self.read_at = now;
        self.cached = result.ok();
        match (self.declared, result) {
            (true, _) | (false, Err(HealthReadError::Transient)) => HealthRefresh::Keep,
            (false, Ok(_)) => HealthRefresh::Rediscover,
            (false, Err(_)) => HealthRefresh::Forget,
        }
    }

    /// Sensor values; `None` while the sensors are not declared.
    pub(crate) fn values(&self, now: &Stamp) -> Option<[Option<f64>; 3]> {
        if !self.declared {
            return None;
        }
        let fresh = self.read_at.elapsed_until(now) < HEALTH_TTL;
        Some(health_values(self.cached.as_ref().filter(|_| fresh)))
    }
}

/// The NVMe disk to refresh on a poll without a temperature refresh: the one
/// with the oldest due read (the lowest index on a tie).
pub(crate) fn next_health_refresh<'a>(
    disks: impl IntoIterator<Item = (u32, &'a DiskHealth)>,
    now: &Stamp,
) -> Option<u32> {
    disks
        .into_iter()
        .filter(|(_, disk)| disk.due(now))
        .min_by_key(|&(index, disk)| (disk.read_at.mono, index))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{ERROR_GEN_FAILURE, ERROR_IO_DEVICE};

    const NVME: Option<i32> = Some(17);
    const SATA: Option<i32> = Some(11);

    /// A `STORAGE_PROTOCOL_DATA_DESCRIPTOR` answer followed by a 512-byte
    /// log whose first bytes are `log`.
    fn answer(offset: u32, length: u32, log: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 8 + 40 + 512];
        bytes[0..4].copy_from_slice(&48u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&48u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&3u32.to_le_bytes()); // ProtocolTypeNvme
        bytes[12..16].copy_from_slice(&2u32.to_le_bytes()); // NVMeDataTypeLogPage
        bytes[16..20].copy_from_slice(&2u32.to_le_bytes()); // log page 02h
        bytes[24..28].copy_from_slice(&offset.to_le_bytes());
        bytes[28..32].copy_from_slice(&length.to_le_bytes());
        let start = 8 + offset as usize;
        if start < bytes.len() {
            let end = (start + log.len()).min(bytes.len());
            bytes[start..end].copy_from_slice(&log[..end - start]);
        }
        bytes
    }

    fn healthy() -> NvmeHealth {
        NvmeHealth {
            critical_warning: 0,
            available_spare: 100,
            spare_threshold: 10,
            percentage_used: 5,
        }
    }

    fn stamp(start: Instant, mono_s: u64, wall_s: u64) -> Stamp {
        Stamp {
            mono: start + Duration::from_secs(mono_s),
            wall: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + wall_s),
        }
    }

    #[test]
    fn parses_the_health_log() {
        let bytes = answer(40, 512, &[0x04, 0x3F, 0x01, 100, 10, 5]);
        assert_eq!(
            parse_health_log(&bytes),
            Some(NvmeHealth {
                critical_warning: 0x04,
                available_spare: 100,
                spare_threshold: 10,
                percentage_used: 5,
            })
        );
    }

    #[test]
    fn truncated_health_logs_are_safe() {
        let bytes = answer(40, 512, &[0x04, 0x3F, 0x01, 100, 10, 5]);
        // The driver reported fewer bytes than the whole log.
        for len in [0, 4, 8, 47, 48, 54, 100, 559] {
            assert_eq!(parse_health_log(&bytes[..len]), None, "{len} bytes");
        }
        assert!(parse_health_log(&bytes).is_some());
    }

    #[test]
    fn invalid_descriptor_offsets_and_lengths_are_rejected() {
        let log = [0x04, 0, 0, 100, 10, 5];
        let valid = answer(40, 512, &log);
        let mut wrong_version = valid.clone();
        wrong_version[0..4].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(parse_health_log(&wrong_version), None, "version");
        let mut wrong_size = valid.clone();
        wrong_size[4..8].copy_from_slice(&40u32.to_le_bytes());
        assert_eq!(parse_health_log(&wrong_size), None, "size");
        // The data may not overlap the protocol-specific header.
        assert_eq!(parse_health_log(&answer(8, 512, &log)), None, "offset");
        // A short log would leave the fields past its end undefined.
        assert_eq!(parse_health_log(&answer(40, 511, &log)), None, "length");
        // An offset past the returned bytes, or one that overflows.
        assert_eq!(parse_health_log(&answer(41, 512, &log)), None, "past end");
        assert_eq!(
            parse_health_log(&answer(u32::MAX, 512, &log)),
            None,
            "overflow"
        );
        let mut huge_length = valid.clone();
        huge_length[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            parse_health_log(&huge_length).map(|h| h.percentage_used),
            Some(5),
            "a longer log only needs its first 512 bytes"
        );
        // The offset is relative to ProtocolSpecificData (byte 8), not to the buffer.
        let mut shifted = vec![0u8; 8 + 48 + 512];
        shifted[..valid.len()].copy_from_slice(&valid);
        shifted[24..28].copy_from_slice(&48u32.to_le_bytes());
        shifted[8 + 48] = 0x01;
        shifted[8 + 48 + 5] = 42;
        let parsed = parse_health_log(&shifted).expect("offset 48");
        assert_eq!(
            (parsed.critical_warning, parsed.percentage_used),
            (0x01, 42)
        );
    }

    #[test]
    fn the_request_asks_for_log_page_two() {
        let request = health_log_request();
        assert_eq!(request.len(), 8 + 40 + 512);
        let u32_at = |at: usize| u32::from_le_bytes(request[at..at + 4].try_into().unwrap());
        assert_eq!(u32_at(0), 50, "StorageDeviceProtocolSpecificProperty");
        assert_eq!(u32_at(4), 0, "PropertyStandardQuery");
        assert_eq!(u32_at(8), 3, "ProtocolTypeNvme");
        assert_eq!(u32_at(12), 2, "NVMeDataTypeLogPage");
        assert_eq!(u32_at(16), 2, "log page 02h");
        assert_eq!(u32_at(20), 0, "no log offset");
        assert_eq!(u32_at(24), 40, "data right after the protocol header");
        assert_eq!(u32_at(28), 512);
        assert!(request[32..].iter().all(|&b| b == 0));
    }

    #[test]
    fn critical_flag_ignores_the_temperature_bit() {
        assert_eq!(critical_flag(0x00), 0.0);
        assert_eq!(critical_flag(0x02), 0.0);
        assert_eq!(critical_flag(0x01), 1.0);
        assert_eq!(critical_flag(0x3D), 1.0);
        assert_eq!(critical_flag(0x3F), 1.0);
        assert_eq!(critical_flag(0xC0), 0.0);
    }

    #[test]
    fn read_errors_are_classified() {
        use HealthReadError::*;
        assert_eq!(classify(Some(ERROR_INVALID_FUNCTION)), Unsupported);
        assert_eq!(classify(Some(ERROR_NOT_SUPPORTED)), Unsupported);
        assert_eq!(classify(Some(ERROR_INVALID_PARAMETER)), Unsupported);
        assert_eq!(classify(Some(ERROR_ACCESS_DENIED)), AccessDenied);
        assert_eq!(classify(Some(ERROR_IO_DEVICE)), Transient);
        assert_eq!(classify(Some(ERROR_GEN_FAILURE)), Transient);
        assert_eq!(classify(None), Transient);
    }

    #[test]
    fn bus_type_comes_from_the_device_descriptor() {
        let mut descriptor = vec![0u8; 40];
        descriptor[28..32].copy_from_slice(&17u32.to_le_bytes());
        assert_eq!(parse_bus_type(&descriptor), NVME);
        descriptor[28..32].copy_from_slice(&11u32.to_le_bytes());
        assert_eq!(parse_bus_type(&descriptor), SATA);
        assert_eq!(parse_bus_type(&descriptor[..31]), None);
    }

    #[test]
    fn health_sensors_only_for_nvme() {
        let ids: Vec<String> = health_sensors("storage/device-a", NVME)
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "storage/device-a/flag/critical-warning",
                "storage/device-a/percent/wear",
                "storage/device-a/percent/available-spare",
            ]
        );
        let sensors = health_sensors("storage/device-a", NVME);
        assert_eq!(
            (sensors[0].kind, sensors[0].unit),
            (SensorKind::Flag, Unit::Boolean)
        );
        assert_eq!(sensors[0].label, Label::new("storage.criticalWarning"));
        assert_eq!(sensors[1].label, Label::new("storage.percentUsed"));
        assert_eq!(sensors[2].label, Label::new("storage.availableSpare"));
        assert!(sensors
            .iter()
            .all(|s| s.source == Source::Win32 && s.device_id == "storage/device-a"));
        assert!(health_sensors("storage/device-a", SATA).is_empty());
        assert!(health_sensors("storage/device-a", None).is_empty());

        // A SATA disk (or one of unknown bus) is never sent the NVMe command.
        let now = stamp(Instant::now(), 0, 0);
        for bus in [SATA, None] {
            let state = DiskHealth::discover(bus, || panic!("queried a non-NVMe disk"), now);
            assert!(state.is_none());
        }
        let nvme = DiskHealth::discover(NVME, || Ok(healthy()), now).expect("tracked");
        assert!(nvme.declared());
    }

    #[test]
    fn values_follow_the_log() {
        let start = Instant::now();
        let mut failing = healthy();
        failing.critical_warning = 0x06;
        failing.percentage_used = 120;
        let disk = DiskHealth::discover(NVME, || Ok(failing), stamp(start, 0, 0)).unwrap();
        assert_eq!(
            disk.values(&stamp(start, 1, 1)),
            Some([Some(1.0), Some(120.0), Some(100.0)])
        );
        assert_eq!(
            health_properties(disk.cached())
                .get("availableSpareThresholdPct")
                .map(String::as_str),
            Some("10")
        );
        assert!(health_properties(None).is_empty());
    }

    #[test]
    fn transient_discovery_failure_is_retried() {
        let start = Instant::now();
        let mut disk =
            DiskHealth::discover(NVME, || Err(HealthReadError::Transient), stamp(start, 0, 0))
                .expect("kept for a retry");
        assert!(!disk.declared(), "no sensors before a successful read");
        assert_eq!(disk.values(&stamp(start, 1, 1)), None);
        assert!(!disk.due(&stamp(start, 29, 29)));
        assert!(disk.due(&stamp(start, 30, 30)));
        assert_eq!(
            disk.refresh(Err(HealthReadError::Transient), stamp(start, 30, 30)),
            HealthRefresh::Keep
        );
        assert!(!disk.due(&stamp(start, 31, 31)));
        assert_eq!(
            disk.refresh(Ok(healthy()), stamp(start, 60, 60)),
            HealthRefresh::Rediscover,
            "the first success declares the sensors through a rediscovery"
        );
        assert!(!disk.declared(), "the schema changes only in discover");

        // Unsupported or denied at discovery: nothing until the next discovery.
        for error in [HealthReadError::Unsupported, HealthReadError::AccessDenied] {
            assert!(DiskHealth::discover(NVME, || Err(error), stamp(start, 0, 0)).is_none());
            let mut retrying =
                DiskHealth::discover(NVME, || Err(HealthReadError::Transient), stamp(start, 0, 0))
                    .unwrap();
            assert_eq!(
                retrying.refresh(Err(error), stamp(start, 30, 30)),
                HealthRefresh::Forget
            );
        }
    }

    #[test]
    fn failed_refresh_clears_cached_health() {
        let start = Instant::now();
        let mut disk = DiskHealth::discover(NVME, || Ok(healthy()), stamp(start, 0, 0)).unwrap();
        assert_eq!(
            disk.values(&stamp(start, 10, 10)),
            Some([Some(0.0), Some(5.0), Some(100.0)])
        );
        for error in [
            HealthReadError::Transient,
            HealthReadError::Unsupported,
            HealthReadError::AccessDenied,
        ] {
            assert_eq!(
                disk.refresh(Err(error), stamp(start, 30, 30)),
                HealthRefresh::Keep,
                "declared sensors stay"
            );
            assert!(disk.declared());
            assert_eq!(disk.values(&stamp(start, 31, 31)), Some([None, None, None]));
            assert_eq!(
                disk.refresh(Ok(healthy()), stamp(start, 30, 30)),
                HealthRefresh::Keep
            );
            assert_eq!(
                disk.values(&stamp(start, 31, 31)),
                Some([Some(0.0), Some(5.0), Some(100.0)])
            );
        }
    }

    #[test]
    fn health_cache_expires_after_suspend() {
        let start = Instant::now();
        let disk = DiskHealth::discover(NVME, || Ok(healthy()), stamp(start, 0, 0)).unwrap();
        // The one-disk-per-poll rotation may reach the disk a few polls late.
        assert!(disk.values(&stamp(start, 45, 45)).unwrap()[1].is_some());
        assert_eq!(
            disk.values(&stamp(start, HEALTH_TTL.as_secs(), HEALTH_TTL.as_secs())),
            Some([None, None, None]),
            "never held past the missed refresh"
        );
        // Two hours of suspend that the monotonic clock did not count.
        let resumed = stamp(start, 1, 7_200);
        assert_eq!(disk.values(&resumed), Some([None, None, None]));
        assert!(
            disk.due(&resumed),
            "refreshed on the first poll after resume"
        );
        // A wall clock set back does not keep it alive either.
        let set_back = Stamp {
            mono: start + Duration::from_secs(1),
            wall: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - 7_200),
        };
        assert_eq!(disk.values(&set_back), Some([None, None, None]));
        assert!(TEMPERATURE_PERIOD < HEALTH_TTL);
    }

    #[test]
    fn one_health_refresh_per_poll_oldest_first() {
        let start = Instant::now();
        let at = |s| DiskHealth::discover(NVME, || Ok(healthy()), stamp(start, s, s)).unwrap();
        let (a, b, c) = (at(5), at(0), at(20));
        let now = stamp(start, 36, 36);
        assert_eq!(
            next_health_refresh([(2, &a), (3, &b), (4, &c)], &now),
            Some(3)
        );
        assert_eq!(
            next_health_refresh([(2, &a), (4, &c)], &now),
            Some(2),
            "c is not due yet"
        );
        assert_eq!(next_health_refresh([(4, &c)], &now), None);
        assert_eq!(next_health_refresh([(7, &b), (1, &at(0))], &now), Some(1));
    }

    #[test]
    fn nvme_wear_above_100_survives_sanitization() {
        use oma_core::sanitize::sanitize_sensor;
        let sensors = health_sensors("storage/device-a", NVME);
        let [flag, wear, spare] = &sensors[..] else {
            panic!("three sensors");
        };
        assert_eq!(wear.source, Source::Win32);
        assert_eq!(sanitize_sensor(wear, Some(120.0)), Some(120.0));
        assert_eq!(sanitize_sensor(wear, Some(255.0)), Some(255.0));
        assert_eq!(sanitize_sensor(wear, Some(256.0)), None);
        assert_eq!(sanitize_sensor(spare, Some(100.0)), Some(100.0));
        assert_eq!(sanitize_sensor(spare, Some(255.0)), None, "spare is 0-100");
        assert_eq!(sanitize_sensor(flag, Some(1.0)), Some(1.0));
        assert_eq!(sanitize_sensor(flag, Some(4.0)), None);
    }

    #[test]
    fn core_and_service_health_ids_deduplicate() {
        use crate::storage::{DriveEntry, DriveIds};
        use crate::svc::provider::bind;
        use crate::svc::SourceRequest;
        use oma_ipc::{IdentityHint, WireDevice, WireSchema, WireSensor};

        let core_id = "storage/device-abc";
        let drives = DriveIds {
            generation: 1,
            drives: vec![DriveEntry::new(
                2,
                core_id.to_owned(),
                Some("Fanxiang S880 2TB".to_owned()),
                Some("SN2".to_owned()),
            )],
        };
        let wire = |kind: &str, name: &str, unit: &str, label: &str| WireSensor {
            device_id: "svc-nvme".to_owned(),
            kind: kind.to_owned(),
            name: name.to_owned(),
            unit: unit.to_owned(),
            label_key: label.to_owned(),
            label_arg: None,
            category: kind.to_owned(),
        };
        // As SchemaBuilder.cs names them (Task 10 adds the flag with this id).
        let schema = WireSchema {
            service: Default::default(),
            devices: vec![WireDevice {
                id: "svc-nvme".to_owned(),
                kind: "storage".to_owned(),
                name: "Fanxiang S880 2TB".to_owned(),
                vendor: None,
                properties: BTreeMap::new(),
                hint: Some(IdentityHint::Storage {
                    physical_drive: 2,
                    model: Some("Fanxiang S880 2TB".to_owned()),
                    serial: Some("SN2".to_owned()),
                }),
            }],
            sensors: vec![
                wire(
                    "flag",
                    "critical-warning",
                    "boolean",
                    "storage.criticalWarning",
                ),
                wire("percent", "wear", "percent", "storage.percentUsed"),
                wire(
                    "percent",
                    "available-spare",
                    "percent",
                    "storage.availableSpare",
                ),
            ],
        };
        let (service, _) = bind(&schema, &drives, &SourceRequest::default()).expect("bind");
        let service: Vec<(String, SensorKind, Unit, Label)> = service
            .sensors
            .into_iter()
            .map(|s| (s.id, s.kind, s.unit, s.label))
            .collect();
        let core: Vec<(String, SensorKind, Unit, Label)> = health_sensors(core_id, NVME)
            .into_iter()
            .map(|s| (s.id, s.kind, s.unit, s.label))
            .collect();
        assert_eq!(
            core, service,
            "the engine keeps the core's and drops the service's"
        );
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn reads_nvme_health_log_per_drive() {
        let mut nvme = 0;
        for index in 0..16 {
            let Some(drive) = PhysicalDrive::open(index) else {
                continue;
            };
            let bus = drive
                .query_property(StorageDeviceProperty, 65_536)
                .as_deref()
                .and_then(parse_bus_type);
            if bus != NVME {
                println!("disk {index}: bus {bus:?}, not queried");
                continue;
            }
            assert!(may_query(drive.powered_on()));
            let started = Instant::now();
            let health = query_health_log(&drive);
            println!("disk {index}: {:?}, {health:?}", started.elapsed());
            let health = health.expect("NVMe health log without admin");
            assert!(health.available_spare <= 100 && health.spare_threshold <= 100);
            nvme += 1;
        }
        assert_eq!(nvme, 2, "the two NVMe disks of this machine");
    }
}
