//! Intel GPU telemetry from the Intel Graphics Control Library (`ControlLib.dll`, installed by
//! the Intel graphics driver).
//!
//! Interface declarations for interoperability with Intel's ControlLib.dll, written from the
//! library's public ABI (exported symbol names, struct layouts, constant values); no Intel
//! header is included, and identifiers and comments are our own. Every size and offset below
//! was measured with clang (`x86_64-pc-windows-msvc`) against IGCL header v298 and is pinned
//! with compile-time assertions.
//!
//! Not verified on real hardware (the development machine has no Intel GPU): the unit tests
//! drive the layer through fake entry points.
//!
//! The DLL is loaded from System32 only and kept for the process lifetime; `ctlClose` is never
//! called (decision D1). C `bool` fields are `u8`: a byte other than 0/1 in a Rust `bool`
//! written by foreign code would be undefined behaviour.
//!
//! x86_64: `CTL_APICALL` (cdecl) is the C convention there.

use std::collections::BTreeSet;
use std::ffi::c_void;
use std::mem::{align_of, offset_of, size_of};
use std::ptr::null_mut;

use oma_core::model::Source;
use oma_core::provider::ProviderError;

use super::adapter::{Adapter, PciAddress, Vendor};
use super::field::GpuField;
use super::layer::{GpuLayer, Readings};
use crate::dynlib::Library;

type CtlResult = u32;
type ApiHandle = *mut c_void;
type DeviceHandle = *mut c_void;

const SUCCESS: CtlResult = 0;
const ERROR_DEVICE_LOST: CtlResult = 0x4000_0003;
const ERROR_UNSUPPORTED_VERSION: CtlResult = 0x4000_0009;
const ERROR_INVALID_ARGUMENT: CtlResult = 0x4000_000B;
const ERROR_INVALID_SIZE: CtlResult = 0x4000_000F;
const ERROR_UNSUPPORTED_SIZE: CtlResult = 0x4000_0010;
/// The device is in a low-power state or recovering from a TDR: transient.
const ERROR_DEVICE_UNAVAILABLE: CtlResult = 0x4000_0027;

const fn api_version(major: u32, minor: u32) -> u32 {
    (major << 16) | (minor & 0xFFFF)
}
/// API version requested first; runtimes that only know 1.0 reject it.
const API_VERSION_1_1: u32 = api_version(1, 1);
const API_VERSION_1_0: u32 = api_version(1, 0);
/// Init flag bit 0: use Level Zero (needed for telemetry).
const INIT_USE_LEVEL_ZERO: u32 = 1;
/// Properties record version that also fills the PCI subsystem ids and bus/device/function.
const PROPERTIES_VERSION: u8 = 2;
/// Telemetry record: current layout (1024 bytes, version 1) and the older one (808, version 0).
const TELEMETRY_V1: (u32, u8) = (1024, 1);
const TELEMETRY_V0: (u32, u8) = (808, 0);

const NAME_LEN: usize = 100;
const RESERVED_LEN: usize = 108;
const PSU_COUNT: usize = 5;
const FAN_COUNT: usize = 5;

