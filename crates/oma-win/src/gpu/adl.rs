//! AMD GPU telemetry from the ADL runtime (`atiadlxx.dll`, installed by the AMD driver).
//!
//! Interface declarations for interoperability with AMD's atiadlxx.dll, written from AMD's
//! public ADL API documentation; no AMD header is included. Identifiers and comments are our
//! own: only the exported symbol names, argument order, struct layouts and numeric sensor ids
//! come from the DLL's interface.
//!
//! The DLL is loaded from System32 only, and the DLL and its ADL context are kept for the
//! process lifetime (decision D1: `FreeLibrary` does not give ADL's memory back). Every call
//! happens on the GPU worker thread that owns the layer.
//!
//! Data path: the PMLog shared-memory log (`ADL2_Overdrive8_PMLog_ShareMemory_*`, about 3 µs
//! per read), falling back to `ADL2_New_QueryPMLogData_Get` (about 365 µs per read). Both fill
//! a 256-slot table indexed by PMLog sensor id.
//!
//! Verified on a Raphael iGPU (driver 7.25.10.1590): ADL lists one logical adapter per display
//! output (5 for the AMD GPU) and also lists the NVIDIA card (4 entries); the vendor field holds
//! the PCI vendor hex digits read as decimal (AMD 1002, NVIDIA 10). On a desktop APU the GFX
//! power, ASIC power and GFX voltage sensors follow the CPU, so an integrated GPU only gets the
//! GFX temperature and the GFX clock. The discrete Radeon mapping follows LibreHardwareMonitor's
//! usage and has NOT been verified on real hardware.
//!
//! x86_64: `extern "system"` is the C convention there; C `int` is `i32`.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use oma_core::model::Source;
use oma_core::provider::ProviderError;

use super::adapter::{Adapter, PciAddress, Vendor};
use super::field::GpuField;
use super::layer::{GpuLayer, Readings};
use crate::dynlib::Library;

/// PCI vendor 0x1002 as ADL reports it: the hex digits parsed as a decimal number.
const AMD_VENDOR_DECIMAL: i32 = 1002;
/// Bit of the ASIC family type mask that marks an integrated GPU.
const ASIC_INTEGRATED: i32 = 1 << 1;
/// Length of every fixed string in the adapter record.
const TEXT_LEN: usize = 256;
/// Slots of the PMLog output table (one per sensor id).
const SENSOR_SLOTS: usize = 256;
/// Sampling period asked of the shared-memory log, in milliseconds.
const LOG_PERIOD_MS: i32 = 1000;

// PMLog sensor ids used here (index into the output table).
const SENSOR_GFX_CLOCK: usize = 1;
const SENSOR_MEMORY_CLOCK: usize = 2;
const SENSOR_EDGE_TEMPERATURE: usize = 8;
const SENSOR_MEMORY_TEMPERATURE: usize = 9;
const SENSOR_FAN_RPM: usize = 14;
const SENSOR_FAN_PERCENT: usize = 15;
const SENSOR_GFX_VOLTAGE_MV: usize = 21;
const SENSOR_ASIC_POWER: usize = 23;
const SENSOR_HOTSPOT_TEMPERATURE: usize = 27;
const SENSOR_GFX_TEMPERATURE: usize = 28;
const SENSOR_BOARD_POWER: usize = 73;

/// Fields read from an integrated GPU, with their candidate sensors in preference order.
/// Power and voltage are left out on purpose: on a desktop APU they track the CPU.
const INTEGRATED_SENSORS: &[(GpuField, &[usize])] = &[
    (GpuField::TemperatureCore, &[SENSOR_GFX_TEMPERATURE]),
    (GpuField::ClockCore, &[SENSOR_GFX_CLOCK]),
];

/// Fields read from a discrete GPU (LibreHardwareMonitor's mapping, unverified here).
const DISCRETE_SENSORS: &[(GpuField, &[usize])] = &[
    (GpuField::TemperatureCore, &[SENSOR_EDGE_TEMPERATURE]),
    (GpuField::TemperatureHotspot, &[SENSOR_HOTSPOT_TEMPERATURE]),
    (GpuField::TemperatureMemory, &[SENSOR_MEMORY_TEMPERATURE]),
    (GpuField::ClockCore, &[SENSOR_GFX_CLOCK]),
    (GpuField::ClockMemory, &[SENSOR_MEMORY_CLOCK]),
    (
        GpuField::PowerBoard,
        &[SENSOR_BOARD_POWER, SENSOR_ASIC_POWER],
    ),
    (GpuField::FanRpm, &[SENSOR_FAN_RPM]),
    (GpuField::FanPercent, &[SENSOR_FAN_PERCENT]),
    (GpuField::VoltageCore, &[SENSOR_GFX_VOLTAGE_MV]),
];

