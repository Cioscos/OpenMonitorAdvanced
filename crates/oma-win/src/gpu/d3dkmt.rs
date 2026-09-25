//! Vendor-neutral GPU readings from the kernel graphics interface (D3DKMT in
//! gdi32): temperature, power as % of the limit, memory and core clocks, fan.

use std::collections::BTreeSet;
use std::mem::size_of;

use oma_core::model::Source;
use oma_core::provider::ProviderError;
use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo, D3DKMT_ADAPTERADDRESS,
    D3DKMT_ADAPTERTYPE, D3DKMT_ADAPTER_PERFDATA, D3DKMT_ADAPTER_PERFDATACAPS, D3DKMT_CLOSEADAPTER,
    D3DKMT_NODE_PERFDATA, D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO,
    KMTQAITYPE_ADAPTERADDRESS, KMTQAITYPE_ADAPTERPERFDATA, KMTQAITYPE_ADAPTERPERFDATA_CAPS,
    KMTQAITYPE_ADAPTERTYPE, KMTQAITYPE_NODEPERFDATA, KMTQUERYADAPTERINFOTYPE,
};
use windows::Win32::Foundation::NTSTATUS;

use super::adapter::Adapter;
use super::enumerate::u64_to_luid;
use super::field::GpuField;
use super::layer::{GpuLayer, Readings};

// The kernel accepts only the exact PrivateDriverDataSize of each query
// (anything else fails with STATUS_INVALID_PARAMETER), so the x64 layouts are pinned.
const _: () = assert!(size_of::<D3DKMT_OPENADAPTERFROMLUID>() == 12);
const _: () = assert!(size_of::<D3DKMT_CLOSEADAPTER>() == 4);
const _: () = assert!(size_of::<D3DKMT_QUERYADAPTERINFO>() == 24);
const _: () = assert!(size_of::<D3DKMT_ADAPTERADDRESS>() == 12);
const _: () = assert!(size_of::<D3DKMT_ADAPTERTYPE>() == 4);
const _: () = assert!(size_of::<D3DKMT_ADAPTER_PERFDATA>() == 64);
const _: () = assert!(size_of::<D3DKMT_ADAPTER_PERFDATACAPS>() == 40);
const _: () = assert!(size_of::<D3DKMT_NODE_PERFDATA>() == 56);

/// Node 0 is the 3D/graphics engine on every adapter observed; its clock is the core clock.
const CORE_NODE: u32 = 0;

/// Fields read from ADAPTERPERFDATA (the rest come from NODEPERFDATA).
const ADAPTER_PERF_FIELDS: [GpuField; 4] = [
    GpuField::TemperatureCore,
    GpuField::PowerLimitPercent,
    GpuField::ClockMemory,
    GpuField::FanRpm,
];

fn nt_result(status: NTSTATUS) -> Result<(), NTSTATUS> {
    if status.0 < 0 {
        Err(status)
    } else {
        Ok(())
    }
}

/// A query that worked at attach and fails now means the handle went stale
/// (driver update, TDR, device removed): the adapters must be enumerated again.
fn stale(status: NTSTATUS) -> ProviderError {
    tracing::debug!(
        status = format!("{:#010x}", status.0 as u32),
        "D3DKMT query failed; requesting rediscovery"
    );
    ProviderError::Rediscover
}

/// An open D3DKMT adapter handle, closed on drop.
pub(crate) struct KmtAdapter {
    handle: u32,
}

impl KmtAdapter {
    pub(crate) fn open(luid: u64) -> Result<Self, NTSTATUS> {
        let mut open = D3DKMT_OPENADAPTERFROMLUID {
            AdapterLuid: u64_to_luid(luid),
            hAdapter: 0,
        };
        // SAFETY: `open` is a valid, writable D3DKMT_OPENADAPTERFROMLUID.
        nt_result(unsafe { D3DKMTOpenAdapterFromLuid(&mut open) })?;
        Ok(Self {
            handle: open.hAdapter,
        })
    }

