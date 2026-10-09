//! Batteries, read from the battery class driver (`IOCTL_BATTERY_*`) without
//! privileges. Each battery is `battery/<n>` in enumeration order. The
//! estimated time is re-read every 10 s and the static data every 60 s; a
//! different tag or different static data means the battery changed, so the
//! provider asks for a new discovery.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use windows::core::{GUID, HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    SP_DEVICE_INTERFACE_DATA,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;

use crate::storage_identity::{interface_detail, DeviceInfoSet};

/// `GUID_DEVICE_BATTERY`, the battery class device interface.
const GUID_DEVICE_BATTERY: GUID = GUID::from_u128(0x72631e54_78a4_11d0_bcf7_00aa00b7b32a);

const IOCTL_BATTERY_QUERY_TAG: u32 = 0x0029_4040;
const IOCTL_BATTERY_QUERY_INFORMATION: u32 = 0x0029_4044;
const IOCTL_BATTERY_QUERY_STATUS: u32 = 0x0029_404C;

const LEVEL_INFORMATION: u32 = 0;
const LEVEL_ESTIMATED_TIME: u32 = 3;
const LEVEL_DEVICE_NAME: u32 = 4;
const LEVEL_MANUFACTURE_NAME: u32 = 6;

const POWER_ON_LINE: u32 = 1;
const DISCHARGING: u32 = 2;
const CHARGING: u32 = 4;
const CAPACITY_RELATIVE: u32 = 0x4000_0000;

/// "Unknown" capacity, voltage and time; the rate's own marker is `i32::MIN`.
const UNKNOWN: u32 = 0xFFFF_FFFF;
const UNKNOWN_RATE: i32 = i32::MIN;

