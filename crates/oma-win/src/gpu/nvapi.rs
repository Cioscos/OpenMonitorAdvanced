//! NVIDIA layer over NVAPI (nvapi64.dll, loaded from System32 only): hotspot and memory
//! junction temperatures and core voltage. All three come from undocumented calls and are
//! marked experimental (spec §5.2, decision D7).
//!
//! Interface ids of the public calls (Initialize, EnumPhysicalGPUs, GPU_GetBusId,
//! GPU_GetBusSlotId) and the struct-version rule come from NVIDIA's NVAPI headers,
//! Copyright (c) NVIDIA CORPORATION & AFFILIATES, MIT License (see THIRD_PARTY_NOTICES.md).
//! The ids and layouts of GPU_ThermalGetSensors (0x65FE3AAD) and GPU_ClientVoltRailsGetStatus
//! (0x465F9BCF) are interoperability facts documented by LibreHardwareMonitor (credited in
//! THIRD_PARTY_NOTICES.md); no LibreHardwareMonitor code is used.
//!
//! Lifetime (decision D1): NvAPI_Unload is never called. After it every pointer returned by
//! nvapi_QueryInterface dangles and calling one is an access violation.

use std::collections::BTreeSet;
use std::ffi::c_void;

use oma_core::model::Source;
use oma_core::provider::ProviderError;

use super::adapter::{Adapter, PciAddress, Vendor};
use super::field::GpuField;
use super::layer::{GpuLayer, Readings};
use crate::dynlib::Library;

/// Result of every NVAPI call.
type Status = i32;
/// Opaque physical-GPU handle.
type GpuHandle = *mut c_void;

const OK: Status = 0;
const HANDLE_INVALIDATED: Status = -10;

const ID_INITIALIZE: u32 = 0x0150_E828;
const ID_ENUM_PHYSICAL_GPUS: u32 = 0xE5AC_921F;
const ID_GPU_GET_BUS_ID: u32 = 0x1BE0_B8E5;
const ID_GPU_GET_BUS_SLOT_ID: u32 = 0x2A0A_350F;
const ID_GPU_THERMAL_GET_SENSORS: u32 = 0x65FE_3AAD;
const ID_GPU_CLIENT_VOLT_RAILS_GET_STATUS: u32 = 0x465F_9BCF;

/// Size of the handle array EnumPhysicalGPUs fills.
const MAX_PHYSICAL_GPUS: usize = 64;

/// Version tag of an NVAPI struct: its size in the low 16 bits, the version above.
const fn versioned<T>(version: u32) -> u32 {
    std::mem::size_of::<T>() as u32 | (version << 16)
}

/// In/out argument of GPU_ThermalGetSensors: `mask` selects the slots to fill; temperatures
/// are in 1/256 °C.
#[repr(C)]
#[derive(Clone, Copy)]
struct ThermalSensors {
    version: u32,
    mask: u32,
    reserved: [i32; 8],
    temperatures: [i32; 32],
}
const _: () = assert!(std::mem::size_of::<ThermalSensors>() == 168);
const THERMAL_SENSORS_V2: u32 = versioned::<ThermalSensors>(2);
const _: () = assert!(THERMAL_SENSORS_V2 == 0x0002_00A8);

/// Out argument of GPU_ClientVoltRailsGetStatus: the core rail in µV at offset 0x28.
#[repr(C)]
#[derive(Clone, Copy)]
struct VoltRailsStatus {
    version: u32,
    reserved: [u32; 9],
    core_microvolts: u32,
    reserved_tail: [u32; 8],
}
const _: () = assert!(std::mem::size_of::<VoltRailsStatus>() == 0x4C);
const _: () = assert!(std::mem::offset_of!(VoltRailsStatus, core_microvolts) == 0x28);
const VOLT_RAILS_STATUS_V1: u32 = versioned::<VoltRailsStatus>(1);
const _: () = assert!(VOLT_RAILS_STATUS_V1 == 0x0001_004C);

type QueryInterfaceFn = unsafe extern "C" fn(u32) -> *mut c_void;
type InitializeFn = unsafe extern "C" fn() -> Status;
type EnumPhysicalGpusFn = unsafe extern "C" fn(*mut GpuHandle, *mut u32) -> Status;
type GpuU32Fn = unsafe extern "C" fn(GpuHandle, *mut u32) -> Status;
type ThermalGetSensorsFn = unsafe extern "C" fn(GpuHandle, *mut ThermalSensors) -> Status;
type VoltRailsGetStatusFn = unsafe extern "C" fn(GpuHandle, *mut VoltRailsStatus) -> Status;