    /// # Safety
    /// `T` must be the exact structure the kernel expects for `kind`.
    unsafe fn query<T>(&self, kind: KMTQUERYADAPTERINFOTYPE, data: &mut T) -> Result<(), NTSTATUS> {
        let mut query = D3DKMT_QUERYADAPTERINFO {
            hAdapter: self.handle,
            Type: kind,
            pPrivateDriverData: (data as *mut T).cast(),
            PrivateDriverDataSize: size_of::<T>() as u32,
        };
        // SAFETY: the handle is open and `data` is a writable buffer of the
        // declared size whose layout matches `kind` (caller contract).
        nt_result(unsafe { D3DKMTQueryAdapterInfo(&mut query) })
    }

    pub(crate) fn address(&self) -> Result<D3DKMT_ADAPTERADDRESS, NTSTATUS> {
        let mut address = D3DKMT_ADAPTERADDRESS::default();
        // SAFETY: ADAPTERADDRESS fills a D3DKMT_ADAPTERADDRESS.
        unsafe { self.query(KMTQAITYPE_ADAPTERADDRESS, &mut address) }?;
        Ok(address)
    }

    /// ADAPTERTYPE flag bits (bit 5 = HybridIntegrated).
    pub(crate) fn adapter_type(&self) -> Result<u32, NTSTATUS> {
        let mut adapter_type = D3DKMT_ADAPTERTYPE::default();
        // SAFETY: ADAPTERTYPE fills a D3DKMT_ADAPTERTYPE.
        unsafe { self.query(KMTQAITYPE_ADAPTERTYPE, &mut adapter_type) }?;
        // SAFETY: every bit pattern of the union is a valid u32.
        Ok(unsafe { adapter_type.Anonymous.Value })
    }

    fn perf(&self) -> Result<D3DKMT_ADAPTER_PERFDATA, NTSTATUS> {
        let mut perf = D3DKMT_ADAPTER_PERFDATA {
            PhysicalAdapterIndex: 0,
            ..Default::default()
        };
        // SAFETY: ADAPTERPERFDATA fills a D3DKMT_ADAPTER_PERFDATA.
        unsafe { self.query(KMTQAITYPE_ADAPTERPERFDATA, &mut perf) }?;
        Ok(perf)
    }

    fn caps(&self) -> Result<D3DKMT_ADAPTER_PERFDATACAPS, NTSTATUS> {
        let mut caps = D3DKMT_ADAPTER_PERFDATACAPS {
            PhysicalAdapterIndex: 0,
            ..Default::default()
        };
        // SAFETY: ADAPTERPERFDATA_CAPS fills a D3DKMT_ADAPTER_PERFDATACAPS.
        unsafe { self.query(KMTQAITYPE_ADAPTERPERFDATA_CAPS, &mut caps) }?;
        Ok(caps)
    }

    fn node_perf(&self, node: u32) -> Result<D3DKMT_NODE_PERFDATA, NTSTATUS> {
        let mut perf = D3DKMT_NODE_PERFDATA {
            NodeOrdinal: node,
            PhysicalAdapterIndex: 0,
            ..Default::default()
        };
        // SAFETY: NODEPERFDATA fills a D3DKMT_NODE_PERFDATA.
        unsafe { self.query(KMTQAITYPE_NODEPERFDATA, &mut perf) }?;
        Ok(perf)
    }
}

impl Drop for KmtAdapter {
    fn drop(&mut self) {
        let close = D3DKMT_CLOSEADAPTER {
            hAdapter: self.handle,
        };
        // SAFETY: the handle is open and never used after this point.
        unsafe {
            let _ = D3DKMTCloseAdapter(&close);
        }
    }
}

/// One reading in the kernel's units.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct PerfSample {
    /// Deci-degrees Celsius (452 = 45.2 °C); 0 = not reported.
    pub temperature: u32,
    /// Board power in tenths of a percent of the power limit (109 = 10.9 %).
    pub power: u32,
    /// Memory clock in Hz.
    pub memory_hz: u64,
    /// Fan speed in RPM; 0 while the fan is stopped.
    pub fan_rpm: u32,
    /// Node 0 clock in Hz, when NODEPERFDATA was queried.
    pub core_hz: Option<u64>,
}

impl PerfSample {
    fn new(adapter: &D3DKMT_ADAPTER_PERFDATA, node: Option<&D3DKMT_NODE_PERFDATA>) -> Self {
        Self {
            temperature: adapter.Temperature,
            power: adapter.Power,
            memory_hz: adapter.MemoryFrequency,
            fan_rpm: adapter.FanRPM,
            core_hz: node.map(|n| n.Frequency),
        }
    }
}