/// One logical adapter as ADL describes it (Windows layout).
#[repr(C)]
#[derive(Clone, Copy)]
struct RawAdapterInfo {
    size: i32,
    index: i32,
    udid: [u8; TEXT_LEN],
    bus: i32,
    device: i32,
    function: i32,
    vendor: i32,
    adapter_name: [u8; TEXT_LEN],
    display_name: [u8; TEXT_LEN],
    present: i32,
    exists: i32,
    driver_path: [u8; TEXT_LEN],
    driver_path_ext: [u8; TEXT_LEN],
    pnp: [u8; TEXT_LEN],
    os_display_index: i32,
}
const _: () = assert!(size_of::<RawAdapterInfo>() == 1572);

impl RawAdapterInfo {
    fn zeroed() -> Self {
        // SAFETY: integers and byte arrays only; all-zero is a valid value.
        let mut info: Self = unsafe { std::mem::zeroed() };
        info.size = size_of::<Self>() as i32;
        info
    }
}

/// One slot of the PMLog output table.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct SensorSlot {
    supported: i32,
    value: i32,
}
const _: () = assert!(size_of::<SensorSlot>() == 8);

/// PMLog output: a size header and one slot per sensor id.
#[repr(C)]
#[derive(Clone, Copy)]
struct SensorTable {
    size: i32,
    slots: [SensorSlot; SENSOR_SLOTS],
}
const _: () = assert!(size_of::<SensorTable>() == 2052);

impl SensorTable {
    fn new() -> Self {
        Self {
            size: size_of::<Self>() as i32,
            slots: [SensorSlot::default(); SENSOR_SLOTS],
        }
    }
}

type Context = *mut c_void;
type AllocFn = unsafe extern "system" fn(i32) -> *mut c_void;
type CreateFn = unsafe extern "system" fn(AllocFn, i32, *mut Context) -> i32;
type AdapterCountFn = unsafe extern "system" fn(Context, *mut i32) -> i32;
type AdapterInfoFn = unsafe extern "system" fn(Context, *mut RawAdapterInfo, i32) -> i32;
type AsicFamilyFn = unsafe extern "system" fn(Context, i32, *mut i32, *mut i32) -> i32;
type QueryLogFn = unsafe extern "system" fn(Context, i32, *mut SensorTable) -> i32;
type SharedSupportFn = unsafe extern "system" fn(Context, i32, *mut i32, i32) -> i32;
type LogDeviceCreateFn = unsafe extern "system" fn(Context, i32, *mut u32) -> i32;
type LogDeviceDestroyFn = unsafe extern "system" fn(Context, u32) -> i32;
type SensorListFn = unsafe extern "system" fn(Context, i32, *mut i32, *mut *mut i32) -> i32;
type SharedStartFn = unsafe extern "system" fn(
    Context,
    i32,
    i32,
    i32,
    *mut i32,
    *mut u32,
    *mut *mut c_void,
    i32,
) -> i32;
type SharedReadFn = unsafe extern "system" fn(
    Context,
    i32,
    i32,
    *mut i32,
    *mut *mut c_void,
    *mut SensorTable,
) -> i32;
type SharedStopFn = unsafe extern "system" fn(Context, i32, *mut u32) -> i32;

/// ADL success codes are 0..=4; negative values are errors (-8 = not supported).
fn succeeded(rc: i32) -> bool {
    rc >= 0
}

/// Size of the bookkeeping header in front of every buffer handed to ADL.
const ALLOC_HEADER: usize = 16;