/// Slots of the ThermalGetSensors array that hold the experimental sensors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThermalSlots {
    hotspot: Option<usize>,
    junction: Option<usize>,
}

/// Slots by GPU generation (PCI device id), kept only when `mask` covers them.
fn thermal_slots(pci_device_id: u32, mask: u32) -> ThermalSlots {
    let (hotspot, junction) = match pci_device_id {
        0x2680..=0x28FF => (Some(1), Some(7)), // Ada Lovelace (RTX 40)
        0x2B80..=0x2FFF => (None, Some(2)),    // Blackwell (RTX 50): no hotspot
        0x1E00..=0x25FF => (Some(1), Some(9)), // Turing and Ampere (RTX 20/30)
        _ => (None, None),
    };
    let covered = |slot: usize| (mask >> slot) & 1 == 1;
    ThermalSlots {
        hotspot: hotspot.filter(|&s| covered(s)),
        junction: junction.filter(|&s| covered(s)),
    }
}

/// Widest contiguous mask the driver accepts: grows one slot at a time until `call(1 << bit)`
/// fails (0xFF on an RTX 4080 with driver 617.14).
fn probe_mask(mut call: impl FnMut(u32) -> Status) -> u32 {
    let mut mask = 0u32;
    for bit in 0..32 {
        if call(1 << bit) != OK {
            break;
        }
        mask |= 1 << bit;
    }
    mask
}

/// 1/256 °C to °C; zero or negative raw values mean "no reading".
fn celsius(raw: i32) -> Option<f64> {
    (raw > 0).then(|| f64::from(raw) / 256.0)
}

fn volts(microvolts: u32) -> Option<f64> {
    (microvolts > 0).then(|| f64::from(microvolts) / 1e6)
}

/// OK → the call produced data; HANDLE_INVALIDATED → the binding is stale; anything else →
/// the fields of this call are missing this tick.
fn usable(status: Status) -> Result<bool, ProviderError> {
    match status {
        OK => Ok(true),
        HANDLE_INVALIDATED => Err(ProviderError::Rediscover),
        _ => Ok(false),
    }
}

/// Hotspot and memory-junction readings out of a ThermalGetSensors array.
fn thermal_readings(temperatures: &[i32; 32], slots: ThermalSlots) -> Readings {
    let mut readings = Readings::new();
    for (field, slot) in [
        (GpuField::TemperatureHotspot, slots.hotspot),
        (GpuField::TemperatureMemory, slots.junction),
    ] {
        if let Some(value) = slot.and_then(|s| celsius(temperatures[s])) {
            readings.insert(field, value);
        }
    }
    readings
}

/// Index of the NVAPI GPU whose (bus, slot) equals the adapter's PCI bus and device.
fn match_gpu(ids: &[(u32, u32)], pci: PciAddress) -> Option<usize> {
    ids.iter()
        .position(|&(bus, slot)| bus == pci.bus && slot == pci.device)
}

/// The resolved NVAPI entry points.
struct Api {
    enum_physical_gpus: EnumPhysicalGpusFn,
    bus_id: GpuU32Fn,
    bus_slot_id: GpuU32Fn,
    thermal_get_sensors: Option<ThermalGetSensorsFn>,
    volt_rails_get_status: Option<VoltRailsGetStatusFn>,
}

/// Resolves one interface through nvapi_QueryInterface.
///
/// # Safety
/// `F` must be the exact `unsafe extern "C"` fn-pointer type of interface `id`.
unsafe fn query<F: Copy>(query_interface: QueryInterfaceFn, id: u32) -> Option<F> {
    const { assert!(std::mem::size_of::<F>() == std::mem::size_of::<*mut c_void>()) };
    // SAFETY: nvapi_QueryInterface takes any id and returns NULL for unknown ones.
    let pointer = unsafe { query_interface(id) };
    // SAFETY: a non-null result is the entry point of `id`, whose type the caller guarantees.
    (!pointer.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&pointer) })
}

impl Api {
    fn gpus(&self) -> Vec<GpuHandle> {
        let mut handles = [std::ptr::null_mut(); MAX_PHYSICAL_GPUS];
        let mut count = 0u32;
        // SAFETY: `handles` has room for the documented maximum; `count` is a valid out pointer.
        let status = unsafe { (self.enum_physical_gpus)(handles.as_mut_ptr(), &mut count) };
        if status != OK {
            return Vec::new();
        }
        handles[..(count as usize).min(MAX_PHYSICAL_GPUS)].to_vec()
    }

