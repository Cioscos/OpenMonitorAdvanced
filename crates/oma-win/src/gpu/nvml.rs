//! NVIDIA layer over NVML (nvml.dll, loaded from System32 only): core temperature, clocks,
//! board power and limit, fans, dedicated memory and throttle reasons.
//!
//! The declarations below are our own interoperability declarations, written from the public
//! NVML API reference (symbol names, argument types, constant values, struct layouts). No
//! NVIDIA header is used or included. Every function except `nvmlInit_v2` is optional: a
//! missing symbol only means that the fields depending on it are not offered.
//!
//! Lifetime (decision D1): NVML is initialised once and never shut down or unloaded.
//! `nvmlShutdown` touches the 19 MB `.data` section again and a new init would pay it again.

use std::collections::BTreeSet;
use std::ffi::{c_char, c_void, CString};

use oma_core::model::Source;
use oma_core::provider::ProviderError;

use super::adapter::{Adapter, PciAddress, Vendor};
use super::field::GpuField;
use super::layer::{GpuLayer, Readings};
use super::trim;
use crate::dynlib::Library;

/// Return code of every NVML function (a C enum, passed as `int`).
type Ret = i32;
/// Opaque per-GPU token handed out by NVML.
type DeviceHandle = *mut c_void;

const SUCCESS: Ret = 0;
const ERROR_UNINITIALIZED: Ret = 1;
const ERROR_NOT_SUPPORTED: Ret = 3;
const ERROR_DRIVER_NOT_LOADED: Ret = 9;
const ERROR_FUNCTION_NOT_FOUND: Ret = 13;
const ERROR_GPU_IS_LOST: Ret = 15;

/// Temperature sensor id of the GPU die.
const SENSOR_GPU: u32 = 0;
/// Clock domains of `nvmlDeviceGetClockInfo`.
const CLOCK_GRAPHICS: u32 = 0;
const CLOCK_MEMORY: u32 = 2;

/// Clock event ("throttle") reason bits.
const REASON_SW_POWER_CAP: u64 = 0x4;
const REASON_SW_THERMAL: u64 = 0x20;
const REASON_HW_THERMAL: u64 = 0x40;
const REASON_HW_POWER_BRAKE: u64 = 0x80;
const POWER_REASONS: u64 = REASON_SW_POWER_CAP | REASON_HW_POWER_BRAKE;
const THERMAL_REASONS: u64 = REASON_SW_THERMAL | REASON_HW_THERMAL;

/// Fields this layer can offer, probed per GPU at attach.
const FIELDS: [GpuField; 12] = [
    GpuField::MemoryDedicatedUsed,
    GpuField::MemoryDedicatedTotal,
    GpuField::TemperatureCore,
    GpuField::ClockCore,
    GpuField::ClockMemory,
    GpuField::PowerBoard,
    GpuField::PowerLimit,
    GpuField::PowerLimitPercent,
    GpuField::FanPercent,
    GpuField::FanRpm,
    GpuField::ThrottlePower,
    GpuField::ThrottleThermal,
];

/// Version tag of a versioned NVML struct: its size in the low bits, the version in the top byte.
const fn versioned<T>(version: u32) -> u32 {
    std::mem::size_of::<T>() as u32 | (version << 24)
}

/// PCI identity filled by `nvmlDeviceGetPciInfo_v3`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct PciInfo {
    bus_id_legacy: [u8; 16],
    domain: u32,
    bus: u32,
    device: u32,
    pci_device_id: u32,
    pci_subsystem_id: u32,
    bus_id: [u8; 32],
}
const _: () = assert!(std::mem::size_of::<PciInfo>() == 68);

/// Memory totals filled by `nvmlDeviceGetMemoryInfo` (first revision).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct MemoryV1 {
    total: u64,
    free: u64,
    used: u64,
}
const _: () = assert!(std::mem::size_of::<MemoryV1>() == 24);