/// Allocator ADL uses for buffers it returns to us (e.g. the sensor list). The total size is
/// stored in a 16-byte header so `adl_free` can rebuild the layout. Never unwinds.
unsafe extern "system" fn adl_alloc(size: i32) -> *mut c_void {
    let Ok(size) = usize::try_from(size) else {
        return null_mut();
    };
    if size == 0 {
        return null_mut();
    }
    let Ok(layout) = Layout::from_size_align(size + ALLOC_HEADER, ALLOC_HEADER) else {
        return null_mut();
    };
    // SAFETY: the layout has a non-zero size.
    let base = unsafe { alloc_zeroed(layout) };
    if base.is_null() {
        return null_mut();
    }
    // SAFETY: `base` is valid for `layout.size()` bytes and 16-byte aligned, so the header
    // write is in bounds and aligned, and `base + 16` stays inside the allocation.
    unsafe {
        base.cast::<usize>().write(layout.size());
        base.add(ALLOC_HEADER).cast()
    }
}

/// Frees a buffer allocated by `adl_alloc`.
///
/// # Safety
/// `ptr` must be null or a pointer returned by `adl_alloc` that was not freed yet.
unsafe fn adl_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: per the contract `ptr` came from `adl_alloc`, so the header sits 16 bytes
    // before it and holds the size of the whole allocation made with 16-byte alignment.
    unsafe {
        let base = ptr.cast::<u8>().sub(ALLOC_HEADER);
        let total = base.cast::<usize>().read();
        dealloc(base, Layout::from_size_align_unchecked(total, ALLOC_HEADER));
    }
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn is_amd(info: &RawAdapterInfo) -> bool {
    info.vendor == AMD_VENDOR_DECIMAL || text(&info.pnp).to_ascii_uppercase().contains("VEN_1002")
}

/// Physical AMD GPUs among ADL's logical adapters: other vendors are dropped and the
/// per-output duplicates are merged on the PCI address. Value = first ADL adapter index.
fn physical_adapters(infos: &[RawAdapterInfo]) -> BTreeMap<PciAddress, i32> {
    let mut physical = BTreeMap::new();
    for info in infos.iter().filter(|info| is_amd(info)) {
        let (Ok(bus), Ok(device), Ok(function)) = (
            u32::try_from(info.bus),
            u32::try_from(info.device),
            u32::try_from(info.function),
        ) else {
            continue;
        };
        let pci = PciAddress {
            bus,
            device,
            function,
        };
        physical.entry(pci).or_insert(info.index);
    }
    physical
}

/// Integrated if the adapter list says so or ADL's ASIC family mask (types & valid bits) does:
/// treating an APU as discrete would publish power/voltage values that belong to the CPU.
fn is_integrated(adapter_integrated: bool, asic_mask: Option<i32>) -> bool {
    adapter_integrated || asic_mask.is_some_and(|mask| mask & ASIC_INTEGRATED != 0)
}

/// For each field of the GPU class, the first candidate sensor that the driver lists (when a
/// list is known) and marks as supported in the first table read.
fn choose_sensors(
    table: &SensorTable,
    listed: Option<&[i32]>,
    integrated: bool,
) -> Vec<(GpuField, usize)> {
    let candidates = if integrated {
        INTEGRATED_SENSORS
    } else {
        DISCRETE_SENSORS
    };
    candidates
        .iter()
        .filter_map(|&(field, sensors)| {
            sensors
                .iter()
                .copied()
                .find(|&id| {
                    let in_list = listed.is_none_or(|list| list.contains(&(id as i32)));
                    in_list && table.slots[id].supported != 0
                })
                .map(|id| (field, id))
        })
        .collect()
}

/// Converts a raw PMLog value (MHz, °C, W, RPM, %, mV) to the field's unit.
fn convert(field: GpuField, raw: i32) -> Option<f64> {
    match field {
        GpuField::TemperatureCore | GpuField::TemperatureHotspot | GpuField::TemperatureMemory => {
            (raw > 0).then_some(f64::from(raw))
        }
        GpuField::VoltageCore => (raw > 0).then(|| f64::from(raw) / 1000.0),
        _ => (raw >= 0).then_some(f64::from(raw)),
    }
}

fn readings_from(table: &SensorTable, chosen: &[(GpuField, usize)]) -> Readings {
    chosen
        .iter()
        .filter(|&&(_, id)| table.slots[id].supported != 0)
        .filter_map(|&(field, id)| convert(field, table.slots[id].value).map(|v| (field, v)))
        .collect()
}