// Tags of a telemetry value.
const TYPE_I8: u32 = 0;
const TYPE_U8: u32 = 1;
const TYPE_I16: u32 = 2;
const TYPE_U16: u32 = 3;
const TYPE_I32: u32 = 4;
const TYPE_U32: u32 = 5;
const TYPE_I64: u32 = 6;
const TYPE_U64: u32 = 7;
const TYPE_F32: u32 = 8;
const TYPE_F64: u32 = 9;
/// Unit tag for millivolts (volts are the default for the voltage items).
const UNITS_MILLIVOLTS: u32 = 13;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ApplicationId {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InitArgs {
    size: u32,
    version: u8,
    app_version: u32,
    flags: u32,
    supported_version: u32,
    application_id: ApplicationId,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FirmwareVersion {
    major: u64,
    minor: u64,
    build: u64,
}

/// PCI bus/device/function (no domain).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct AdapterBdf {
    bus: u8,
    device: u8,
    function: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DeviceProperties {
    size: u32,
    version: u8,
    /// Caller-owned buffer; on Windows the library writes the adapter LUID (8 bytes).
    device_id: *mut c_void,
    device_id_size: u32,
    device_type: u32,
    subfunction_flags: u32,
    driver_version: u64,
    firmware_version: FirmwareVersion,
    pci_vendor_id: u32,
    pci_device_id: u32,
    revision: u32,
    eus_per_sub_slice: u32,
    sub_slices_per_slice: u32,
    slices: u32,
    name: [u8; NAME_LEN],
    adapter_flags: u32,
    frequency: u32,
    pci_subsys_id: u16,
    pci_subsys_vendor_id: u16,
    bdf: AdapterBdf,
    xe_cores: u32,
    reserved: [u8; RESERVED_LEN],
}

/// A telemetry value; `TelemetryItem::data_type` says which member is valid.
#[repr(C)]
#[derive(Clone, Copy)]
union DataValue {
    i8: i8,
    u8: u8,
    i16: i16,
    u16: u16,
    i32: i32,
    u32: u32,
    i64: i64,
    u64: u64,
    f32: f32,
    f64: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TelemetryItem {
    supported: u8,
    units: u32,
    data_type: u32,
    value: DataValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PsuInfo {
    supported: u8,
    psu_type: u32,
    energy_counter: TelemetryItem,
    voltage: TelemetryItem,
}

/// Power/thermal telemetry of one device. Counters are cumulative (energy in joules,
/// timestamp in seconds): rates come from the difference of two reads.
#[repr(C)]
#[derive(Clone, Copy)]
struct PowerTelemetry {
    size: u32,
    version: u8,
    time_stamp: TelemetryItem,
    gpu_energy_counter: TelemetryItem,
    gpu_voltage: TelemetryItem,
    gpu_clock: TelemetryItem,
    gpu_temperature: TelemetryItem,
    global_activity_counter: TelemetryItem,
    render_compute_activity_counter: TelemetryItem,
    media_activity_counter: TelemetryItem,
    gpu_power_limited: u8,
    gpu_temperature_limited: u8,
    gpu_current_limited: u8,
    gpu_voltage_limited: u8,
    gpu_utilization_limited: u8,
    vram_energy_counter: TelemetryItem,
    vram_voltage: TelemetryItem,
    vram_clock: TelemetryItem,
    vram_effective_clock: TelemetryItem,
    vram_read_bandwidth_counter: TelemetryItem,
    vram_write_bandwidth_counter: TelemetryItem,
    vram_temperature: TelemetryItem,
    vram_power_limited: u8,
    vram_temperature_limited: u8,
    vram_current_limited: u8,
    vram_voltage_limited: u8,
    vram_utilization_limited: u8,
    total_card_energy_counter: TelemetryItem,
    psu: [PsuInfo; PSU_COUNT],
    fan_speed: [TelemetryItem; FAN_COUNT],
    // Only filled with version >= 1 (the 808-byte layout ends here).
    gpu_vr_temperature: TelemetryItem,
    vram_vr_temperature: TelemetryItem,
    sa_vr_temperature: TelemetryItem,
    gpu_effective_clock: TelemetryItem,
    gpu_over_voltage_percent: TelemetryItem,
    gpu_power_percent: TelemetryItem,
    gpu_temperature_percent: TelemetryItem,
    vram_read_bandwidth: TelemetryItem,
    vram_write_bandwidth: TelemetryItem,
}

macro_rules! pin {
    ($t:ty, $size:expr, $align:expr) => {
        const _: () = assert!(size_of::<$t>() == $size && align_of::<$t>() == $align);
    };
    ($t:ty, $field:ident @ $offset:expr) => {
        const _: () = assert!(offset_of!($t, $field) == $offset);
    };
}

pin!(ApplicationId, 16, 4);
pin!(InitArgs, 36, 4);
pin!(InitArgs, version @ 4);
pin!(InitArgs, app_version @ 8);
pin!(InitArgs, flags @ 12);
pin!(InitArgs, supported_version @ 16);
pin!(InitArgs, application_id @ 20);
pin!(FirmwareVersion, 24, 8);
pin!(AdapterBdf, 3, 1);
pin!(DeviceProperties, 320, 8);
pin!(DeviceProperties, device_id @ 8);
pin!(DeviceProperties, device_id_size @ 16);
pin!(DeviceProperties, device_type @ 20);
pin!(DeviceProperties, subfunction_flags @ 24);
pin!(DeviceProperties, driver_version @ 32);
pin!(DeviceProperties, firmware_version @ 40);
pin!(DeviceProperties, pci_vendor_id @ 64);
pin!(DeviceProperties, pci_device_id @ 68);
pin!(DeviceProperties, revision @ 72);
pin!(DeviceProperties, eus_per_sub_slice @ 76);
pin!(DeviceProperties, sub_slices_per_slice @ 80);
pin!(DeviceProperties, slices @ 84);
pin!(DeviceProperties, name @ 88);
pin!(DeviceProperties, adapter_flags @ 188);
pin!(DeviceProperties, frequency @ 192);
pin!(DeviceProperties, pci_subsys_id @ 196);
pin!(DeviceProperties, pci_subsys_vendor_id @ 198);
pin!(DeviceProperties, bdf @ 200);
pin!(DeviceProperties, xe_cores @ 204);
pin!(DeviceProperties, reserved @ 208);
pin!(DataValue, 8, 8);
pin!(TelemetryItem, 24, 8);
pin!(TelemetryItem, units @ 4);
pin!(TelemetryItem, data_type @ 8);
pin!(TelemetryItem, value @ 16);
pin!(PsuInfo, 56, 8);
pin!(PsuInfo, psu_type @ 4);
pin!(PsuInfo, energy_counter @ 8);
pin!(PsuInfo, voltage @ 32);
pin!(PowerTelemetry, 1024, 8);
pin!(PowerTelemetry, version @ 4);
pin!(PowerTelemetry, time_stamp @ 8);
pin!(PowerTelemetry, gpu_energy_counter @ 32);
pin!(PowerTelemetry, gpu_voltage @ 56);
pin!(PowerTelemetry, gpu_clock @ 80);
pin!(PowerTelemetry, gpu_temperature @ 104);
pin!(PowerTelemetry, global_activity_counter @ 128);
pin!(PowerTelemetry, render_compute_activity_counter @ 152);
pin!(PowerTelemetry, media_activity_counter @ 176);
pin!(PowerTelemetry, gpu_power_limited @ 200);
pin!(PowerTelemetry, gpu_temperature_limited @ 201);
pin!(PowerTelemetry, gpu_current_limited @ 202);
pin!(PowerTelemetry, gpu_voltage_limited @ 203);
pin!(PowerTelemetry, gpu_utilization_limited @ 204);
pin!(PowerTelemetry, vram_energy_counter @ 208);
pin!(PowerTelemetry, vram_voltage @ 232);
pin!(PowerTelemetry, vram_clock @ 256);
pin!(PowerTelemetry, vram_effective_clock @ 280);
pin!(PowerTelemetry, vram_read_bandwidth_counter @ 304);
pin!(PowerTelemetry, vram_write_bandwidth_counter @ 328);
pin!(PowerTelemetry, vram_temperature @ 352);
pin!(PowerTelemetry, vram_power_limited @ 376);
pin!(PowerTelemetry, vram_utilization_limited @ 380);
pin!(PowerTelemetry, total_card_energy_counter @ 384);
pin!(PowerTelemetry, psu @ 408);
pin!(PowerTelemetry, fan_speed @ 688);
pin!(PowerTelemetry, gpu_vr_temperature @ 808);
pin!(PowerTelemetry, vram_vr_temperature @ 832);
pin!(PowerTelemetry, sa_vr_temperature @ 856);
pin!(PowerTelemetry, gpu_effective_clock @ 880);
pin!(PowerTelemetry, gpu_over_voltage_percent @ 904);
pin!(PowerTelemetry, gpu_power_percent @ 928);
pin!(PowerTelemetry, gpu_temperature_percent @ 952);
pin!(PowerTelemetry, vram_read_bandwidth @ 976);
pin!(PowerTelemetry, vram_write_bandwidth @ 1000);
const _: () = assert!(offset_of!(PowerTelemetry, gpu_vr_temperature) == TELEMETRY_V0.0 as usize);
const _: () = assert!(size_of::<PowerTelemetry>() == TELEMETRY_V1.0 as usize);

impl InitArgs {
    fn new(app_version: u32) -> Self {
        Self {
            size: size_of::<Self>() as u32,
            version: 0,
            app_version,
            flags: INIT_USE_LEVEL_ZERO,
            ..Self::default()
        }
    }
}

impl DeviceProperties {
    /// Zeroed record with size/version set and the LUID buffer attached.
    fn new(luid: &mut [u8; 8]) -> Self {
        // SAFETY: integers, byte arrays and a raw pointer only; all-zero is valid for each.
        let mut properties: Self = unsafe { std::mem::zeroed() };
        properties.size = size_of::<Self>() as u32;
        properties.version = PROPERTIES_VERSION;
        properties.device_id = luid.as_mut_ptr().cast();
        properties.device_id_size = 8;
        properties
    }
}

impl TelemetryItem {
    /// The value as f64 when supported, of a known numeric type and finite.
    fn value(&self) -> Option<f64> {
        if self.supported == 0 {
            return None;
        }
        // SAFETY: every union member is plain data valid for any bit pattern, and the member
        // read is the one the item's own tag selects.
        let v = unsafe {
            match self.data_type {
                TYPE_I8 => f64::from(self.value.i8),
                TYPE_U8 => f64::from(self.value.u8),
                TYPE_I16 => f64::from(self.value.i16),
                TYPE_U16 => f64::from(self.value.u16),
                TYPE_I32 => f64::from(self.value.i32),
                TYPE_U32 => f64::from(self.value.u32),
                TYPE_I64 => self.value.i64 as f64,
                TYPE_U64 => self.value.u64 as f64,
                TYPE_F32 => f64::from(self.value.f32),
                TYPE_F64 => self.value.f64,
                _ => return None,
            }
        };
        v.is_finite().then_some(v)
    }

    fn volts(&self) -> Option<f64> {
        let v = self.value()?;
        Some(if self.units == UNITS_MILLIVOLTS {
            v / 1000.0
        } else {
            v
        })
    }

    fn is_supported(&self) -> bool {
        self.supported != 0
    }
}

impl PowerTelemetry {
    /// Zeroed record announcing `layout` (size, version); the buffer is always 1024 bytes.
    fn new(layout: (u32, u8)) -> Self {
        // SAFETY: integers, `u8` flags and plain-data unions only; all-zero is valid.
        let mut telemetry: Self = unsafe { std::mem::zeroed() };
        (telemetry.size, telemetry.version) = layout;
        telemetry
    }
}

/// Counter used for board power, chosen at attach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnergyCounter {
    TotalCard,
    Gpu,
}

/// (timestamp in seconds, energy in joules) of one read.
#[derive(Debug, Clone, Copy, PartialEq)]
struct EnergySample {
    time: f64,
    joules: f64,
}

fn energy_sample(t: &PowerTelemetry, counter: EnergyCounter) -> Option<EnergySample> {
    let item = match counter {
        EnergyCounter::TotalCard => &t.total_card_energy_counter,
        EnergyCounter::Gpu => &t.gpu_energy_counter,
    };
    Some(EnergySample {
        time: t.time_stamp.value()?,
        joules: item.value()?,
    })
}

/// Turns successive energy reads into watts.
#[derive(Debug, Default)]
struct EnergyMeter {
    last: Option<EnergySample>,
}

impl EnergyMeter {
    /// Average power since the previous read. None for the first read, when the library
    /// returned its cached value (no time elapsed: it caches for 50 ms) or when the counter
    /// went backwards (reset or wrap); the latter restarts from the new read.
    fn update(&mut self, current: Option<EnergySample>) -> Option<f64> {
        let Some(current) = current else {
            self.last = None;
            return None;
        };
        let Some(last) = self.last else {
            self.last = Some(current);
            return None;
        };
        let dt = current.time - last.time;
        if dt <= 0.0 {
            return None;
        }
        self.last = Some(current);
        let joules = current.joules - last.joules;
        (joules >= 0.0).then(|| joules / dt)
    }
}

/// Fields a device supports according to one telemetry read, and the energy counter to use
/// for board power (the whole card if available, else the GPU alone).
fn supported_fields(t: &PowerTelemetry) -> (BTreeSet<GpuField>, Option<EnergyCounter>) {
    let mut fields = BTreeSet::new();
    let items = [
        (GpuField::TemperatureCore, &t.gpu_temperature),
        (GpuField::TemperatureMemory, &t.vram_temperature),
        (GpuField::ClockCore, &t.gpu_clock),
        (GpuField::ClockMemory, &t.vram_clock),
        (GpuField::VoltageCore, &t.gpu_voltage),
        (GpuField::FanRpm, &t.fan_speed[0]),
    ];
    for (field, item) in items {
        if item.is_supported() {
            fields.insert(field);
        }
    }
    // The limit flags have no "supported" bit: present whenever telemetry works.
    fields.insert(GpuField::ThrottlePower);
    fields.insert(GpuField::ThrottleThermal);
    let energy = if !t.time_stamp.is_supported() {
        None
    } else if t.total_card_energy_counter.is_supported() {
        Some(EnergyCounter::TotalCard)
    } else if t.gpu_energy_counter.is_supported() {
        Some(EnergyCounter::Gpu)
    } else {
        None
    };
    if energy.is_some() {
        fields.insert(GpuField::PowerBoard);
    }
    (fields, energy)
}

/// Instant values of one read (board power comes from `EnergyMeter`).
fn readings_from(t: &PowerTelemetry, fields: &BTreeSet<GpuField>) -> Readings {
    let positive = |v: f64| (v > 0.0).then_some(v);
    let flag = |b: u8| if b != 0 { 1.0 } else { 0.0 };
    let mut readings = Readings::new();
    for &field in fields {
        let value = match field {
            GpuField::TemperatureCore => t.gpu_temperature.value().and_then(positive),
            GpuField::TemperatureMemory => t.vram_temperature.value().and_then(positive),
            GpuField::ClockCore => t.gpu_clock.value(),
            GpuField::ClockMemory => t.vram_clock.value(),
            GpuField::VoltageCore => t.gpu_voltage.volts(),
            GpuField::FanRpm => t.fan_speed[0].value(),
            GpuField::ThrottlePower => Some(flag(t.gpu_power_limited)),
            GpuField::ThrottleThermal => Some(flag(t.gpu_temperature_limited)),
            _ => None,
        };
        if let Some(value) = value {
            readings.insert(field, value);
        }
    }
    readings
}

/// The adapter a device belongs to: by LUID, else (LUID unknown) by PCI bus/device/function.
fn match_adapter(adapters: &[Adapter], luid: u64, bdf: AdapterBdf) -> Option<usize> {
    if luid != 0 {
        if let Some(i) = adapters.iter().position(|a| a.luid == luid) {
            return Some(i);
        }
    }
    let pci = PciAddress {
        bus: bdf.bus.into(),
        device: bdf.device.into(),
        function: bdf.function.into(),
    };
    adapters
        .iter()
        .position(|a| a.vendor() == Some(Vendor::Intel) && a.pci == Some(pci))
}

/// Size/version rejections that mean "runtime older than the 1024-byte layout".
fn is_layout_rejection(rc: CtlResult) -> bool {
    matches!(
        rc,
        ERROR_INVALID_SIZE
            | ERROR_UNSUPPORTED_SIZE
            | ERROR_UNSUPPORTED_VERSION
            | ERROR_INVALID_ARGUMENT
    )
}

type InitFn = unsafe extern "C" fn(*mut InitArgs, *mut ApiHandle) -> CtlResult;
type EnumerateFn = unsafe extern "C" fn(ApiHandle, *mut u32, *mut DeviceHandle) -> CtlResult;
type PropertiesFn = unsafe extern "C" fn(DeviceHandle, *mut DeviceProperties) -> CtlResult;
type TelemetryFn = unsafe extern "C" fn(DeviceHandle, *mut PowerTelemetry) -> CtlResult;

#[derive(Clone, Copy)]
struct Api {
    init: InitFn,
    enumerate: EnumerateFn,
    properties: PropertiesFn,
    telemetry: TelemetryFn,
}

/// ctlInit, asking for API 1.1 and retrying once with 1.0 when the runtime rejects the
/// version. Any other failure (no Intel adapter, unsupported platform...) means "absent".
fn init(api: &Api) -> Option<ApiHandle> {
    for version in [API_VERSION_1_1, API_VERSION_1_0] {
        let mut args = InitArgs::new(version);
        let mut handle: ApiHandle = null_mut();
        // SAFETY: `args` is a correctly sized init record and `handle` a valid out-pointer.
        let rc = unsafe { (api.init)(&mut args, &mut handle) };
        if rc == SUCCESS && !handle.is_null() {
            return Some(handle);
        }
        if rc != ERROR_UNSUPPORTED_VERSION {
            tracing::debug!(rc = format_args!("{rc:#x}"), "IGCL not usable");
            return None;
        }
    }
    tracing::debug!("IGCL runtime supports neither API 1.1 nor 1.0");
    None
}

/// One telemetry read with the device's known layout; on a size/version rejection of the
/// 1024-byte layout, one retry with the 808-byte one, which is then kept.
fn read_telemetry(
    api: &Api,
    device: DeviceHandle,
    layout: &mut (u32, u8),
) -> Result<PowerTelemetry, CtlResult> {
    let mut t = PowerTelemetry::new(*layout);
    // SAFETY: `device` came from ctlEnumerateDevices on a live session; `t` is a 1024-byte
    // record whose size field never exceeds the buffer.
    let rc = unsafe { (api.telemetry)(device, &mut t) };
    if rc == SUCCESS {
        return Ok(t);
    }
    if *layout != TELEMETRY_V1 || !is_layout_rejection(rc) {
        return Err(rc);
    }
    let mut t = PowerTelemetry::new(TELEMETRY_V0);
    // SAFETY: as above.
    let rc = unsafe { (api.telemetry)(device, &mut t) };
    if rc != SUCCESS {
        return Err(rc);
    }
    *layout = TELEMETRY_V0;
    Ok(t)
}

struct Bound {
    device: DeviceHandle,
    layout: (u32, u8),
    fields: BTreeSet<GpuField>,
    energy: Option<EnergyCounter>,
    meter: EnergyMeter,
    warned: bool,
    /// The attach probe found the device transiently unavailable: no fields declared yet.
    /// `sample` keeps re-probing and asks for a rediscover once it answers again.
    pending: bool,
}

pub(crate) struct IgclLayer {
    /// Keeps `ControlLib.dll` loaded (never freed, D1); None when driven by test fakes.
    _library: Option<Library>,
    api: Api,
    handle: ApiHandle,
    bound: Vec<Option<Bound>>,
}

// SAFETY: IGCL handles are process-wide objects, not tied to the creating thread, and the
// layer is used by one thread at a time (the GPU worker owns it).
unsafe impl Send for IgclLayer {}

impl IgclLayer {
    /// Loads `ControlLib.dll` from System32 and initialises IGCL. None when the DLL is absent
    /// (no Intel driver), an export is missing or ctlInit reports the library unusable.
    pub(crate) fn load() -> Option<Self> {
        let library = match Library::system32("ControlLib.dll") {
            Ok(library) => library,
            Err(e) => {
                tracing::debug!(error = %e, "IGCL runtime not available");
                return None;
            }
        };
        // SAFETY: each requested type is the exact prototype of the named export.
        let api = unsafe {
            Api {
                init: library.symbol(c"ctlInit")?,
                enumerate: library.symbol(c"ctlEnumerateDevices")?,
                properties: library.symbol(c"ctlGetDeviceProperties")?,
                telemetry: library.symbol(c"ctlPowerTelemetryGet")?,
            }
        };
        Self::start(api, Some(library))
    }

    fn start(api: Api, library: Option<Library>) -> Option<Self> {
        let handle = init(&api)?;
        Some(Self {
            _library: library,
            api,
            handle,
            bound: Vec::new(),
        })
    }

    fn devices(&self) -> Vec<DeviceHandle> {
        let mut count = 0u32;
        // SAFETY: live session handle; a null array asks for the count only.
        let rc = unsafe { (self.api.enumerate)(self.handle, &mut count, null_mut()) };
        if rc != SUCCESS || count == 0 {
            return Vec::new();
        }
        let mut devices = vec![null_mut(); count as usize];
        // SAFETY: the array has room for `count` handles.
        let rc = unsafe { (self.api.enumerate)(self.handle, &mut count, devices.as_mut_ptr()) };
        if rc != SUCCESS {
            return Vec::new();
        }
        devices.truncate(count as usize);
        devices.retain(|d| !d.is_null());
        devices
    }

    /// (LUID, bus/device/function) of a device.
    fn identity(&self, device: DeviceHandle) -> Option<(u64, AdapterBdf)> {
        let mut luid = [0u8; 8];
        let mut properties = DeviceProperties::new(&mut luid);
        // SAFETY: `device` is live; the record is sized and its LUID buffer outlives the call.
        let rc = unsafe { (self.api.properties)(device, &mut properties) };
        // The LUID is little-endian LowPart:u32 then HighPart:i32, the same packing as
        // `Adapter::luid`.
        (rc == SUCCESS).then(|| (u64::from_le_bytes(luid), properties.bdf))
    }
}

impl GpuLayer for IgclLayer {
    fn source(&self) -> Source {
        Source::Igcl
    }

    fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>> {
        let mut bound: Vec<Option<Bound>> = adapters.iter().map(|_| None).collect();
        for device in self.devices() {
            let Some((luid, bdf)) = self.identity(device) else {
                continue;
            };
            let Some(i) = match_adapter(adapters, luid, bdf) else {
                continue;
            };
            if bound[i].is_some() {
                continue;
            }
            let mut layout = TELEMETRY_V1;
            match read_telemetry(&self.api, device, &mut layout) {
                Ok(t) => {
                    let (fields, energy) = supported_fields(&t);
                    bound[i] = Some(Bound {
                        device,
                        layout,
                        fields,
                        energy,
                        meter: EnergyMeter::default(),
                        warned: false,
                        pending: false,
                    });
                }
                // Low-power state or TDR recovery: bind it with no fields yet; `sample` will
                // ask for a rediscover once the device answers again.
                Err(ERROR_DEVICE_UNAVAILABLE) => {
                    bound[i] = Some(Bound {
                        device,
                        layout,
                        fields: BTreeSet::new(),
                        energy: None,
                        meter: EnergyMeter::default(),
                        warned: false,
                        pending: true,
                    });
                }
                Err(_) => continue,
            }
        }
        let supported = bound
            .iter()
            .map(|b| b.as_ref().map(|b| b.fields.clone()).unwrap_or_default())
            .collect();
        self.bound = bound;
        supported
    }

    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
        let api = self.api;
        let mut all = Vec::with_capacity(self.bound.len());
        for slot in &mut self.bound {
            let Some(bound) = slot else {
                all.push(Readings::new());
                continue;
            };
            match read_telemetry(&api, bound.device, &mut bound.layout) {
                // A device bound as "pending" (unavailable at attach) answering now means its
                // fields were never declared: ask for a rediscover so the next attach picks
                // them up, instead of silently reporting values `attach` never advertised.
                Ok(_) if bound.pending => return Err(ProviderError::Rediscover),
                Ok(t) => {
                    bound.warned = false;
                    let mut readings = readings_from(&t, &bound.fields);
                    if let Some(counter) = bound.energy {
                        if let Some(watts) = bound.meter.update(energy_sample(&t, counter)) {
                            readings.insert(GpuField::PowerBoard, watts);
                        }
                    }
                    all.push(readings);
                }
                // Low-power state or TDR recovery: nothing to read this tick.
                Err(ERROR_DEVICE_UNAVAILABLE) => all.push(Readings::new()),
                // The device handle is stale (driver restart): enumerate again.
                Err(ERROR_DEVICE_LOST) => return Err(ProviderError::Rediscover),
                Err(rc) => {
                    if !bound.warned {
                        tracing::warn!(rc = format_args!("{rc:#x}"), "IGCL telemetry read failed");
                        bound.warned = true;
                    }
                    all.push(Readings::new());
                }
            }
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // ---- telemetry builders

    fn item(value: f64) -> TelemetryItem {
        TelemetryItem {
            supported: 1,
            units: 0,
            data_type: TYPE_F64,
            value: DataValue { f64: value },
        }
    }

    fn telemetry(fill: impl FnOnce(&mut PowerTelemetry)) -> PowerTelemetry {
        let mut t = PowerTelemetry::new(TELEMETRY_V1);
        fill(&mut t);
        t
    }

    /// A discrete card at `time` seconds having used `joules` in total.
    fn card(time: f64, joules: f64) -> PowerTelemetry {
        telemetry(|t| {
            t.time_stamp = item(time);
            t.total_card_energy_counter = item(joules);
            t.gpu_energy_counter = item(joules / 2.0);
            t.gpu_temperature = item(61.0);
            t.vram_temperature = item(70.0);
            t.gpu_clock = item(2400.0);
            t.vram_clock = item(1093.0);
            t.gpu_voltage = item(0.95);
            t.fan_speed[0] = item(1500.0);
            t.gpu_power_limited = 1;
        })
    }

    fn adapter(luid: u64, vendor_id: u32, bus: u32) -> Adapter {
        Adapter {
            luid,
            name: format!("GPU {luid}"),
            vendor_id,
            device_id: 0x56A0,
            subsys_id: 0,
            pci: Some(PciAddress {
                bus,
                device: 0,
                function: 0,
            }),
            integrated: false,
            dedicated_bytes: 0,
        }
    }

    // ---- fake ControlLib: per-thread state driven by the test

    struct FakeDevice {
        luid: u64,
        bdf: AdapterBdf,
        /// Code returned to any 1024-byte (version 1) request.
        reject_v1: Option<CtlResult>,
        /// Replies to telemetry reads, in order; the last one repeats.
        replies: Vec<Result<PowerTelemetry, CtlResult>>,
    }

    #[derive(Default)]
    struct Fake {
        init_replies: Vec<CtlResult>,
        init_versions: Vec<u32>,
        devices: Vec<FakeDevice>,
        /// (size, version) announced by every telemetry request.
        telemetry_calls: Vec<(u32, u8)>,
    }

    thread_local! {
        static FAKE: RefCell<Fake> = RefCell::new(Fake::default());
    }

    fn device_index(device: DeviceHandle) -> usize {
        device as usize - 1
    }

    unsafe extern "C" fn fake_init(args: *mut InitArgs, handle: *mut ApiHandle) -> CtlResult {
        FAKE.with_borrow_mut(|f| {
            // SAFETY: the layer passes valid pointers.
            unsafe { f.init_versions.push((*args).app_version) };
            let rc = if f.init_replies.is_empty() {
                SUCCESS
            } else {
                f.init_replies.remove(0)
            };
            if rc == SUCCESS {
                // SAFETY: as above.
                unsafe { *handle = 0x1 as ApiHandle };
            }
            rc
        })
    }

    unsafe extern "C" fn fake_enumerate(
        _api: ApiHandle,
        count: *mut u32,
        out: *mut DeviceHandle,
    ) -> CtlResult {
        FAKE.with_borrow(|f| {
            // SAFETY: the layer passes a valid count and, when non-null, room for `count`.
            unsafe {
                if !out.is_null() {
                    for i in 0..(*count as usize).min(f.devices.len()) {
                        *out.add(i) = (i + 1) as DeviceHandle;
                    }
                }
                *count = f.devices.len() as u32;
            }
            SUCCESS
        })
    }

    unsafe extern "C" fn fake_properties(
        device: DeviceHandle,
        out: *mut DeviceProperties,
    ) -> CtlResult {
        FAKE.with_borrow(|f| {
            let d = &f.devices[device_index(device)];
            // SAFETY: the layer passes a sized record with an 8-byte LUID buffer.
            unsafe {
                (*out)
                    .device_id
                    .cast::<[u8; 8]>()
                    .write(d.luid.to_le_bytes());
                (*out).bdf = d.bdf;
            }
            SUCCESS
        })
    }

    unsafe extern "C" fn fake_telemetry(
        device: DeviceHandle,
        out: *mut PowerTelemetry,
    ) -> CtlResult {
        FAKE.with_borrow_mut(|f| {
            // SAFETY: the layer passes a 1024-byte record.
            let (size, version) = unsafe { ((*out).size, (*out).version) };
            f.telemetry_calls.push((size, version));
            let d = &mut f.devices[device_index(device)];
            if let (Some(rc), 1) = (d.reject_v1, version) {
                return rc;
            }
            let reply = if d.replies.len() > 1 {
                d.replies.remove(0)
            } else {
                d.replies[0]
            };
            match reply {
                Ok(t) => {
                    // SAFETY: copy only the announced size, then restore the header, as the
                    // real library fills the layout it was asked for.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            (&t as *const PowerTelemetry).cast::<u8>(),
                            out.cast::<u8>(),
                            size as usize,
                        );
                        (*out).size = size;
                        (*out).version = version;
                    }
                    SUCCESS
                }
                Err(rc) => rc,
            }
        })
    }

    const FAKE_API: Api = Api {
        init: fake_init,
        enumerate: fake_enumerate,
        properties: fake_properties,
        telemetry: fake_telemetry,
    };

    fn fake_layer(fake: Fake) -> Option<IgclLayer> {
        FAKE.set(fake);
        IgclLayer::start(FAKE_API, None)
    }

    fn one_device(replies: Vec<Result<PowerTelemetry, CtlResult>>) -> Fake {
        Fake {
            devices: vec![FakeDevice {
                luid: 0x0000_0001_0000_ABCD,
                bdf: AdapterBdf {
                    bus: 3,
                    device: 0,
                    function: 0,
                },
                reject_v1: None,
                replies,
            }],
            ..Fake::default()
        }
    }

    // ---- tests

    #[test]
    fn init_retries_once_with_api_1_0() {
        let layer = fake_layer(Fake {
            init_replies: vec![ERROR_UNSUPPORTED_VERSION, SUCCESS],
            ..Fake::default()
        });
        assert!(layer.is_some());
        assert_eq!(
            FAKE.with_borrow(|f| f.init_versions.clone()),
            [API_VERSION_1_1, API_VERSION_1_0]
        );
    }

    #[test]
    fn init_failures_mean_absent() {
        // Not a version problem (e.g. unsupported platform): no retry.
        assert!(fake_layer(Fake {
            init_replies: vec![0x4000_0020],
            ..Fake::default()
        })
        .is_none());
        assert_eq!(FAKE.with_borrow(|f| f.init_versions.len()), 1);
        // Neither version accepted.
        assert!(fake_layer(Fake {
            init_replies: vec![ERROR_UNSUPPORTED_VERSION, ERROR_UNSUPPORTED_VERSION],
            ..Fake::default()
        })
        .is_none());
        assert_eq!(FAKE.with_borrow(|f| f.init_versions.len()), 2);
    }

    #[test]
    fn attach_matches_by_luid_then_by_pci_address() {
        let mut fake = one_device(vec![Ok(card(1.0, 100.0))]);
        fake.devices.push(FakeDevice {
            luid: 0, // LUID not reported: fall back to bus/device/function
            bdf: AdapterBdf {
                bus: 0,
                device: 2,
                function: 0,
            },
            reject_v1: None,
            replies: vec![Ok(card(1.0, 100.0))],
        });
        let mut layer = fake_layer(fake).expect("init");
        let mut igpu = adapter(0x77, 0x8086, 0);
        igpu.pci = Some(PciAddress {
            bus: 0,
            device: 2,
            function: 0,
        });
        let adapters = [
            adapter(0x0000_0002_0000_0001, 0x10DE, 1),
            igpu,
            adapter(0x0000_0001_0000_ABCD, 0x8086, 3),
        ];

        let supported = layer.attach(&adapters);

        assert!(supported[0].is_empty());
        assert!(supported[1].contains(&GpuField::TemperatureCore));
        assert!(supported[2].contains(&GpuField::TemperatureCore));
    }

    #[test]
    fn telemetry_maps_supported_items() {
        let mut t = card(10.0, 500.0);
        t.gpu_voltage = TelemetryItem {
            units: UNITS_MILLIVOLTS,
            ..item(950.0)
        };
        t.vram_temperature.supported = 0;
        let (fields, energy) = supported_fields(&t);
        assert_eq!(
            fields,
            BTreeSet::from([
                GpuField::TemperatureCore,
                GpuField::ClockCore,
                GpuField::ClockMemory,
                GpuField::PowerBoard,
                GpuField::FanRpm,
                GpuField::VoltageCore,
                GpuField::ThrottlePower,
                GpuField::ThrottleThermal,
            ])
        );
        assert_eq!(energy, Some(EnergyCounter::TotalCard));
        assert_eq!(
            readings_from(&t, &fields),
            Readings::from([
                (GpuField::TemperatureCore, 61.0),
                (GpuField::ClockCore, 2400.0),
                (GpuField::ClockMemory, 1093.0),
                (GpuField::FanRpm, 1500.0),
                (GpuField::VoltageCore, 0.95),
                (GpuField::ThrottlePower, 1.0),
                (GpuField::ThrottleThermal, 0.0),
            ])
        );
    }

    #[test]
    fn non_positive_temperatures_are_missing() {
        let mut t = card(1.0, 1.0);
        t.gpu_temperature = item(0.0);
        t.vram_temperature = item(-1.0);
        let (fields, _) = supported_fields(&t);
        let readings = readings_from(&t, &fields);
        assert!(!readings.contains_key(&GpuField::TemperatureCore));
        assert!(!readings.contains_key(&GpuField::TemperatureMemory));
    }

    #[test]
    fn power_uses_the_gpu_counter_without_a_card_counter() {
        let mut t = card(1.0, 100.0);
        t.total_card_energy_counter.supported = 0;
        assert_eq!(supported_fields(&t).1, Some(EnergyCounter::Gpu));
        t.time_stamp.supported = 0;
        let (fields, energy) = supported_fields(&t);
        assert_eq!(energy, None);
        assert!(!fields.contains(&GpuField::PowerBoard));
    }

    #[test]
    fn energy_rate_needs_two_samples() {
        let sample = |time, joules| Some(EnergySample { time, joules });
        let mut meter = EnergyMeter::default();
        assert_eq!(meter.update(sample(100.0, 5000.0)), None, "first sample");
        assert_eq!(meter.update(sample(101.0, 5150.0)), Some(150.0));
        // Cached read (same timestamp): no value, baseline kept.
        assert_eq!(meter.update(sample(101.0, 5150.0)), None);
        assert_eq!(meter.update(sample(103.0, 5450.0)), Some(150.0));
        // Counter went backwards: no value, restart from the new read.
        assert_eq!(meter.update(sample(104.0, 10.0)), None);
        assert_eq!(meter.update(sample(105.0, 60.0)), Some(50.0));
    }

    #[test]
    fn layer_reports_power_from_the_second_sample() {
        let mut layer = fake_layer(one_device(vec![
            Ok(card(1.0, 100.0)), // attach probe
            Ok(card(2.0, 200.0)),
            Ok(card(3.0, 320.0)),
        ]))
        .expect("init");
        let adapters = [adapter(0x0000_0001_0000_ABCD, 0x8086, 3)];
        layer.attach(&adapters);

        let first = layer.sample().expect("first sample");
        assert!(!first[0].contains_key(&GpuField::PowerBoard));
        assert_eq!(first[0][&GpuField::TemperatureCore], 61.0);
        let second = layer.sample().expect("second sample");
        assert_eq!(second[0][&GpuField::PowerBoard], 120.0);
    }

    #[test]
    fn telemetry_falls_back_to_the_808_byte_layout_once() {
        let mut fake = one_device(vec![Ok(card(1.0, 100.0))]);
        fake.devices[0].reject_v1 = Some(ERROR_INVALID_SIZE);
        let mut layer = fake_layer(fake).expect("init");
        let adapters = [adapter(0x0000_0001_0000_ABCD, 0x8086, 3)];

        let supported = layer.attach(&adapters);
        let readings = layer.sample().expect("sample");

        assert!(supported[0].contains(&GpuField::ClockCore));
        assert_eq!(readings[0][&GpuField::ClockCore], 2400.0);
        assert_eq!(
            FAKE.with_borrow(|f| f.telemetry_calls.clone()),
            [(1024, 1), (808, 0), (808, 0)]
        );
    }

    #[test]
    fn device_unavailable_means_all_missing_not_an_error() {
        let mut layer = fake_layer(one_device(vec![
            Ok(card(1.0, 100.0)),
            Err(ERROR_DEVICE_UNAVAILABLE),
            Ok(card(2.0, 150.0)),
        ]))
        .expect("init");
        layer.attach(&[adapter(0x0000_0001_0000_ABCD, 0x8086, 3)]);

        assert_eq!(layer.sample(), Ok(vec![Readings::new()]));
        assert_eq!(
            layer.sample().expect("recovered")[0][&GpuField::ClockCore],
            2400.0
        );
    }

    #[test]
    fn device_lost_asks_for_rediscover() {
        let mut layer = fake_layer(one_device(vec![
            Ok(card(1.0, 100.0)),
            Err(ERROR_DEVICE_LOST),
        ]))
        .expect("init");
        layer.attach(&[adapter(0x0000_0001_0000_ABCD, 0x8086, 3)]);

        assert_eq!(layer.sample(), Err(ProviderError::Rediscover));
    }

    #[test]
    fn pending_device_asks_for_rediscover_once_it_answers() {
        let mut layer = fake_layer(one_device(vec![
            Err(ERROR_DEVICE_UNAVAILABLE), // attach probe: transiently unavailable
            Err(ERROR_DEVICE_UNAVAILABLE), // still unavailable on the first sample
            Ok(card(1.0, 100.0)),          // now answers: ask for a rediscover
        ]))
        .expect("init");
        let adapters = [adapter(0x0000_0001_0000_ABCD, 0x8086, 3)];

        let supported = layer.attach(&adapters);
        assert!(supported[0].is_empty());
        assert_eq!(layer.sample(), Ok(vec![Readings::new()]));
        assert_eq!(layer.sample(), Err(ProviderError::Rediscover));

        // The next discover's attach probes it again and, now that it answers, declares fields.
        let supported = layer.attach(&adapters);
        assert!(supported[0].contains(&GpuField::TemperatureCore));
    }

    #[test]
    fn item_values_follow_their_type_tag() {
        let mut unsupported = item(1.0);
        unsupported.supported = 0;
        assert_eq!(unsupported.value(), None);
        let counter = TelemetryItem {
            data_type: TYPE_U64,
            value: DataValue { u64: 42 },
            ..item(0.0)
        };
        assert_eq!(counter.value(), Some(42.0));
        let signed = TelemetryItem {
            data_type: TYPE_I32,
            value: DataValue { i32: -5 },
            ..item(0.0)
        };
        assert_eq!(signed.value(), Some(-5.0));
        let unknown = TelemetryItem {
            data_type: 0x4800_FFFF,
            ..item(1.0)
        };
        assert_eq!(unknown.value(), None);
        assert_eq!(item(f64::NAN).value(), None);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn igcl_absent_without_intel_gpu() {
        // Without Intel graphics, IGCL must report itself absent: either ControlLib.dll is
        // missing (the development machine) or a leftover copy fails ctlInit.
        let adapters = crate::gpu::enumerate::enumerate().expect("enumerate");
        if adapters.iter().any(|a| a.vendor() == Some(Vendor::Intel)) {
            eprintln!("Intel adapter present: skipped");
            return;
        }
        assert!(IgclLayer::load().is_none());
    }

    #[test]
    #[ignore = "requires an Intel GPU"]
    fn igcl_reads_an_intel_gpu() {
        let adapters = crate::gpu::enumerate::enumerate().expect("enumerate");
        // `--include-ignored` also runs this on machines without Intel graphics (like the
        // development machine): nothing to check there.
        let Some(intel) = adapters
            .iter()
            .position(|a| a.vendor() == Some(Vendor::Intel))
        else {
            eprintln!("no Intel adapter: skipped");
            return;
        };
        let mut layer = IgclLayer::load().expect("ControlLib.dll loads and ctlInit succeeds");

        let supported = layer.attach(&adapters);
        assert!(supported[intel].contains(&GpuField::ThrottleThermal));
        let _ = layer.sample().expect("first sample");
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        let readings = layer.sample().expect("second sample");

        if let Some(t) = readings[intel].get(&GpuField::TemperatureCore) {
            assert!((1.0..=110.0).contains(t), "temperature {t}");
        }
        if let Some(clock) = readings[intel].get(&GpuField::ClockCore) {
            assert!((0.0..=5000.0).contains(clock), "clock {clock}");
        }
        if supported[intel].contains(&GpuField::PowerBoard) {
            let watts = readings[intel][&GpuField::PowerBoard];
            assert!((0.0..=1000.0).contains(&watts), "power {watts}");
        }
    }
}