/// Memory totals filled by `nvmlDeviceGetMemoryInfo_v2` (`used` excludes the driver reserve).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct MemoryV2 {
    version: u32,
    total: u64,
    reserved: u64,
    free: u64,
    used: u64,
}
const _: () = assert!(std::mem::size_of::<MemoryV2>() == 40);
const MEMORY_V2: u32 = versioned::<MemoryV2>(2);
const _: () = assert!(MEMORY_V2 == 0x0200_0028);

/// In/out argument of `nvmlDeviceGetTemperatureV`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct TemperatureV1 {
    version: u32,
    sensor: u32,
    celsius: i32,
}
const _: () = assert!(std::mem::size_of::<TemperatureV1>() == 12);
const TEMPERATURE_V1: u32 = versioned::<TemperatureV1>(1);
const _: () = assert!(TEMPERATURE_V1 == 0x0100_000C);

/// In/out argument of `nvmlDeviceGetFanSpeedRPM`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct FanSpeedV1 {
    version: u32,
    fan: u32,
    rpm: u32,
}
const _: () = assert!(std::mem::size_of::<FanSpeedV1>() == 12);
const FAN_SPEED_V1: u32 = versioned::<FanSpeedV1>(1);
const _: () = assert!(FAN_SPEED_V1 == 0x0100_000C);

type InitFn = unsafe extern "C" fn() -> Ret;
type CountFn = unsafe extern "C" fn(*mut u32) -> Ret;
type ByIndexFn = unsafe extern "C" fn(u32, *mut DeviceHandle) -> Ret;
type ByBusIdFn = unsafe extern "C" fn(*const c_char, *mut DeviceHandle) -> Ret;
type PciInfoFn = unsafe extern "C" fn(DeviceHandle, *mut PciInfo) -> Ret;
type U32Fn = unsafe extern "C" fn(DeviceHandle, *mut u32) -> Ret;
type U32ArgFn = unsafe extern "C" fn(DeviceHandle, u32, *mut u32) -> Ret;
type U64Fn = unsafe extern "C" fn(DeviceHandle, *mut u64) -> Ret;
type TemperatureVFn = unsafe extern "C" fn(DeviceHandle, *mut TemperatureV1) -> Ret;
type FanSpeedRpmFn = unsafe extern "C" fn(DeviceHandle, *mut FanSpeedV1) -> Ret;
type MemoryV1Fn = unsafe extern "C" fn(DeviceHandle, *mut MemoryV1) -> Ret;
type MemoryV2Fn = unsafe extern "C" fn(DeviceHandle, *mut MemoryV2) -> Ret;

/// One raw read: the NVML return code and, on success, the value in the field's unit
/// (`None` when the call succeeded but the value is unusable, e.g. a zero power limit).
type Read = (Ret, Option<f64>);

/// A lost GPU, an unloaded driver or an uninitialised library: the binding is stale.
fn is_stale(ret: Ret) -> bool {
    matches!(
        ret,
        ERROR_UNINITIALIZED | ERROR_DRIVER_NOT_LOADED | ERROR_GPU_IS_LOST
    )
}

/// Fields among `candidates` whose getter succeeds once (support is decided at attach).
fn probe(candidates: &[GpuField], mut read: impl FnMut(GpuField) -> Read) -> BTreeSet<GpuField> {
    candidates
        .iter()
        .copied()
        .filter(|&field| read(field).0 == SUCCESS)
        .collect()
}

/// Reads every field of `fields` through `read`. A stale binding asks for a rediscover; any
/// other failure (NOT_SUPPORTED, a timeout, ...) only drops that field for this tick.
fn collect(
    fields: &BTreeSet<GpuField>,
    mut read: impl FnMut(GpuField) -> Read,
) -> Result<Readings, ProviderError> {
    let mut readings = Readings::new();
    for &field in fields {
        match read(field) {
            (SUCCESS, Some(value)) if value.is_finite() => {
                readings.insert(field, value);
            }
            (ret, _) if is_stale(ret) => return Err(ProviderError::Rediscover),
            _ => {}
        }
    }
    Ok(readings)
}