/// Entry points of the shared-memory log; all present or the path is not used.
struct SharedApi {
    support: SharedSupportFn,
    device_create: LogDeviceCreateFn,
    device_destroy: LogDeviceDestroyFn,
    sensor_list: SensorListFn,
    start: SharedStartFn,
    read: SharedReadFn,
    stop: SharedStopFn,
}

struct Api {
    adapter_count: AdapterCountFn,
    adapter_info: AdapterInfoFn,
    asic_family: Option<AsicFamilyFn>,
    query: Option<QueryLogFn>,
    shared: Option<SharedApi>,
}

/// A running shared-memory log for one GPU.
struct SharedLog {
    device: u32,
    memory: *mut c_void,
    /// The driver's own sensor list: reads are positional, so a list of our own would
    /// return the wrong sensors.
    sensors: Vec<i32>,
}

enum Feed {
    Shared(SharedLog),
    Query,
}

/// An attached AMD GPU.
struct Bound {
    index: i32,
    feed: Feed,
    chosen: Vec<(GpuField, usize)>,
    warned: bool,
}

pub(crate) struct AdlLayer {
    _library: Library,
    api: Api,
    context: Context,
    bound: Vec<Option<Bound>>,
}

// SAFETY: the ADL context and the shared-memory pointers are process-wide objects of the
// driver, not tied to the creating thread (the spike read the log from a second thread with
// the same results). The layer is used by one thread at a time (the GPU worker owns it).
unsafe impl Send for AdlLayer {}

impl AdlLayer {
    /// Loads `atiadlxx.dll` from System32 and creates the ADL context. None when the DLL is
    /// absent (no AMD driver), a required export is missing or the context cannot be created.
    pub(crate) fn load() -> Option<Self> {
        let library = match Library::system32("atiadlxx.dll") {
            Ok(library) => library,
            Err(e) => {
                tracing::debug!(error = %e, "ADL runtime not available");
                return None;
            }
        };
        // SAFETY: each requested type is the exact prototype of the named export.
        let create: CreateFn = unsafe { library.symbol(c"ADL2_Main_Control_Create") }?;
        // SAFETY: as above.
        let api = unsafe { resolve(&library) }?;
        let mut context: Context = null_mut();
        // SAFETY: `adl_alloc` matches the allocator prototype and never unwinds; `context`
        // is a valid out-pointer. 1 = enumerate connected adapters only.
        let rc = unsafe { create(adl_alloc, 1, &mut context) };
        if !succeeded(rc) || context.is_null() {
            tracing::warn!(
                rc,
                "ADL2_Main_Control_Create failed; AMD telemetry disabled"
            );
            return None;
        }
        Some(Self {
            _library: library,
            api,
            context,
            bound: Vec::new(),
        })
    }

    /// Every logical adapter ADL reports (all vendors, duplicates included).
    fn adapter_infos(&self) -> Vec<RawAdapterInfo> {
        let mut count = 0;
        // SAFETY: valid context and out-pointer.
        let rc = unsafe { (self.api.adapter_count)(self.context, &mut count) };
        let Ok(count) = usize::try_from(count) else {
            return Vec::new();
        };
        if !succeeded(rc) || count == 0 {
            return Vec::new();
        }
        let mut infos = vec![RawAdapterInfo::zeroed(); count];
        let bytes = (count * size_of::<RawAdapterInfo>()) as i32;
        // SAFETY: the buffer holds `count` records, `bytes` is its exact size in bytes and each
        // record carries its own size as ADL expects.
        let rc = unsafe { (self.api.adapter_info)(self.context, infos.as_mut_ptr(), bytes) };
        if succeeded(rc) {
            infos
        } else {
            Vec::new()
        }
    }

    fn asic_mask(&self, index: i32) -> Option<i32> {
        let family = self.api.asic_family?;
        let (mut types, mut valid) = (0, 0);
        // SAFETY: valid context and out-pointers.
        let rc = unsafe { family(self.context, index, &mut types, &mut valid) };
        succeeded(rc).then_some(types & valid)
    }