    /// (bus, slot) of `gpu`; `None` if either call fails.
    fn location(&self, gpu: GpuHandle) -> Option<(u32, u32)> {
        let (mut bus, mut slot) = (0u32, 0u32);
        // SAFETY: `gpu` comes from EnumPhysicalGPUs; the out pointers are valid.
        let ok = unsafe {
            (self.bus_id)(gpu, &mut bus) == OK && (self.bus_slot_id)(gpu, &mut slot) == OK
        };
        ok.then_some((bus, slot))
    }

    fn thermal(&self, gpu: GpuHandle, mask: u32) -> (Status, [i32; 32]) {
        let Some(f) = self.thermal_get_sensors else {
            return (-1, [0; 32]);
        };
        let mut sensors = ThermalSensors {
            version: THERMAL_SENSORS_V2,
            mask,
            reserved: [0; 8],
            temperatures: [0; 32],
        };
        // SAFETY: `sensors` is a valid, versioned in/out struct of the layout the call expects.
        let status = unsafe { f(gpu, &mut sensors) };
        (status, sensors.temperatures)
    }

    fn core_microvolts(&self, gpu: GpuHandle) -> (Status, u32) {
        let Some(f) = self.volt_rails_get_status else {
            return (-1, 0);
        };
        let mut rails = VoltRailsStatus {
            version: VOLT_RAILS_STATUS_V1,
            reserved: [0; 9],
            core_microvolts: 0,
            reserved_tail: [0; 8],
        };
        // SAFETY: `rails` is a valid, versioned out struct of the layout the call expects.
        let status = unsafe { f(gpu, &mut rails) };
        (status, rails.core_microvolts)
    }
}

/// An adapter bound to its NVAPI GPU.
struct Bound {
    gpu: GpuHandle,
    mask: u32,
    slots: ThermalSlots,
    fields: BTreeSet<GpuField>,
}

/// NVAPI enrichment layer: only the experimental fields NVML does not have.
pub(crate) struct NvapiLayer {
    api: Api,
    bound: Vec<Option<Bound>>,
    /// Keeps nvapi64.dll referenced; it is never unloaded (D1).
    _library: Library,
}

// SAFETY: NVAPI physical-GPU handles are process-wide and not tied to a thread (the spike
// initialised, polled and released NVAPI on three different threads). The layer is used by
// one thread at a time.
unsafe impl Send for NvapiLayer {}

impl NvapiLayer {
    /// Loads nvapi64.dll from System32 and initialises NVAPI; `None` when the DLL, a required
    /// interface or NvAPI_Initialize fails. Never calls NvAPI_Unload (D1).
    pub(crate) fn load() -> Option<Self> {
        let library = match Library::system32("nvapi64.dll") {
            Ok(library) => library,
            Err(e) => {
                tracing::debug!(error = %e, "nvapi64.dll not available");
                return None;
            }
        };
        // SAFETY: nvapi_QueryInterface takes a u32 id and returns a pointer (NULL if unknown).
        let query_interface: QueryInterfaceFn = unsafe { library.symbol(c"nvapi_QueryInterface") }?;
        // SAFETY (all `query` calls): each type alias is the signature of the interface id.
        let (initialize, api) = unsafe {
            let initialize: Option<InitializeFn> = query(query_interface, ID_INITIALIZE);
            let api = Api {
                enum_physical_gpus: query(query_interface, ID_ENUM_PHYSICAL_GPUS)?,
                bus_id: query(query_interface, ID_GPU_GET_BUS_ID)?,
                bus_slot_id: query(query_interface, ID_GPU_GET_BUS_SLOT_ID)?,
                thermal_get_sensors: query(query_interface, ID_GPU_THERMAL_GET_SENSORS),
                volt_rails_get_status: query(query_interface, ID_GPU_CLIENT_VOLT_RAILS_GET_STATUS),
            };
            (initialize?, api)
        };
        // SAFETY: NvAPI_Initialize takes no arguments.
        let status = unsafe { initialize() };
        if status != OK {
            tracing::warn!(status, "NvAPI_Initialize failed; NVAPI disabled");
            return None;
        }
        Some(Self {
            api,
            bound: Vec::new(),
            _library: library,
        })
    }