fn watts(milliwatts: u32) -> f64 {
    f64::from(milliwatts) / 1000.0
}

/// Board power as a share of the enforced limit; `None` without a limit.
fn power_limit_percent(usage_mw: u32, limit_mw: u32) -> Option<f64> {
    (limit_mw > 0).then(|| f64::from(usage_mw) * 100.0 / f64::from(limit_mw))
}

fn flag(on: bool) -> f64 {
    if on {
        1.0
    } else {
        0.0
    }
}

fn throttle_power(reasons: u64) -> bool {
    reasons & POWER_REASONS != 0
}

fn throttle_thermal(reasons: u64) -> bool {
    reasons & THERMAL_REASONS != 0
}

/// Highest duty among the fans that answered; the last failure code when none did.
fn max_fan(reads: impl IntoIterator<Item = (Ret, u32)>) -> Read {
    let mut best: Option<u32> = None;
    let mut failure = ERROR_NOT_SUPPORTED;
    for (ret, percent) in reads {
        if ret == SUCCESS {
            best = Some(best.map_or(percent, |b| b.max(percent)));
        } else {
            failure = ret;
        }
    }
    match best {
        Some(percent) => (SUCCESS, Some(f64::from(percent))),
        None => (failure, None),
    }
}

/// Bus id string accepted by `nvmlDeviceGetHandleByPciBusId_v2`, e.g. "0000:01:00.0".
fn bus_id(pci: PciAddress) -> CString {
    CString::new(pci.to_string()).expect("a PCI address has no NUL byte")
}

fn pci_matches(info: &PciInfo, pci: PciAddress) -> bool {
    info.bus == pci.bus && info.device == pci.device
}

/// The resolved NVML entry points.
struct Api {
    init: InitFn,
    count: Option<CountFn>,
    by_index: Option<ByIndexFn>,
    by_bus_id: Option<ByBusIdFn>,
    pci_info: Option<PciInfoFn>,
    temperature_v: Option<TemperatureVFn>,
    temperature: Option<U32ArgFn>,
    clock_info: Option<U32ArgFn>,
    power_usage: Option<U32Fn>,
    enforced_power_limit: Option<U32Fn>,
    num_fans: Option<U32Fn>,
    fan_speed: Option<U32ArgFn>,
    fan_speed_rpm: Option<FanSpeedRpmFn>,
    memory_v2: Option<MemoryV2Fn>,
    memory_v1: Option<MemoryV1Fn>,
    event_reasons: Option<U64Fn>,
    throttle_reasons: Option<U64Fn>,
}

fn call_u32(f: Option<U32Fn>, device: DeviceHandle) -> (Ret, u32) {
    let Some(f) = f else {
        return (ERROR_FUNCTION_NOT_FOUND, 0);
    };
    let mut value = 0u32;
    // SAFETY: `f` has the declared signature, `device` is a handle NVML returned and `value`
    // is a valid out pointer.
    (unsafe { f(device, &mut value) }, value)
}

fn call_u32_arg(f: Option<U32ArgFn>, device: DeviceHandle, arg: u32) -> (Ret, u32) {
    let Some(f) = f else {
        return (ERROR_FUNCTION_NOT_FOUND, 0);
    };
    let mut value = 0u32;
    // SAFETY: as in `call_u32`.
    (unsafe { f(device, arg, &mut value) }, value)
}

fn call_u64(f: Option<U64Fn>, device: DeviceHandle) -> (Ret, u64) {
    let Some(f) = f else {
        return (ERROR_FUNCTION_NOT_FOUND, 0);
    };
    let mut value = 0u64;
    // SAFETY: as in `call_u32`.
    (unsafe { f(device, &mut value) }, value)
}