const TIME_EVERY: Duration = Duration::from_secs(10);
const STATIC_EVERY: Duration = Duration::from_secs(60);

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BatteryQueryInformation {
    tag: u32,
    level: u32,
    at_rate: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct BatteryInformation {
    capabilities: u32,
    technology: u8,
    reserved: [u8; 3],
    chemistry: [u8; 4],
    designed_capacity: u32,
    full_charged_capacity: u32,
    default_alert1: u32,
    default_alert2: u32,
    critical_bias: u32,
    cycle_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BatteryWaitStatus {
    tag: u32,
    timeout: u32,
    power_state: u32,
    low_capacity: u32,
    high_capacity: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct BatteryStatus {
    power_state: u32,
    capacity: u32,
    voltage: u32,
    rate: i32,
}

const _: () = assert!(size_of::<BatteryQueryInformation>() == 12);
const _: () = assert!(size_of::<BatteryInformation>() == 36);
const _: () = assert!(size_of::<BatteryWaitStatus>() == 20);
const _: () = assert!(size_of::<BatteryStatus>() == 16);
const _: () = assert!(std::mem::offset_of!(BatteryInformation, chemistry) == 8);
const _: () = assert!(std::mem::offset_of!(BatteryInformation, designed_capacity) == 12);
const _: () = assert!(std::mem::offset_of!(BatteryInformation, cycle_count) == 32);
const _: () = assert!(std::mem::offset_of!(BatteryStatus, rate) == 12);

fn is_relative(info: &BatteryInformation) -> bool {
    info.capabilities & CAPACITY_RELATIVE != 0
}

/// A capacity in mWh that is known and not zero.
fn known_capacity(value: u32) -> Option<u32> {
    (value != UNKNOWN && value != 0).then_some(value)
}

/// Charge in percent: the capacity itself when it is relative, otherwise
/// remaining over full-charge capacity.
fn charge_pct(status: &BatteryStatus, info: &BatteryInformation) -> Option<f64> {
    if status.capacity == UNKNOWN {
        return None;
    }
    let pct = if is_relative(info) {
        f64::from(status.capacity)
    } else {
        f64::from(status.capacity) / f64::from(known_capacity(info.full_charged_capacity)?) * 100.0
    };
    Some(pct.clamp(0.0, 100.0))
}

/// Power in watts, positive while charging and negative while discharging.
fn rate_w(status: &BatteryStatus, info: &BatteryInformation) -> Option<f64> {
    // With relative capacity the rate is in relative units, not mW.
    (!is_relative(info) && status.rate != UNKNOWN_RATE).then(|| f64::from(status.rate) / 1000.0)
}

fn voltage_v(status: &BatteryStatus) -> Option<f64> {
    (status.voltage != UNKNOWN).then(|| f64::from(status.voltage) / 1000.0)
}

/// Remaining energy in joules (1 mWh = 3.6 J); absent with relative capacity.
fn remaining_j(status: &BatteryStatus, info: &BatteryInformation) -> Option<f64> {
    if is_relative(info) || status.capacity == UNKNOWN {
        return None;
    }
    Some(f64::from(status.capacity) * 3.6)
}

/// Capacity lost since the design, in percent.
fn wear_pct(info: &BatteryInformation) -> Option<f64> {
    if is_relative(info) {
        return None;
    }
    let design = known_capacity(info.designed_capacity)?;
    let full = known_capacity(info.full_charged_capacity)?;
    Some((100.0 - f64::from(full) / f64::from(design) * 100.0).max(0.0))
}

/// Estimated seconds left, only while discharging.
fn estimated_s(raw: u32, discharging: bool) -> Option<f64> {
    (discharging && raw != UNKNOWN).then(|| f64::from(raw))
}

/// Number with up to three decimals, invariant (`"42.5"`, `"50"`).
fn invariant(mwh: u32) -> String {
    let text = format!("{:.3}", f64::from(mwh) / 1000.0);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn battery_properties(
    info: &BatteryInformation,
    name: Option<&str>,
    manufacturer: Option<&str>,
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if !is_relative(info) {
        if let Some(design) = known_capacity(info.designed_capacity) {
            map.insert("designCapacityWh".to_owned(), invariant(design));
        }
        if let Some(full) = known_capacity(info.full_charged_capacity) {
            map.insert("fullChargeCapacityWh".to_owned(), invariant(full));
        }
        if let Some(wear) = wear_pct(info) {
            map.insert(
                "wearPct".to_owned(),
                format!("{wear:.3}")
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_owned(),
            );
        }
    }
    if info.cycle_count != 0 && info.cycle_count != UNKNOWN {
        map.insert("cycleCount".to_owned(), info.cycle_count.to_string());
    }
    let chemistry: String = info
        .chemistry
        .iter()
        .filter(|b| b.is_ascii_graphic())
        .map(|&b| char::from(b))
        .collect();
    if !chemistry.is_empty() {
        map.insert("chemistry".to_owned(), chemistry);
    }
    if let Some(m) = manufacturer.filter(|m| !m.is_empty()) {
        map.insert("manufacturer".to_owned(), m.to_owned());
    }
    if let Some(n) = name.filter(|n| !n.is_empty()) {
        map.insert("model".to_owned(), n.to_owned());
    }
    map
}

/// Plain integer data (no padding, any bit pattern valid) that a driver may
/// fill through `DeviceIoControl`.
///
/// # Safety
/// Implement only for types with no padding and no invalid bit patterns.
unsafe trait Pod {}
// SAFETY: each type is made only of integers, with no padding.
unsafe impl Pod for u32 {}
unsafe impl Pod for [u16; 128] {}
unsafe impl Pod for BatteryInformation {}
unsafe impl Pod for BatteryStatus {}
unsafe impl Pod for BatteryQueryInformation {}
unsafe impl Pod for BatteryWaitStatus {}

/// An open battery device, closed on drop.
struct BatteryHandle(HANDLE);

// SAFETY: a kernel handle has no thread affinity. `BatteryHandle` owns it
// exclusively and closes it once, in `Drop`; it is Send but not Sync.
unsafe impl Send for BatteryHandle {}

impl Drop for BatteryHandle {
    fn drop(&mut self) {
        // SAFETY: sole owner of the handle, never used after drop.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl BatteryHandle {
    fn open(path: &str) -> Option<Self> {
        let path = HSTRING::from(path);
        // SAFETY: `path` is NUL-terminated and outlives the call. Read and
        // write access match Microsoft's battery sample; the three IOCTLs
        // used here need only read access and never change the battery.
        let handle = unsafe {
            CreateFileW(
                &path,
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .ok()?;
        Some(Self(handle))
    }

    /// Sends `code` and requires the driver to fill all of `output`.
    fn ioctl<I: Pod, O: Pod>(&self, code: u32, input: &I, output: &mut O) -> Option<()> {
        (self.ioctl_partial(code, input, output)? == size_of::<O>()).then_some(())
    }

    /// Sends `code`; the number of bytes the driver wrote, `None` on failure.
    fn ioctl_partial<I: Pod, O: Pod>(&self, code: u32, input: &I, output: &mut O) -> Option<usize> {
        let mut returned = 0u32;
        // SAFETY: the handle is open; both pointers cover exactly the sizes
        // passed, and `returned` is a valid out-pointer.
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                Some((input as *const I).cast()),
                size_of::<I>() as u32,
                Some((output as *mut O).cast()),
                size_of::<O>() as u32,
                Some(&mut returned),
                None,
            )
        }
        .ok()?;
        Some((returned as usize).min(size_of::<O>()))
    }

    fn tag(&self) -> Option<u32> {
        let mut tag = 0u32;
        self.ioctl(IOCTL_BATTERY_QUERY_TAG, &0u32, &mut tag)?;
        (tag != 0).then_some(tag)
    }

    fn query<O: Pod>(&self, tag: u32, level: u32, mut out: O, exact: bool) -> Option<(O, usize)> {
        let query = BatteryQueryInformation {
            tag,
            level,
            at_rate: 0,
        };
        let n = self.ioctl_partial(IOCTL_BATTERY_QUERY_INFORMATION, &query, &mut out)?;
        (!exact || n == size_of::<O>()).then_some((out, n))
    }

    fn info(&self, tag: u32) -> Option<BatteryInformation> {
        self.query(tag, LEVEL_INFORMATION, BatteryInformation::default(), true)
            .map(|(info, _)| info)
    }

    fn text(&self, tag: u32, level: u32) -> Option<String> {
        let (units, bytes): ([u16; 128], usize) = self.query(tag, level, [0u16; 128], false)?;
        let units = &units[..(bytes / 2).min(128)];
        let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
        let text = String::from_utf16_lossy(&units[..end]).trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    fn estimated_time(&self, tag: u32) -> u32 {
        self.query(tag, LEVEL_ESTIMATED_TIME, 0u32, true)
            .map_or(UNKNOWN, |(raw, _)| raw)
    }

    fn status(&self, tag: u32) -> Option<BatteryStatus> {
        let wait = BatteryWaitStatus {
            tag,
            ..Default::default()
        };
        let mut status = BatteryStatus::default();
        self.ioctl(IOCTL_BATTERY_QUERY_STATUS, &wait, &mut status)?;
        Some(status)
    }
}

/// Device paths of the present batteries, in enumeration order.
fn battery_paths() -> Vec<String> {
    // SAFETY: valid interface class GUID, no enumerator, no parent window.
    let Ok(set) = (unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVICE_BATTERY),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }) else {
        return Vec::new();
    };
    let set = DeviceInfoSet(set);
    let mut paths = Vec::new();
    for member in 0u32..64 {
        let mut interface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: valid set, class GUID and interface data with cbSize set.
        // Any error ends the enumeration, not only ERROR_NO_MORE_ITEMS.
        if unsafe {
            SetupDiEnumDeviceInterfaces(set.0, None, &GUID_DEVICE_BATTERY, member, &mut interface)
        }
        .is_err()
        {
            break;
        }
        if let Some((path, _)) = interface_detail(&set, &interface) {
            paths.push(path);
        }
    }
    paths
}

/// Whether the enumerated battery paths differ from the ones known at
/// discovery (which includes batteries that failed to open).
fn paths_changed(known: &[String], current: &[String]) -> bool {
    known != current
}

struct Battery {
    handle: BatteryHandle,
    tag: u32,
    info: BatteryInformation,
    name: Option<String>,
    manufacturer: Option<String>,
    time_raw: u32,
}

impl Battery {
    fn open(path: &str) -> Option<Self> {
        let handle = BatteryHandle::open(path)?;
        let tag = handle.tag()?;
        let info = handle.info(tag)?;
        Some(Self {
            name: handle.text(tag, LEVEL_DEVICE_NAME),
            manufacturer: handle.text(tag, LEVEL_MANUFACTURE_NAME),
            time_raw: handle.estimated_time(tag),
            handle,
            tag,
            info,
        })
    }
}

#[derive(Default)]
pub struct BatteryProvider {
    batteries: Vec<Battery>,
    /// Every enumerated path at discovery, opened or not.
    paths: Vec<String>,
    last_time: Option<Instant>,
    last_static: Option<Instant>,
}

impl BatteryProvider {
    /// Whether the static data, the tags or the set of batteries differ.
    fn static_changed(&self) -> bool {
        if paths_changed(&self.paths, &battery_paths()) {
            return true;
        }
        self.batteries.iter().any(|b| {
            b.handle.tag() != Some(b.tag) || b.handle.info(b.tag).is_none_or(|i| i != b.info)
        })
    }
}

impl Provider for BatteryProvider {
    fn name(&self) -> &'static str {
        "battery"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        self.paths = battery_paths();
        self.batteries = self
            .paths
            .iter()
            .map(String::as_str)
            .filter_map(Battery::open)
            .collect();
        let now = Instant::now();
        self.last_time = Some(now);
        self.last_static = Some(now);
        let mut inventory = Inventory::default();
        for (n, battery) in self.batteries.iter().enumerate() {
            let id = format!("battery/{n}");
            inventory.devices.push(Device {
                id: id.clone(),
                kind: DeviceKind::Battery,
                name: battery
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Battery {n}")),
                vendor: battery.manufacturer.clone(),
                properties: battery_properties(
                    &battery.info,
                    battery.name.as_deref(),
                    battery.manufacturer.as_deref(),
                ),
            });
            for (kind, name, unit, label) in [
                (
                    SensorKind::Percent,
                    "charge",
                    Unit::Percent,
                    "battery.charge",
                ),
                (SensorKind::Power, "rate", Unit::Watt, "battery.rate"),
                (
                    SensorKind::Voltage,
                    "voltage",
                    Unit::Volt,
                    "battery.voltage",
                ),
                (
                    SensorKind::Energy,
                    "remaining",
                    Unit::Joule,
                    "battery.remaining",
                ),
                (
                    SensorKind::Duration,
                    "remaining",
                    Unit::Seconds,
                    "battery.timeRemaining",
                ),
                (
                    SensorKind::Flag,
                    "charging",
                    Unit::Boolean,
                    "battery.charging",
                ),
                (
                    SensorKind::Flag,
                    "ac-online",
                    Unit::Boolean,
                    "battery.acOnline",
                ),
            ] {
                inventory.sensors.push(Sensor::new(
                    &id,
                    kind,
                    name,
                    unit,
                    Label::new(label),
                    Source::Win32,
                ));
            }
        }
        Ok(inventory)
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let now = Instant::now();
        if self
            .last_static
            .is_none_or(|t| now.duration_since(t) >= STATIC_EVERY)
        {
            self.last_static = Some(now);
            if self.static_changed() {
                return Err(ProviderError::Rediscover);
            }
        }
        let refresh_time = self
            .last_time
            .is_none_or(|t| now.duration_since(t) >= TIME_EVERY);
        if refresh_time {
            self.last_time = Some(now);
        }
        let mut values = Vec::with_capacity(self.batteries.len() * 7);
        for battery in &mut self.batteries {
            let Some(status) = battery.handle.status(battery.tag) else {
                return Err(ProviderError::Rediscover);
            };
            if refresh_time {
                battery.time_raw = battery.handle.estimated_time(battery.tag);
            }
            let flag = |bit: u32| Some(f64::from(u8::from(status.power_state & bit != 0)));
            values.extend([
                charge_pct(&status, &battery.info),
                rate_w(&status, &battery.info),
                voltage_v(&status),
                remaining_j(&status, &battery.info),
                estimated_s(battery.time_raw, status.power_state & DISCHARGING != 0),
                flag(CHARGING),
                flag(POWER_ON_LINE),
            ]);
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absolute(design: u32, full: u32) -> BatteryInformation {
        BatteryInformation {
            designed_capacity: design,
            full_charged_capacity: full,
            chemistry: *b"LION",
            ..Default::default()
        }
    }

    fn status(capacity: u32, rate: i32) -> BatteryStatus {
        BatteryStatus {
            power_state: 0,
            capacity,
            voltage: 12_000,
            rate,
        }
    }

    #[test]
    fn charge_from_absolute_capacity() {
        let pct = charge_pct(&status(35_000, 0), &absolute(50_000, 50_000));
        assert_eq!(pct, Some(70.0));
        assert_eq!(
            remaining_j(&status(35_000, 0), &absolute(50_000, 50_000)),
            Some(126_000.0)
        );
    }

    #[test]
    fn relative_capacity_reports_percent_only() {
        let info = BatteryInformation {
            capabilities: CAPACITY_RELATIVE,
            ..Default::default()
        };
        assert_eq!(charge_pct(&status(64, 0), &info), Some(64.0));
        assert_eq!(remaining_j(&status(64, 0), &info), None);
        assert_eq!(rate_w(&status(64, 500), &info), None);
        assert_eq!(wear_pct(&info), None);
    }

    #[test]
    fn rate_sign_follows_charging() {
        assert_eq!(rate_w(&status(1, -12_000), &absolute(1, 1)), Some(-12.0));
        assert_eq!(rate_w(&status(1, 5_000), &absolute(1, 1)), Some(5.0));
    }

    #[test]
    fn unknown_values_are_absent() {
        let unknown = BatteryStatus {
            power_state: 0,
            capacity: UNKNOWN,
            voltage: UNKNOWN,
            rate: UNKNOWN_RATE,
        };
        let info = absolute(50_000, 50_000);
        assert_eq!(charge_pct(&unknown, &info), None);
        assert_eq!(rate_w(&unknown, &info), None);
        assert_eq!(voltage_v(&unknown), None);
        assert_eq!(remaining_j(&unknown, &info), None);
        assert_eq!(estimated_s(UNKNOWN, true), None);
        assert_eq!(wear_pct(&absolute(UNKNOWN, UNKNOWN)), None);
    }

    #[test]
    fn zero_full_capacity_has_no_charge_or_wear() {
        let info = absolute(50_000, 0);
        assert_eq!(charge_pct(&status(35_000, 0), &info), None);
        assert_eq!(wear_pct(&info), None);
        assert_eq!(wear_pct(&absolute(0, 42_500)), None);
    }

    #[test]
    fn wear_from_design_and_full() {
        let wear = wear_pct(&absolute(50_000, 42_500)).unwrap();
        assert!((wear - 15.0).abs() < 1e-9);
        assert_eq!(wear_pct(&absolute(50_000, 55_000)), Some(0.0));
    }

    #[test]
    fn a_path_that_never_opens_is_no_change() {
        let known = vec!["a".to_owned(), "b".to_owned()];
        assert!(!paths_changed(&known, &known.clone()));
        assert!(paths_changed(&known, &known[..1]));
    }

    #[test]
    fn estimated_time_only_while_discharging() {
        assert_eq!(estimated_s(5_400, true), Some(5_400.0));
        assert_eq!(estimated_s(5_400, false), None);
    }

    #[test]
    fn properties_use_invariant_numbers() {
        let mut info = absolute(50_000, 42_500);
        info.cycle_count = 312;
        let map = battery_properties(&info, Some("DELL 1234"), Some("Dell"));
        assert_eq!(map["designCapacityWh"], "50");
        assert_eq!(map["fullChargeCapacityWh"], "42.5");
        assert_eq!(map["wearPct"], "15");
        assert_eq!(map["chemistry"], "LION");
        assert_eq!(map["cycleCount"], "312");
        assert_eq!(map["manufacturer"], "Dell");
        assert_eq!(map["model"], "DELL 1234");
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn battery_discovers_and_polls() {
        let mut provider = BatteryProvider::default();
        let inventory = provider.discover().expect("discover");
        assert_eq!(inventory.sensors.len(), inventory.devices.len() * 7);
        let values = provider.poll().expect("poll");
        assert_eq!(values.len(), inventory.sensors.len());
    }
}