    fn bind(&self, adapter: &Adapter, gpus: &[GpuHandle], ids: &[(u32, u32)]) -> Option<Bound> {
        if adapter.vendor() != Some(Vendor::Nvidia) {
            return None;
        }
        let gpu = gpus[match_gpu(ids, adapter.pci?)?];
        let mask = if self.api.thermal_get_sensors.is_some() {
            probe_mask(|mask| self.api.thermal(gpu, mask).0)
        } else {
            0
        };
        let slots = thermal_slots(adapter.device_id, mask);
        let mut fields = BTreeSet::new();
        if mask != 0 {
            let (status, temperatures) = self.api.thermal(gpu, mask);
            if status == OK {
                fields.extend(thermal_readings(&temperatures, slots).into_keys());
            }
        }
        let (status, microvolts) = self.api.core_microvolts(gpu);
        if status == OK && volts(microvolts).is_some() {
            fields.insert(GpuField::VoltageCore);
        }
        Some(Bound {
            gpu,
            mask,
            slots,
            fields,
        })
    }

    fn read(&self, bound: &Bound) -> Result<Readings, ProviderError> {
        let mut readings = Readings::new();
        if bound.fields.contains(&GpuField::TemperatureHotspot)
            || bound.fields.contains(&GpuField::TemperatureMemory)
        {
            let (status, temperatures) = self.api.thermal(bound.gpu, bound.mask);
            if usable(status)? {
                readings.extend(
                    thermal_readings(&temperatures, bound.slots)
                        .into_iter()
                        .filter(|(field, _)| bound.fields.contains(field)),
                );
            }
        }
        if bound.fields.contains(&GpuField::VoltageCore) {
            let (status, microvolts) = self.api.core_microvolts(bound.gpu);
            if usable(status)? {
                if let Some(value) = volts(microvolts) {
                    readings.insert(GpuField::VoltageCore, value);
                }
            }
        }
        Ok(readings)
    }
}

impl GpuLayer for NvapiLayer {
    fn source(&self) -> Source {
        Source::Nvapi
    }

    fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>> {
        let gpus = self.api.gpus();
        let ids: Vec<(u32, u32)> = gpus
            .iter()
            .map(|&gpu| self.api.location(gpu).unwrap_or((u32::MAX, u32::MAX)))
            .collect();
        self.bound = adapters
            .iter()
            .map(|adapter| self.bind(adapter, &gpus, &ids))
            .collect();
        self.bound
            .iter()
            .map(|bound| bound.as_ref().map(|b| b.fields.clone()).unwrap_or_default())
            .collect()
    }

    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
        self.bound
            .iter()
            .map(|bound| match bound {
                Some(b) => self.read(b),
                None => Ok(Readings::new()),
            })
            .collect()
    }

    fn is_experimental(&self, field: GpuField) -> bool {
        matches!(
            field,
            GpuField::TemperatureHotspot | GpuField::TemperatureMemory | GpuField::VoltageCore
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_slots_by_architecture() {
        // RTX 4080 (AD103) with the mask probed on driver 617.14.
        assert_eq!(
            thermal_slots(0x2704, 0xFF),
            ThermalSlots {
                hotspot: Some(1),
                junction: Some(7)
            }
        );
        // RTX 5090 (GB202): no hotspot.
        assert_eq!(
            thermal_slots(0x2B85, 0xFF),
            ThermalSlots {
                hotspot: None,
                junction: Some(2)
            }
        );
        // RTX 3090 (GA102): junction only with a mask reaching slot 9.
        assert_eq!(
            thermal_slots(0x2204, 0xFF),
            ThermalSlots {
                hotspot: Some(1),
                junction: None
            }
        );
        assert_eq!(
            thermal_slots(0x2204, 0x3FF),
            ThermalSlots {
                hotspot: Some(1),
                junction: Some(9)
            }
        );
        // RTX 2080 Ti (TU102).
        assert_eq!(
            thermal_slots(0x1E04, 0x3FF),
            ThermalSlots {
                hotspot: Some(1),
                junction: Some(9)
            }
        );
        // GTX 1080 (Pascal) and unknown ids: nothing.
        assert_eq!(
            thermal_slots(0x1B80, 0xFFFF),
            ThermalSlots {
                hotspot: None,
                junction: None
            }
        );
        // A mask that does not cover a slot drops it.
        assert_eq!(
            thermal_slots(0x2704, 0x1),
            ThermalSlots {
                hotspot: None,
                junction: None
            }
        );
    }

    #[test]
    fn mask_probe_stops_at_the_first_refused_slot() {
        assert_eq!(probe_mask(|mask| if mask < 1 << 8 { OK } else { -1 }), 0xFF);
        assert_eq!(probe_mask(|_| -1), 0);
        assert_eq!(probe_mask(|_| OK), u32::MAX);
        let mut calls = Vec::new();
        probe_mask(|mask| {
            calls.push(mask);
            if mask <= 4 {
                OK
            } else {
                -104
            }
        });
        assert_eq!(calls, [1, 2, 4, 8]);
    }

    #[test]
    fn values_are_scaled_and_zero_is_missing() {
        assert_eq!(celsius(13_920), Some(54.375));
        assert_eq!(celsius(0), None);
        assert_eq!(celsius(-256), None);
        assert_eq!(volts(930_000), Some(0.93));
        assert_eq!(volts(0), None);
    }

    #[test]
    fn thermal_array_maps_to_fields() {
        // Raw array read from the RTX 4080 at idle during the spike.
        let mut temperatures = [0i32; 32];
        temperatures[..8].copy_from_slice(&[
            11_210, 13_920, 12_208, 11_992, 12_128, 13_920, 12_288, 12_800,
        ]);
        let readings = thermal_readings(&temperatures, thermal_slots(0x2704, 0xFF));
        assert_eq!(
            readings,
            Readings::from([
                (GpuField::TemperatureHotspot, 54.375),
                (GpuField::TemperatureMemory, 50.0)
            ])
        );
        temperatures[7] = 0;
        let readings = thermal_readings(&temperatures, thermal_slots(0x2704, 0xFF));
        assert_eq!(
            readings,
            Readings::from([(GpuField::TemperatureHotspot, 54.375)])
        );
    }

    #[test]
    fn invalidated_handle_requests_rediscover() {
        assert_eq!(usable(OK), Ok(true));
        assert_eq!(usable(HANDLE_INVALIDATED), Err(ProviderError::Rediscover));
        assert_eq!(usable(-104), Ok(false)); // NOT_SUPPORTED
        assert_eq!(usable(-1), Ok(false));
    }

    #[test]
    fn gpus_match_by_bus_and_slot() {
        let ids = [(0x11, 0), (1, 0)];
        assert_eq!(
            match_gpu(
                &ids,
                PciAddress {
                    bus: 1,
                    device: 0,
                    function: 0
                }
            ),
            Some(1)
        );
        assert_eq!(
            match_gpu(
                &ids,
                PciAddress {
                    bus: 2,
                    device: 0,
                    function: 0
                }
            ),
            None
        );
        assert_eq!(
            match_gpu(
                &[(u32::MAX, u32::MAX)],
                PciAddress {
                    bus: 1,
                    device: 0,
                    function: 0
                }
            ),
            None
        );
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn nvapi_reads_experimental_sensors_of_the_rtx_4080() {
        let adapters = crate::gpu::enumerate::enumerate().expect("enumerate");
        let nvidia = adapters
            .iter()
            .position(|a| a.vendor() == Some(Vendor::Nvidia))
            .expect("an NVIDIA adapter");
        let mut layer = NvapiLayer::load().expect("NVAPI loads");
        let supported = layer.attach(&adapters);
        assert_eq!(supported.len(), adapters.len());
        let all: BTreeSet<GpuField> = [
            GpuField::TemperatureHotspot,
            GpuField::TemperatureMemory,
            GpuField::VoltageCore,
        ]
        .into();
        for (i, set) in supported.iter().enumerate() {
            if i != nvidia {
                assert!(set.is_empty(), "NVAPI must not bind {}", adapters[i].name);
            }
        }
        let set = &supported[nvidia];
        println!("NVAPI fields: {set:?}");
        assert!(set.is_subset(&all), "{set:?}");
        // On Ada (RTX 40, e.g. the RTX 4080 at 0x2704) all three are expected;
        // other generations expose only what their slot table covers.
        if (0x2680..=0x28FF).contains(&adapters[nvidia].device_id) {
            assert_eq!(*set, all);
        }
        assert!(all.iter().all(|&field| layer.is_experimental(field)));
        let readings = layer.sample().expect("sample");
        let r = &readings[nvidia];
        println!("NVAPI readings: {r:?}");
        for field in [GpuField::TemperatureHotspot, GpuField::TemperatureMemory] {
            if let Some(&celsius) = r.get(&field) {
                assert!((20.0..=110.0).contains(&celsius), "{field:?} {celsius}");
            }
        }
        if let Some(&voltage) = r.get(&GpuField::VoltageCore) {
            assert!((0.5..=1.3).contains(&voltage), "voltage {voltage}");
        }
        assert_eq!(r.keys().copied().collect::<BTreeSet<_>>(), *set);
    }
}