impl Api {
    /// Resolves the entry points; `None` without `nvmlInit_v2`.
    fn resolve(library: &Library) -> Option<Self> {
        // SAFETY (all `symbol` calls): each type alias is the exact C signature of the named
        // NVML export (public NVML API reference).
        unsafe {
            Some(Self {
                init: library.symbol(c"nvmlInit_v2")?,
                count: library.symbol(c"nvmlDeviceGetCount_v2"),
                by_index: library.symbol(c"nvmlDeviceGetHandleByIndex_v2"),
                by_bus_id: library.symbol(c"nvmlDeviceGetHandleByPciBusId_v2"),
                pci_info: library.symbol(c"nvmlDeviceGetPciInfo_v3"),
                temperature_v: library.symbol(c"nvmlDeviceGetTemperatureV"),
                temperature: library.symbol(c"nvmlDeviceGetTemperature"),
                clock_info: library.symbol(c"nvmlDeviceGetClockInfo"),
                power_usage: library.symbol(c"nvmlDeviceGetPowerUsage"),
                enforced_power_limit: library.symbol(c"nvmlDeviceGetEnforcedPowerLimit"),
                num_fans: library.symbol(c"nvmlDeviceGetNumFans"),
                fan_speed: library.symbol(c"nvmlDeviceGetFanSpeed_v2"),
                fan_speed_rpm: library.symbol(c"nvmlDeviceGetFanSpeedRPM"),
                memory_v2: library.symbol(c"nvmlDeviceGetMemoryInfo_v2"),
                memory_v1: library.symbol(c"nvmlDeviceGetMemoryInfo"),
                event_reasons: library.symbol(c"nvmlDeviceGetCurrentClocksEventReasons"),
                throttle_reasons: library.symbol(c"nvmlDeviceGetCurrentClocksThrottleReasons"),
            })
        }
    }

    /// The NVML handle of the GPU at `pci`: by bus id, else by scanning every handle.
    fn handle_for(&self, pci: PciAddress) -> Option<DeviceHandle> {
        if let Some(by_bus_id) = self.by_bus_id {
            let id = bus_id(pci);
            let mut device: DeviceHandle = std::ptr::null_mut();
            // SAFETY: `id` is a NUL-terminated string and `device` a valid out pointer.
            if unsafe { by_bus_id(id.as_ptr(), &mut device) } == SUCCESS {
                return Some(device);
            }
        }
        let (count, by_index, pci_info) = (self.count?, self.by_index?, self.pci_info?);
        let mut n = 0u32;
        // SAFETY: `n` is a valid out pointer.
        if unsafe { count(&mut n) } != SUCCESS {
            return None;
        }
        (0..n).find_map(|index| {
            let mut device: DeviceHandle = std::ptr::null_mut();
            let mut info = PciInfo::default();
            // SAFETY: valid out pointers; `device` is only used after NVML filled it.
            let found = unsafe {
                by_index(index, &mut device) == SUCCESS
                    && pci_info(device, &mut info) == SUCCESS
                    && pci_matches(&info, pci)
            };
            found.then_some(device)
        })
    }

    fn fan_count(&self, device: DeviceHandle) -> u32 {
        match call_u32(self.num_fans, device) {
            (SUCCESS, n) => n,
            _ => 0,
        }
    }

    fn temperature(&self, device: DeviceHandle) -> Read {
        if let Some(f) = self.temperature_v {
            let mut t = TemperatureV1 {
                version: TEMPERATURE_V1,
                sensor: SENSOR_GPU,
                celsius: 0,
            };
            // SAFETY: `t` is a valid, versioned in/out struct.
            let ret = unsafe { f(device, &mut t) };
            if ret == SUCCESS || self.temperature.is_none() {
                return (ret, Some(f64::from(t.celsius)));
            }
        }
        let (ret, celsius) = call_u32_arg(self.temperature, device, SENSOR_GPU);
        (ret, Some(f64::from(celsius)))
    }