    fn bind(&self, adapter: &Adapter, physical: &BTreeMap<PciAddress, i32>) -> Option<Bound> {
        if adapter.vendor() != Some(Vendor::Amd) {
            return None;
        }
        let index = *physical.get(&adapter.pci?)?;
        let integrated = is_integrated(adapter.integrated, self.asic_mask(index));
        let (feed, table) = match self.start_shared(index) {
            Some((log, table)) => (Feed::Shared(log), table),
            None => (Feed::Query, self.query(index)?),
        };
        let listed = match &feed {
            Feed::Shared(log) => Some(log.sensors.as_slice()),
            Feed::Query => None,
        };
        let chosen = choose_sensors(&table, listed, integrated);
        let mut bound = Bound {
            index,
            feed,
            chosen,
            warned: false,
        };
        if bound.chosen.is_empty() {
            self.stop(&mut bound);
            return None;
        }
        Some(bound)
    }

    /// Starts the shared-memory log and reads it once. None when unsupported or failing.
    fn start_shared(&self, index: i32) -> Option<(SharedLog, SensorTable)> {
        let api = self.api.shared.as_ref()?;
        let mut supported = 0;
        // SAFETY: valid context and out-pointer; 0 = default option.
        let rc = unsafe { (api.support)(self.context, index, &mut supported, 0) };
        if !succeeded(rc) || supported == 0 {
            return None;
        }
        let mut device = 0u32;
        // SAFETY: valid context and out-pointer.
        if !succeeded(unsafe { (api.device_create)(self.context, index, &mut device) }) {
            return None;
        }
        let mut count = 0;
        let mut list: *mut i32 = null_mut();
        // SAFETY: valid context and out-pointers; ADL allocates `list` through `adl_alloc`.
        let rc = unsafe { (api.sensor_list)(self.context, index, &mut count, &mut list) };
        let sensors = match usize::try_from(count) {
            Ok(count) if succeeded(rc) && !list.is_null() && count > 0 => {
                // SAFETY: on success ADL wrote `count` ids to the buffer it allocated.
                unsafe { std::slice::from_raw_parts(list, count) }.to_vec()
            }
            _ => Vec::new(),
        };
        // SAFETY: `list` is null or was allocated by `adl_alloc` and is not used again.
        unsafe { adl_free(list.cast()) };
        if sensors.is_empty() {
            // SAFETY: `device` was created above.
            unsafe { (api.device_destroy)(self.context, device) };
            return None;
        }
        let mut memory: *mut c_void = null_mut();
        // SAFETY: valid context and out-pointers; -1 with a null list = log every sensor.
        let rc = unsafe {
            (api.start)(
                self.context,
                index,
                LOG_PERIOD_MS,
                -1,
                null_mut(),
                &mut device,
                &mut memory,
                0,
            )
        };
        if !succeeded(rc) || memory.is_null() {
            // SAFETY: `device` was created above.
            unsafe { (api.device_destroy)(self.context, device) };
            return None;
        }
        let mut log = SharedLog {
            device,
            memory,
            sensors,
        };
        let mut table = SensorTable::new();
        if !read_shared(api, self.context, index, &mut log, &mut table) {
            stop_shared(api, self.context, index, &mut log);
            return None;
        }
        Some((log, table))
    }

    fn query(&self, index: i32) -> Option<SensorTable> {
        let query = self.api.query?;
        let mut table = SensorTable::new();
        // SAFETY: valid context; `table` is a correctly sized output table.
        succeeded(unsafe { query(self.context, index, &mut table) }).then_some(table)
    }

    fn stop(&self, bound: &mut Bound) {
        if let (Feed::Shared(log), Some(api)) = (&mut bound.feed, self.api.shared.as_ref()) {
            stop_shared(api, self.context, bound.index, log);
        }
    }
}

fn read_shared(
    api: &SharedApi,
    context: Context,
    index: i32,
    log: &mut SharedLog,
    table: &mut SensorTable,
) -> bool {
    // SAFETY: the log was started on this context and adapter; the sensor list is the
    // driver's own and outlives the call; `table` is a correctly sized output table.
    let rc = unsafe {
        (api.read)(
            context,
            index,
            log.sensors.len() as i32,
            log.sensors.as_mut_ptr(),
            &mut log.memory,
            table,
        )
    };
    succeeded(rc)
}

fn stop_shared(api: &SharedApi, context: Context, index: i32, log: &mut SharedLog) {
    // SAFETY: the log and its device were started/created on this context and adapter.
    unsafe {
        (api.stop)(context, index, &mut log.device);
        (api.device_destroy)(context, log.device);
    }
}