/// Static capabilities probed once at attach.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Caps {
    /// ADAPTERPERFDATA_CAPS MaxFanRPM; 0 = the adapter reports no fan.
    pub max_fan_rpm: u32,
    /// NODEPERFDATA node 0 MaxFrequency in Hz; 0 = no core clock.
    pub max_core_hz: u64,
}

/// Maps one reading to fields. Memory clock and power are discrete-only: on
/// an iGPU they describe system RAM and the shared SoC budget.
pub(crate) fn readings_from(perf: PerfSample, caps: Caps, integrated: bool) -> Readings {
    let mut readings = Readings::new();
    if perf.temperature > 0 {
        readings.insert(GpuField::TemperatureCore, perf.temperature as f64 / 10.0);
    }
    if !integrated {
        readings.insert(GpuField::PowerLimitPercent, perf.power as f64 / 10.0);
        if perf.memory_hz > 0 {
            readings.insert(GpuField::ClockMemory, perf.memory_hz as f64 / 1e6);
        }
    }
    if caps.max_core_hz > 0 {
        if let Some(hz) = perf.core_hz.filter(|&hz| hz > 0) {
            readings.insert(GpuField::ClockCore, hz as f64 / 1e6);
        }
    }
    if caps.max_fan_rpm > 0 {
        // With a fan present, 0 RPM is a real value (zero-RPM idle mode).
        readings.insert(GpuField::FanRpm, perf.fan_rpm as f64);
    }
    readings
}

/// Fields supported given the attach-time probe. Power needs a non-zero
/// probe: a driver that does not implement it reports a constant 0.
pub(crate) fn supported_from(
    probe: PerfSample,
    caps: Caps,
    integrated: bool,
) -> BTreeSet<GpuField> {
    readings_from(probe, caps, integrated)
        .into_keys()
        .filter(|&field| field != GpuField::PowerLimitPercent || probe.power > 0)
        .collect()
}

struct Bound {
    kmt: KmtAdapter,
    caps: Caps,
    integrated: bool,
    fields: BTreeSet<GpuField>,
}

impl Bound {
    fn attach(adapter: &Adapter) -> Option<Self> {
        let kmt = KmtAdapter::open(adapter.luid)
            .map_err(|status| {
                tracing::debug!(
                    adapter = %adapter.name,
                    status = format!("{:#010x}", status.0 as u32),
                    "D3DKMTOpenAdapterFromLuid failed"
                );
            })
            .ok()?;
        let perf = kmt.perf().ok();
        let node = kmt.node_perf(CORE_NODE).ok();
        let caps = Caps {
            // Without ADAPTERPERFDATA no fan value can ever be read.
            max_fan_rpm: match perf {
                Some(_) => kmt.caps().map_or(0, |c| c.MaxFanRPM),
                None => 0,
            },
            max_core_hz: node.map_or(0, |n| n.MaxFrequency),
        };
        let probe = PerfSample::new(&perf.unwrap_or_default(), node.as_ref());
        let fields = supported_from(probe, caps, adapter.integrated);
        (!fields.is_empty()).then_some(Self {
            kmt,
            caps,
            integrated: adapter.integrated,
            fields,
        })
    }

    fn read(&self) -> Result<Readings, ProviderError> {
        let perf = if ADAPTER_PERF_FIELDS.iter().any(|f| self.fields.contains(f)) {
            self.kmt.perf().map_err(stale)?
        } else {
            D3DKMT_ADAPTER_PERFDATA::default()
        };
        let node = if self.fields.contains(&GpuField::ClockCore) {
            Some(self.kmt.node_perf(CORE_NODE).map_err(stale)?)
        } else {
            None
        };
        let mut readings = readings_from(
            PerfSample::new(&perf, node.as_ref()),
            self.caps,
            self.integrated,
        );
        readings.retain(|field, _| self.fields.contains(field));
        Ok(readings)
    }
}

/// D3DKMT layer: one open handle per adapter, kept between samples.
#[derive(Default)]
pub(crate) struct D3dkmtLayer {
    bound: Vec<Option<Bound>>,
}

impl GpuLayer for D3dkmtLayer {
    fn source(&self) -> Source {
        Source::D3dkmt
    }

    fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>> {
        self.bound.clear(); // closes the previous handles
        self.bound = adapters.iter().map(Bound::attach).collect();
        self.bound
            .iter()
            .map(|b| b.as_ref().map(|b| b.fields.clone()).unwrap_or_default())
            .collect()
    }

    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
        self.bound
            .iter()
            .map(|b| b.as_ref().map_or_else(|| Ok(Readings::new()), Bound::read))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RTX 4080 at idle (spike): 47.0 °C, 10.7 % of 320 W, 405 MHz memory,
    /// fan stopped, 210 MHz core; MaxFanRPM 3000, MaxFrequency 3120 MHz.
    fn rtx_idle() -> (PerfSample, Caps) {
        (
            PerfSample {
                temperature: 470,
                power: 107,
                memory_hz: 405_000_000,
                fan_rpm: 0,
                core_hz: Some(210_000_000),
            },
            Caps {
                max_fan_rpm: 3000,
                max_core_hz: 3_120_000_000,
            },
        )
    }

    /// Radeon iGPU (spike): 41.0 °C, power 55, DRAM 3000 MHz, 600 MHz core, no fan.
    fn radeon_idle() -> (PerfSample, Caps) {
        (
            PerfSample {
                temperature: 410,
                power: 55,
                memory_hz: 3_000_000_000,
                fan_rpm: 0,
                core_hz: Some(600_000_000),
            },
            Caps {
                max_fan_rpm: 0,
                max_core_hz: 2_200_000_000,
            },
        )
    }

    #[test]
    fn discrete_readings_use_kernel_units() {
        let (perf, caps) = rtx_idle();
        let r = readings_from(perf, caps, false);
        assert_eq!(r.get(&GpuField::TemperatureCore), Some(&47.0));
        assert!((r[&GpuField::PowerLimitPercent] - 10.7).abs() < 1e-9);
        assert_eq!(r.get(&GpuField::ClockMemory), Some(&405.0));
        assert_eq!(r.get(&GpuField::ClockCore), Some(&210.0));
        assert_eq!(r.len(), 5);
    }

    #[test]
    fn integrated_has_no_memory_clock_or_power() {
        let (perf, caps) = radeon_idle();
        let r = readings_from(perf, caps, true);
        assert_eq!(
            r.keys().copied().collect::<Vec<_>>(),
            vec![GpuField::TemperatureCore, GpuField::ClockCore]
        );
        assert_eq!(r[&GpuField::TemperatureCore], 41.0);
        assert_eq!(r[&GpuField::ClockCore], 600.0);
    }

    #[test]
    fn zero_rpm_is_a_value_when_the_fan_exists() {
        let (perf, caps) = rtx_idle();
        assert_eq!(
            readings_from(perf, caps, false).get(&GpuField::FanRpm),
            Some(&0.0)
        );
        let no_fan = Caps {
            max_fan_rpm: 0,
            ..caps
        };
        assert_eq!(
            readings_from(perf, no_fan, false).get(&GpuField::FanRpm),
            None
        );
    }

    #[test]
    fn zero_temperature_is_missing() {
        let (perf, caps) = rtx_idle();
        let cold = PerfSample {
            temperature: 0,
            ..perf
        };
        assert_eq!(
            readings_from(cold, caps, false).get(&GpuField::TemperatureCore),
            None
        );
        assert!(!supported_from(cold, caps, false).contains(&GpuField::TemperatureCore));
    }

    #[test]
    fn core_clock_needs_a_max_frequency_and_a_non_zero_value() {
        let (perf, caps) = rtx_idle();
        let no_max = Caps {
            max_core_hz: 0,
            ..caps
        };
        assert_eq!(
            readings_from(perf, no_max, false).get(&GpuField::ClockCore),
            None
        );
        let gated = PerfSample {
            core_hz: Some(0),
            ..perf
        };
        assert_eq!(
            readings_from(gated, caps, false).get(&GpuField::ClockCore),
            None
        );
        let not_queried = PerfSample {
            core_hz: None,
            ..perf
        };
        assert_eq!(
            readings_from(not_queried, caps, false).get(&GpuField::ClockCore),
            None
        );
    }