    fn fan_rpm(&self, device: DeviceHandle) -> Read {
        let Some(f) = self.fan_speed_rpm else {
            return (ERROR_FUNCTION_NOT_FOUND, None);
        };
        let mut s = FanSpeedV1 {
            version: FAN_SPEED_V1,
            fan: 0,
            rpm: 0,
        };
        // SAFETY: `s` is a valid, versioned in/out struct.
        let ret = unsafe { f(device, &mut s) };
        (ret, Some(f64::from(s.rpm)))
    }

    /// (used, total) bytes of dedicated memory.
    fn memory(&self, device: DeviceHandle) -> (Ret, u64, u64) {
        if let Some(f) = self.memory_v2 {
            let mut m = MemoryV2 {
                version: MEMORY_V2,
                ..MemoryV2::default()
            };
            // SAFETY: `m` is a valid, versioned out struct.
            let ret = unsafe { f(device, &mut m) };
            if ret == SUCCESS || self.memory_v1.is_none() {
                return (ret, m.used, m.total);
            }
        }
        let Some(f) = self.memory_v1 else {
            return (ERROR_FUNCTION_NOT_FOUND, 0, 0);
        };
        let mut m = MemoryV1::default();
        // SAFETY: `m` is a valid out struct.
        let ret = unsafe { f(device, &mut m) };
        (ret, m.used, m.total)
    }

    fn reasons(&self, device: DeviceHandle) -> (Ret, u64) {
        match call_u64(self.event_reasons, device) {
            (ERROR_FUNCTION_NOT_FOUND, _) => call_u64(self.throttle_reasons, device),
            read => read,
        }
    }

    /// Reads one field of `device` (which has `fans` fans).
    fn read(&self, device: DeviceHandle, fans: u32, field: GpuField) -> Read {
        let u32_read = |(ret, value): (Ret, u32), map: fn(u32) -> f64| (ret, Some(map(value)));
        match field {
            GpuField::TemperatureCore => self.temperature(device),
            GpuField::ClockCore => u32_read(
                call_u32_arg(self.clock_info, device, CLOCK_GRAPHICS),
                f64::from,
            ),
            GpuField::ClockMemory => u32_read(
                call_u32_arg(self.clock_info, device, CLOCK_MEMORY),
                f64::from,
            ),
            GpuField::PowerBoard => u32_read(call_u32(self.power_usage, device), watts),
            GpuField::PowerLimit => u32_read(call_u32(self.enforced_power_limit, device), watts),
            GpuField::PowerLimitPercent => match call_u32(self.power_usage, device) {
                (SUCCESS, usage) => match call_u32(self.enforced_power_limit, device) {
                    (SUCCESS, limit) => (SUCCESS, power_limit_percent(usage, limit)),
                    (ret, _) => (ret, None),
                },
                (ret, _) => (ret, None),
            },
            GpuField::FanPercent => {
                max_fan((0..fans).map(|fan| call_u32_arg(self.fan_speed, device, fan)))
            }
            GpuField::FanRpm => self.fan_rpm(device),
            GpuField::MemoryDedicatedUsed => {
                let (ret, used, _) = self.memory(device);
                (ret, Some(used as f64))
            }
            GpuField::MemoryDedicatedTotal => {
                let (ret, _, total) = self.memory(device);
                (ret, Some(total as f64))
            }
            GpuField::ThrottlePower => {
                let (ret, reasons) = self.reasons(device);
                (ret, Some(flag(throttle_power(reasons))))
            }
            GpuField::ThrottleThermal => {
                let (ret, reasons) = self.reasons(device);
                (ret, Some(flag(throttle_thermal(reasons))))
            }
            _ => (ERROR_NOT_SUPPORTED, None),
        }
    }
}

/// An adapter bound to its NVML handle, with the fields that answered at attach.
struct Bound {
    device: DeviceHandle,
    fans: u32,
    fields: BTreeSet<GpuField>,
}