/// Resolves every entry point except the context constructor.
///
/// # Safety
/// `library` must be ADL's `atiadlxx.dll` (the prototypes below are its exports').
unsafe fn resolve(library: &Library) -> Option<Api> {
    // SAFETY (whole block): each requested type is the exact prototype of the named export.
    unsafe {
        let shared = (|| {
            Some(SharedApi {
                support: library.symbol(c"ADL2_Overdrive8_PMLog_ShareMemory_Support")?,
                device_create: library.symbol(c"ADL2_Device_PMLog_Device_Create")?,
                device_destroy: library.symbol(c"ADL2_Device_PMLog_Device_Destroy")?,
                sensor_list: library.symbol(c"ADL2_Overdrive8_PMLogSenorType_Support_Get")?,
                start: library.symbol(c"ADL2_Overdrive8_PMLog_ShareMemory_Start")?,
                read: library.symbol(c"ADL2_Overdrive8_PMLog_ShareMemory_Read")?,
                stop: library.symbol(c"ADL2_Overdrive8_PMLog_ShareMemory_Stop")?,
            })
        })();
        let api = Api {
            adapter_count: library.symbol(c"ADL2_Adapter_NumberOfAdapters_Get")?,
            adapter_info: library.symbol(c"ADL2_Adapter_AdapterInfo_Get")?,
            asic_family: library.symbol(c"ADL2_Adapter_ASICFamilyType_Get"),
            query: library.symbol(c"ADL2_New_QueryPMLogData_Get"),
            shared,
        };
        (api.shared.is_some() || api.query.is_some()).then_some(api)
    }
}

impl GpuLayer for AdlLayer {
    fn source(&self) -> Source {
        Source::Adl
    }

    fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>> {
        // A re-attach replaces every binding: stop the logs of the previous one first.
        let mut previous = std::mem::take(&mut self.bound);
        for bound in previous.iter_mut().flatten() {
            self.stop(bound);
        }
        let physical = physical_adapters(&self.adapter_infos());
        let bound: Vec<Option<Bound>> = adapters
            .iter()
            .map(|adapter| self.bind(adapter, &physical))
            .collect();
        let supported = bound
            .iter()
            .map(|b| {
                b.as_ref()
                    .map(|b| b.chosen.iter().map(|&(field, _)| field).collect())
                    .unwrap_or_default()
            })
            .collect();
        self.bound = bound;
        supported
    }

    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
        let (api, context) = (&self.api, self.context);
        let readings = self
            .bound
            .iter_mut()
            .map(|slot| {
                let Some(bound) = slot else {
                    return Readings::new();
                };
                let mut table = SensorTable::new();
                let ok = match (&mut bound.feed, api.shared.as_ref(), api.query) {
                    (Feed::Shared(log), Some(shared), _) => {
                        read_shared(shared, context, bound.index, log, &mut table)
                    }
                    // SAFETY: valid context; `table` is a correctly sized output table.
                    (Feed::Query, _, Some(query)) => {
                        succeeded(unsafe { query(context, bound.index, &mut table) })
                    }
                    _ => false,
                };
                if !ok {
                    if !bound.warned {
                        tracing::warn!(index = bound.index, "ADL sensor read failed");
                        bound.warned = true;
                    }
                    return Readings::new();
                }
                bound.warned = false;
                readings_from(&table, &bound.chosen)
            })
            .collect();
        Ok(readings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(index: i32, bus: i32, vendor: i32, pnp: &str) -> RawAdapterInfo {
        let mut info = RawAdapterInfo::zeroed();
        info.index = index;
        info.bus = bus;
        info.vendor = vendor;
        info.pnp[..pnp.len()].copy_from_slice(pnp.as_bytes());
        info
    }

    fn table(values: &[(usize, i32)]) -> SensorTable {
        let mut table = SensorTable::new();
        for &(id, value) in values {
            table.slots[id] = SensorSlot {
                supported: 1,
                value,
            };
        }
        table
    }

    /// The table the Raphael iGPU returned in the spike (idle).
    fn raphael_idle() -> (SensorTable, Vec<i32>) {
        let values = [
            (1, 600),
            (2, 3000),
            (3, 1200),
            (7, 100),
            (16, 1199),
            (17, 11),
            (18, 9),
            (19, 0),
            (21, 1115),
            (23, 38),
            (30, 14),
            (31, 12),
            (35, 0),
            (28, 42),
            (29, 46),
            (40, 3),
            (41, 16),
        ];
        let listed = values.iter().map(|&(id, _)| id as i32).collect();
        (table(&values), listed)
    }

    #[test]
    fn adl_dedups_logical_adapters_and_skips_other_vendors() {
        let amd = "PCI\\VEN_1002&DEV_164E&SUBSYS_D0001458&REV_CB\\4&16012499&0&0041";
        let nvidia = "PCI\\VEN_10DE&DEV_2704&SUBSYS_51111462&REV_A1\\4&D0BDF66&0&0009";
        let mut infos: Vec<RawAdapterInfo> = (0..5).map(|i| info(i, 17, 1002, amd)).collect();
        infos.extend((5..9).map(|i| info(i, 1, 10, nvidia)));

        let physical = physical_adapters(&infos);

        let igpu = PciAddress {
            bus: 0x11,
            device: 0,
            function: 0,
        };
        assert_eq!(physical, BTreeMap::from([(igpu, 0)]));
        assert_eq!(igpu.to_string(), "0000:11:00.0");
    }

    #[test]
    fn amd_is_recognised_from_the_pnp_string_too() {
        let infos = [info(3, 3, 0, "PCI\\ven_1002&DEV_744C")];
        let physical = physical_adapters(&infos);
        assert_eq!(physical.values().copied().collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn negative_pci_numbers_are_skipped() {
        let infos = [info(0, -1, 1002, "")];
        assert!(physical_adapters(&infos).is_empty());
    }

    #[test]
    fn integrated_from_adapter_flag_or_asic_mask() {
        assert!(is_integrated(true, None));
        assert!(is_integrated(false, Some(0x22)));
        assert!(!is_integrated(false, Some(0x01)));
        assert!(!is_integrated(false, None));
    }

    #[test]
    fn integrated_gpu_gets_gfx_temperature_and_clock_only() {
        let (table, listed) = raphael_idle();
        let chosen = choose_sensors(&table, Some(&listed), true);
        assert_eq!(
            chosen,
            [
                (GpuField::TemperatureCore, SENSOR_GFX_TEMPERATURE),
                (GpuField::ClockCore, SENSOR_GFX_CLOCK)
            ]
        );
        let readings = readings_from(&table, &chosen);
        assert_eq!(
            readings,
            Readings::from([
                (GpuField::TemperatureCore, 42.0),
                (GpuField::ClockCore, 600.0)
            ])
        );
    }

    #[test]
    fn discrete_gpu_maps_every_sensor() {
        let table = table(&[
            (1, 2450),
            (2, 1250),
            (8, 55),
            (9, 70),
            (14, 1200),
            (15, 35),
            (21, 1050),
            (23, 180),
            (27, 78),
            (73, 240),
        ]);
        let chosen = choose_sensors(&table, None, false);
        let readings = readings_from(&table, &chosen);
        assert_eq!(
            readings,
            Readings::from([
                (GpuField::TemperatureCore, 55.0),
                (GpuField::TemperatureHotspot, 78.0),
                (GpuField::TemperatureMemory, 70.0),
                (GpuField::ClockCore, 2450.0),
                (GpuField::ClockMemory, 1250.0),
                (GpuField::PowerBoard, 240.0),
                (GpuField::FanRpm, 1200.0),
                (GpuField::FanPercent, 35.0),
                (GpuField::VoltageCore, 1.05),
            ])
        );
    }

    #[test]
    fn discrete_power_falls_back_to_asic_power() {
        let table = table(&[(23, 180)]);
        let chosen = choose_sensors(&table, None, false);
        assert_eq!(chosen, [(GpuField::PowerBoard, SENSOR_ASIC_POWER)]);
        assert_eq!(readings_from(&table, &chosen)[&GpuField::PowerBoard], 180.0);
    }

    #[test]
    fn sensors_missing_from_the_driver_list_are_not_chosen() {
        let table = table(&[(8, 55), (1, 2000)]);
        let chosen = choose_sensors(&table, Some(&[1]), false);
        assert_eq!(chosen, [(GpuField::ClockCore, SENSOR_GFX_CLOCK)]);
    }

    #[test]
    fn zero_temperature_and_unsupported_slots_are_missing_at_a_tick() {
        let chosen = [
            (GpuField::TemperatureCore, SENSOR_EDGE_TEMPERATURE),
            (GpuField::FanRpm, SENSOR_FAN_RPM),
            (GpuField::ClockCore, SENSOR_GFX_CLOCK),
        ];
        let table = table(&[(8, 0), (14, 0)]);
        // 0 °C is a driver placeholder, 0 RPM is a real value (fan stop), the clock slot is
        // no longer marked supported.
        assert_eq!(
            readings_from(&table, &chosen),
            Readings::from([(GpuField::FanRpm, 0.0)])
        );
    }

    #[test]
    fn allocator_callback_round_trips() {
        // SAFETY: plain calls of our own allocator pair.
        unsafe {
            let p = adl_alloc(68).cast::<u8>();
            assert!(!p.is_null());
            assert_eq!(p as usize % 16, 0);
            assert!(std::slice::from_raw_parts(p, 68).iter().all(|&b| b == 0));
            p.write_bytes(0xAB, 68);
            adl_free(p.cast());
            assert!(adl_alloc(0).is_null());
            assert!(adl_alloc(-4).is_null());
            adl_free(null_mut());
        }
    }

    /// ADL sessions are per process: hardware tests must not overlap.
    static HARDWARE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn find(adapters: &[Adapter], vendor: Vendor) -> usize {
        adapters
            .iter()
            .position(|a| a.vendor() == Some(vendor) && a.pci.is_some())
            .unwrap_or_else(|| panic!("no {vendor:?} adapter with a PCI address"))
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn adl_binds_the_amd_igpu_and_reads_gfx_temperature_and_clock() {
        let _guard = HARDWARE.lock().unwrap_or_else(|e| e.into_inner());
        let adapters = crate::gpu::enumerate::enumerate().expect("enumerate");
        let amd = find(&adapters, Vendor::Amd);
        assert!(adapters[amd].integrated, "this test needs an AMD iGPU");
        let mut layer = AdlLayer::load().expect("atiadlxx.dll loads and the context is created");

        let supported = layer.attach(&adapters);

        assert_eq!(
            supported[amd],
            BTreeSet::from([GpuField::TemperatureCore, GpuField::ClockCore])
        );
        let bound = layer.bound[amd].as_ref().expect("AMD iGPU bound");
        assert!(
            matches!(bound.feed, Feed::Shared(_)),
            "this driver supports the shared-memory log"
        );
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        let readings = layer.sample().expect("sample");
        assert_eq!(readings.len(), adapters.len());
        println!("ADL readings: {:?}", readings[amd]);
        let temperature = readings[amd][&GpuField::TemperatureCore];
        let clock = readings[amd][&GpuField::ClockCore];
        assert!(
            (20.0..=100.0).contains(&temperature),
            "GFX temperature {temperature}"
        );
        assert!((100.0..=4000.0).contains(&clock), "GFX clock {clock}");

        // A second attach (rediscover) stops the old log and binds again.
        let again = layer.attach(&adapters);
        assert_eq!(again[amd], supported[amd]);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn adl_ignores_the_nvidia_entries_it_lists() {
        let _guard = HARDWARE.lock().unwrap_or_else(|e| e.into_inner());
        let adapters = crate::gpu::enumerate::enumerate().expect("enumerate");
        let amd = find(&adapters, Vendor::Amd);
        let nvidia = find(&adapters, Vendor::Nvidia);
        let mut layer = AdlLayer::load().expect("atiadlxx.dll loads and the context is created");

        let infos = layer.adapter_infos();
        assert!(
            infos
                .iter()
                .any(|i| text(&i.pnp).to_ascii_uppercase().contains("VEN_10DE")),
            "this driver lists the NVIDIA card among ADL adapters"
        );
        let physical: Vec<PciAddress> = physical_adapters(&infos).into_keys().collect();
        assert_eq!(physical, [adapters[amd].pci.unwrap()]);

        let supported = layer.attach(&adapters);
        assert!(supported[nvidia].is_empty());
        assert!(layer.sample().expect("sample")[nvidia].is_empty());
    }
}