    #[test]
    fn supported_fields_match_this_machine() {
        let (perf, caps) = rtx_idle();
        assert_eq!(
            supported_from(perf, caps, false),
            BTreeSet::from([
                GpuField::TemperatureCore,
                GpuField::ClockCore,
                GpuField::ClockMemory,
                GpuField::PowerLimitPercent,
                GpuField::FanRpm,
            ])
        );
        let (perf, caps) = radeon_idle();
        assert_eq!(
            supported_from(perf, caps, true),
            BTreeSet::from([GpuField::TemperatureCore, GpuField::ClockCore])
        );
    }

    #[test]
    fn power_needs_a_non_zero_probe() {
        let (perf, caps) = rtx_idle();
        let unpowered = PerfSample { power: 0, ..perf };
        assert!(!supported_from(unpowered, caps, false).contains(&GpuField::PowerLimitPercent));
        // Once supported, a 0 at a later tick is still a value.
        assert_eq!(
            readings_from(unpowered, caps, false).get(&GpuField::PowerLimitPercent),
            Some(&0.0)
        );
    }

    #[test]
    fn stale_handle_requests_rediscover() {
        const STATUS_INVALID_PARAMETER: i32 = 0xC000_000Du32 as i32;
        const STATUS_DEVICE_REMOVED: i32 = 0xC000_02B6u32 as i32;
        for code in [STATUS_INVALID_PARAMETER, STATUS_DEVICE_REMOVED] {
            assert_eq!(
                nt_result(NTSTATUS(code)).map_err(stale),
                Err(ProviderError::Rediscover)
            );
        }
        assert_eq!(nt_result(NTSTATUS(0)).map_err(stale), Ok(()));
    }

    #[test]
    fn empty_attach_samples_nothing() {
        let mut layer = D3dkmtLayer::default();
        assert!(layer.attach(&[]).is_empty());
        assert_eq!(layer.sample(), Ok(vec![]));
        assert_eq!(layer.source(), Source::D3dkmt);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn reads_this_machines_gpus() {
        let adapters = super::super::enumerate::enumerate().expect("enumerate");
        let mut layer = D3dkmtLayer::default();
        let supported = layer.attach(&adapters);
        assert_eq!(supported.len(), adapters.len());
        let rtx = adapters
            .iter()
            .position(|a| a.vendor_id == 0x10DE)
            .expect("NVIDIA adapter");
        let radeon = adapters
            .iter()
            .position(|a| a.vendor_id == 0x1002)
            .expect("AMD adapter");
        assert_eq!(
            supported[rtx],
            BTreeSet::from([
                GpuField::TemperatureCore,
                GpuField::ClockCore,
                GpuField::ClockMemory,
                GpuField::PowerLimitPercent,
                GpuField::FanRpm,
            ])
        );
        assert_eq!(
            supported[radeon],
            BTreeSet::from([GpuField::TemperatureCore, GpuField::ClockCore])
        );

        let values = layer.sample().expect("sample");
        assert_eq!(values.len(), adapters.len());
        let nv = &values[rtx];
        let temperature = nv[&GpuField::TemperatureCore];
        assert!(
            (20.0..=100.0).contains(&temperature),
            "temperature {temperature}"
        );
        let core = nv[&GpuField::ClockCore];
        assert!((100.0..=3_500.0).contains(&core), "core clock {core} MHz");
        let memory = nv[&GpuField::ClockMemory];
        assert!(
            (100.0..=15_000.0).contains(&memory),
            "memory clock {memory} MHz"
        );
        let power = nv[&GpuField::PowerLimitPercent];
        assert!((1.0..=150.0).contains(&power), "power {power} %");
        let fan = nv[&GpuField::FanRpm];
        assert!((0.0..=5_000.0).contains(&fan), "fan {fan} rpm");
        let amd = &values[radeon];
        let temperature = amd[&GpuField::TemperatureCore];
        assert!(
            (20.0..=100.0).contains(&temperature),
            "iGPU temperature {temperature}"
        );
        let core = amd[&GpuField::ClockCore];
        assert!(
            (100.0..=3_000.0).contains(&core),
            "iGPU core clock {core} MHz"
        );

        // Re-attaching closes the old handles and opens new ones.
        assert_eq!(layer.attach(&adapters), supported);
        assert!(layer.sample().is_ok());
    }
}