/// NVML enrichment layer (highest priority, spec §5.2).
pub(crate) struct NvmlLayer {
    api: Api,
    /// Per adapter of the last attach: its NVML binding, if it is an NVIDIA GPU NVML knows.
    bound: Vec<Option<Bound>>,
    /// Keeps nvml.dll referenced; it is never unloaded (D1).
    _library: Library,
}

// SAFETY: NVML is documented as thread-safe and its device handles are process-wide tokens,
// not tied to the thread that obtained them. The layer is used by one thread at a time.
unsafe impl Send for NvmlLayer {}

impl NvmlLayer {
    /// Loads nvml.dll from System32 and initialises NVML; `None` when the DLL is missing,
    /// `nvmlInit_v2` is missing or fails. Never shuts NVML down (D1).
    pub(crate) fn load() -> Option<Self> {
        let library = match Library::system32("nvml.dll") {
            Ok(library) => library,
            Err(e) => {
                tracing::debug!(error = %e, "nvml.dll not available");
                return None;
            }
        };
        let Some(api) = Api::resolve(&library) else {
            tracing::warn!("nvml.dll has no nvmlInit_v2; NVML disabled");
            return None;
        };
        // SAFETY: `init` has the declared signature and takes no arguments.
        let ret = unsafe { (api.init)() };
        if ret != SUCCESS {
            tracing::warn!(code = ret, "nvmlInit_v2 failed; NVML disabled");
            return None;
        }
        // Decision D2: give back the 19 MB NVML wrote into its DriverStore copy's .data.
        match trim::release_module_data("DriverStore", "nvml.dll") {
            Ok(bytes) => tracing::debug!(bytes, "released nvml.dll .data from the working set"),
            Err(e) => tracing::warn!(error = %e, "could not release nvml.dll .data pages"),
        }
        Some(Self {
            api,
            bound: Vec::new(),
            _library: library,
        })
    }

    fn bind(&self, adapter: &Adapter) -> Option<Bound> {
        if adapter.vendor() != Some(Vendor::Nvidia) {
            return None;
        }
        let device = self.api.handle_for(adapter.pci?)?;
        let fans = self.api.fan_count(device);
        let fields = probe(&FIELDS, |field| self.api.read(device, fans, field));
        Some(Bound {
            device,
            fans,
            fields,
        })
    }
}

impl GpuLayer for NvmlLayer {
    fn source(&self) -> Source {
        Source::Nvml
    }

    fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>> {
        self.bound = adapters.iter().map(|adapter| self.bind(adapter)).collect();
        self.bound
            .iter()
            .map(|bound| bound.as_ref().map(|b| b.fields.clone()).unwrap_or_default())
            .collect()
    }

    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
        self.bound
            .iter()
            .map(|bound| match bound {
                Some(b) => collect(&b.fields, |field| self.api.read(b.device, b.fans, field)),
                None => Ok(Readings::new()),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS_CHILD_ENV: &str = "OMA_NVML_WS_CHILD";

    fn fields(list: &[GpuField]) -> BTreeSet<GpuField> {
        list.iter().copied().collect()
    }

    #[test]
    fn lost_gpu_requests_rediscover() {
        let set = fields(&[GpuField::TemperatureCore, GpuField::ClockCore]);
        for code in [
            ERROR_GPU_IS_LOST,
            ERROR_UNINITIALIZED,
            ERROR_DRIVER_NOT_LOADED,
        ] {
            let result = collect(&set, |field| match field {
                GpuField::ClockCore => (code, None),
                _ => (SUCCESS, Some(47.0)),
            });
            assert_eq!(result, Err(ProviderError::Rediscover), "code {code}");
        }
        let result = collect(&set, |field| match field {
            GpuField::ClockCore => (ERROR_NOT_SUPPORTED, None),
            _ => (SUCCESS, Some(47.0)),
        });
        assert_eq!(
            result,
            Ok(Readings::from([(GpuField::TemperatureCore, 47.0)]))
        );
    }

    #[test]
    fn unusable_values_are_missing() {
        let set = fields(&[GpuField::PowerLimitPercent, GpuField::PowerBoard]);
        let result = collect(&set, |field| match field {
            GpuField::PowerLimitPercent => (SUCCESS, None),
            _ => (SUCCESS, Some(f64::NAN)),
        });
        assert_eq!(result, Ok(Readings::new()));
    }

    #[test]
    fn support_is_what_answers_at_attach() {
        let supported = probe(&FIELDS, |field| match field {
            GpuField::FanRpm => (ERROR_FUNCTION_NOT_FOUND, None),
            GpuField::FanPercent => (ERROR_NOT_SUPPORTED, None),
            GpuField::PowerLimitPercent => (SUCCESS, None),
            _ => (SUCCESS, Some(1.0)),
        });
        assert_eq!(supported.len(), FIELDS.len() - 2);
        assert!(!supported.contains(&GpuField::FanRpm));
        assert!(!supported.contains(&GpuField::FanPercent));
        assert!(supported.contains(&GpuField::PowerLimitPercent));
    }

    #[test]
    fn power_conversions() {
        assert_eq!(watts(33_510), 33.51);
        let pct = power_limit_percent(33_510, 320_000).unwrap();
        assert!((pct - 10.471_875).abs() < 1e-9);
        assert_eq!(power_limit_percent(33_510, 0), None);
    }

    #[test]
    fn throttle_reason_bits() {
        assert!(!throttle_power(0x1) && !throttle_thermal(0x1)); // GPU idle
        assert!(throttle_power(0x4) && throttle_power(0x80));
        assert!(throttle_thermal(0x20) && throttle_thermal(0x40));
        assert!(!throttle_thermal(0x8) && !throttle_power(0x8));
        assert!(!throttle_thermal(0x88) && throttle_power(0x88));
        assert!(!throttle_power(0x200) && !throttle_thermal(0x200));
        assert!(!throttle_power(0x400) && !throttle_thermal(0x400)); // reliability
        assert!(!throttle_power(0x68) && throttle_thermal(0x68));
        assert_eq!(flag(true), 1.0);
        assert_eq!(flag(false), 0.0);
    }

    #[test]
    fn fan_percent_is_the_fastest_fan() {
        assert_eq!(
            max_fan([(SUCCESS, 32), (SUCCESS, 31)]),
            (SUCCESS, Some(32.0))
        );
        assert_eq!(
            max_fan([(ERROR_NOT_SUPPORTED, 0), (SUCCESS, 40)]),
            (SUCCESS, Some(40.0))
        );
        assert_eq!(max_fan([]), (ERROR_NOT_SUPPORTED, None));
        assert_eq!(
            max_fan([(ERROR_GPU_IS_LOST, 0), (ERROR_GPU_IS_LOST, 0)]),
            (ERROR_GPU_IS_LOST, None)
        );
    }

    #[test]
    fn pci_lookup_helpers() {
        let pci = PciAddress {
            bus: 1,
            device: 0,
            function: 0,
        };
        assert_eq!(bus_id(pci).to_str(), Ok("0000:01:00.0"));
        let info = PciInfo {
            bus: 1,
            device: 0,
            ..PciInfo::default()
        };
        assert!(pci_matches(&info, pci));
        assert!(!pci_matches(
            &info,
            PciAddress {
                bus: 0x11,
                device: 0,
                function: 0
            }
        ));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn nvml_reads_the_rtx_4080() {
        use GpuField::*;
        let adapters = crate::gpu::enumerate::enumerate().expect("enumerate");
        let nvidia = adapters
            .iter()
            .position(|a| a.vendor() == Some(Vendor::Nvidia))
            .expect("an NVIDIA adapter");
        let mut layer = NvmlLayer::load().expect("NVML loads");
        let supported = layer.attach(&adapters);
        assert_eq!(supported.len(), adapters.len());
        // Fields every NVML-capable desktop GPU answers; fans, reasons and the
        // versioned getters depend on the board and the driver.
        let core = fields(&[
            TemperatureCore,
            ClockCore,
            ClockMemory,
            PowerBoard,
            PowerLimit,
            MemoryDedicatedUsed,
            MemoryDedicatedTotal,
        ]);
        for (i, set) in supported.iter().enumerate() {
            if i == nvidia {
                assert!(set.is_superset(&core), "NVML fields {set:?}");
            } else {
                assert!(set.is_empty(), "NVML must not bind {}", adapters[i].name);
            }
        }
        println!("NVML fields: {:?}", supported[nvidia]);
        let readings = layer.sample().expect("sample");
        assert_eq!(readings.len(), adapters.len());
        let r = &readings[nvidia];
        println!("NVML readings: {r:?}");
        let temperature = r[&TemperatureCore];
        assert!(
            (20.0..=100.0).contains(&temperature),
            "temperature {temperature}"
        );
        let clock = r[&ClockCore];
        assert!(clock > 0.0 && clock < 4_000.0, "core clock {clock}");
        assert!(r[&ClockMemory] > 0.0);
        let power = r[&PowerBoard];
        assert!(power > 1.0 && power < 700.0, "power {power}");
        let limit = r[&PowerLimit];
        assert!((30.0..=700.0).contains(&limit), "power limit {limit}");
        let total = r[&MemoryDedicatedTotal];
        assert!(total > 1024.0 * 1024.0 * 1024.0, "vram {total}");
        assert!(r[&MemoryDedicatedUsed] <= total);
        for flag_field in [ThrottlePower, ThrottleThermal] {
            if let Some(value) = r.get(&flag_field) {
                assert!([0.0, 1.0].contains(value), "{flag_field:?} {value}");
            }
        }
        if let Some(fan) = r.get(&FanPercent) {
            assert!((0.0..=100.0).contains(fan), "fan {fan}");
        }
    }

    fn private_working_set() -> usize {
        use windows::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;
        let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
            ..Default::default()
        };
        // SAFETY: `cb` holds the size of the struct passed in; EX2 extends the base counters.
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2)
                    .cast::<PROCESS_MEMORY_COUNTERS>(),
                counters.cb,
            )
        }
        .expect("GetProcessMemoryInfo");
        counters.PrivateWorkingSetSize
    }

    /// Runs in a child process started by `nvml_keeps_private_working_set_small`; a no-op
    /// otherwise.
    #[test]
    #[ignore = "requires real Windows hardware"]
    fn private_ws_child() {
        if std::env::var_os(WS_CHILD_ENV).is_none() {
            return;
        }
        let adapters = crate::gpu::enumerate::enumerate().expect("enumerate");
        let before = private_working_set();
        let mut layer = NvmlLayer::load().expect("NVML loads");
        layer.attach(&adapters);
        for _ in 0..10 {
            layer.sample().expect("sample");
        }
        let after = private_working_set();
        println!("private-ws-delta={}", after.saturating_sub(before));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn nvml_keeps_private_working_set_small() {
        // Measured in a fresh process: another test may already have initialised NVML in this
        // one, which would hide the cost. Private working set is the budget metric
        // (docs/perf-budget.md, Task Manager "Memory").
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "gpu::nvml::tests::private_ws_child",
                "--ignored",
                "--nocapture",
            ])
            .env(WS_CHILD_ENV, "1")
            .output()
            .expect("child test process");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "child failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let delta: usize = stdout
            .lines()
            .find_map(|line| line.strip_prefix("private-ws-delta="))
            .expect("child printed the delta")
            .trim()
            .parse()
            .expect("delta is a number");
        println!("NVML private working set delta: {} KB", delta / 1024);
        assert!(
            delta < 5 * 1024 * 1024,
            "NVML added {} KB of private working set",
            delta / 1024
        );
    }
}
