//! GPU provider: one device per physical adapter, each field read from the
//! highest-priority layer that supports it (spec §5.2): vendor libraries, then
//! D3DKMT, DXGI and PDH. Layers also contribute static device properties
//! (PnP for the PCIe maximum link, NVML for limits).

pub(crate) mod adapter;
pub(crate) mod adl;
pub(crate) mod d3dkmt;
pub(crate) mod dxgi;
pub(crate) mod enumerate;
pub(crate) mod field;
pub(crate) mod igcl;
pub(crate) mod layer;
pub(crate) mod nvapi;
pub(crate) mod nvml;
pub(crate) mod pdh;
pub(crate) mod pnp;
pub(crate) mod trim;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use oma_core::merge;
use oma_core::model::{Device, DeviceKind, Label, Sensor};
use oma_core::provider::{Inventory, Provider, ProviderError};

use adapter::{Adapter, PciAddress};
use field::GpuField;
use layer::{GpuLayer, Readings};

/// Shared on/off switch for GPU vendor libraries (safe mode, spec §8).
#[derive(Debug, Clone, Default)]
pub struct VendorSwitch(Arc<AtomicBool>);

impl VendorSwitch {
    pub fn new(enabled: bool) -> Self {
        Self(Arc::new(AtomicBool::new(enabled)))
    }

    pub fn enabled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Turns vendor libraries on for every clone of this switch. The GPU
    /// provider notices on its next poll and rediscovers.
    pub fn enable(&self) {
        self.0.store(true, Ordering::Release);
    }
}

type Enumerate = Box<dyn FnMut() -> Result<Vec<Adapter>, ProviderError> + Send>;
type MakeVendor = Box<dyn FnMut() -> Vec<Box<dyn GpuLayer>> + Send>;

/// Where the value of one sensor comes from.
struct Slot {
    /// Index in the active layer list (vendor layers first, then base layers).
    layer: usize,
    adapter: usize,
    field: GpuField,
}

/// What the last discover decided.
#[derive(Default)]
struct State {
    adapters: usize,
    /// Last successful topology, including the empty-adapter case.
    topology: Vec<Adapter>,
    topology_checked: Option<std::time::Instant>,
    /// Switch state seen at discover; a change means rediscover.
    vendor_on: bool,
    /// One entry per sensor, in inventory order.
    slots: Vec<Slot>,
    /// Per active layer: already warned about a failing sample in this streak.
    failing: Vec<bool>,
}

pub struct GpuProvider {
    enumerate: Enumerate,
    base: Vec<Box<dyn GpuLayer>>,
    vendor: Vec<Box<dyn GpuLayer>>,
    /// Taken (so called at most once) by the first discover with the switch on.
    make_vendor: Option<MakeVendor>,
    switch: VendorSwitch,
    state: State,
    /// PCI address last seen per LUID, kept across discovers (see `restore_pci`).
    known_pci: HashMap<u64, PciAddress>,
}

impl GpuProvider {
    /// Test/assembly constructor. `vendor` layers are created by `make_vendor` only while the switch is on,
    /// once, on the first discover that sees the switch on; they then stay for the process lifetime (D1).
    pub(crate) fn with_layers(
        enumerate: Enumerate,
        base: Vec<Box<dyn GpuLayer>>, // priority order: d3dkmt, dxgi, pdh, pnp
        make_vendor: MakeVendor,      // priority order: nvml, nvapi, adl, igcl
        switch: VendorSwitch,
    ) -> Self {
        Self {
            enumerate,
            base,
            vendor: Vec::new(),
            make_vendor: Some(make_vendor),
            switch,
            state: State::default(),
            known_pci: HashMap::new(),
        }
    }

    /// Active layers in priority order: vendor layers (only while on) ++ base layers.
    fn active_layers(&mut self, vendor_on: bool) -> Vec<&mut Box<dyn GpuLayer>> {
        let vendor: &mut [Box<dyn GpuLayer>] = if vendor_on { &mut self.vendor } else { &mut [] };
        vendor.iter_mut().chain(self.base.iter_mut()).collect()
    }
}

impl GpuProvider {
    /// The real provider: DXGI/DXCore/D3DKMT enumeration, the base layers
    /// (D3DKMT, DXGI, PDH, PnP) always on, and the vendor libraries (NVML,
    /// NVAPI, ADL, IGCL) loaded on the first discover that sees `switch` on.
    pub fn new(switch: VendorSwitch) -> Self {
        Self::with_layers(
            Box::new(enumerate::enumerate),
            vec![
                Box::new(d3dkmt::D3dkmtLayer::default()),
                Box::new(dxgi::DxgiLayer::default()),
                Box::new(pdh::PdhLayer::default()),
                Box::new(pnp::PnpLayer::default()),
            ],
            Box::new(load_vendor_layers),
            switch,
        )
    }
}

/// Loads every GPU vendor library installed on this machine, in merge
/// priority order. A library that is absent or fails to initialise is
/// skipped. Called at most once per provider: loaded libraries stay for the
/// process lifetime (decision D1).
fn load_vendor_layers() -> Vec<Box<dyn GpuLayer>> {
    let candidates: [Option<Box<dyn GpuLayer>>; 4] = [
        nvml::NvmlLayer::load().map(|layer| Box::new(layer) as Box<dyn GpuLayer>),
        nvapi::NvapiLayer::load().map(|layer| Box::new(layer) as Box<dyn GpuLayer>),
        adl::AdlLayer::load().map(|layer| Box::new(layer) as Box<dyn GpuLayer>),
        igcl::IgclLayer::load().map(|layer| Box::new(layer) as Box<dyn GpuLayer>),
    ];
    let layers: Vec<Box<dyn GpuLayer>> = candidates.into_iter().flatten().collect();
    let sources: Vec<_> = layers.iter().map(|layer| layer.source()).collect();
    tracing::info!(?sources, "GPU vendor libraries loaded");
    // A vendor DLL may have installed its own top-level exception filter.
    crate::crash::rearm_crash_marker();
    layers
}

/// Gives back the PCI address of an adapter whose kernel query failed this time but
/// worked before (same LUID), so its device id does not change; records every address
/// seen. A failed D3DKMT address query must not rename a GPU (M2 follow-up).
fn restore_pci(known: &mut HashMap<u64, PciAddress>, adapters: &mut [Adapter]) {
    for adapter in adapters {
        match adapter.pci {
            Some(pci) => {
                known.insert(adapter.luid, pci);
            }
            None => adapter.pci = known.get(&adapter.luid).copied(),
        }
    }
}

/// Adds layer properties to a device's own ones: the first layer (highest priority) wins
/// per key, and keys the device already has (`pciAddress`, `integrated`) are never replaced.
fn merge_properties(
    device: &mut BTreeMap<String, String>,
    layers: impl IntoIterator<Item = BTreeMap<String, String>>,
) {
    for properties in layers {
        for (key, value) in properties {
            device.entry(key).or_insert(value);
        }
    }
}

fn device(adapter: &Adapter, id: &str) -> Device {
    let mut properties = std::collections::BTreeMap::new();
    if let Some(pci) = adapter.pci {
        properties.insert("pciAddress".to_owned(), pci.to_string());
    }
    properties.insert("integrated".to_owned(), adapter.integrated.to_string());
    Device {
        id: id.to_owned(),
        kind: DeviceKind::Gpu,
        name: adapter.name.clone(),
        vendor: adapter.vendor().map(|v| v.name().to_owned()),
        properties,
    }
}

impl Provider for GpuProvider {
    fn name(&self) -> &'static str {
        "gpu"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        self.state = State::default();
        let mut adapters = (self.enumerate)()?;
        restore_pci(&mut self.known_pci, &mut adapters);
        let vendor_on = self.switch.enabled();
        if vendor_on {
            if let Some(mut make_vendor) = self.make_vendor.take() {
                self.vendor = make_vendor();
            }
        }
        let count = adapters.len();
        let mut layers = self.active_layers(vendor_on);
        let supported: Vec<Vec<BTreeSet<GpuField>>> = layers
            .iter_mut()
            .map(|layer| {
                let mut sets = layer.attach(&adapters);
                if sets.len() != count {
                    tracing::warn!(source = ?layer.source(), "GPU layer attach returned a wrong adapter count");
                    sets.resize(count, BTreeSet::new());
                }
                sets
            })
            .collect();

        let mut inventory = Inventory::default();
        let mut slots = Vec::new();
        let mut ordinal = 0;
        for (index, adapter) in adapters.iter().enumerate() {
            let id = adapter.device_id(ordinal);
            if adapter.pci.is_none() {
                ordinal += 1;
            }
            tracing::debug!(
                id,
                luid = format_args!("{:#x}", adapter.luid),
                device = format_args!("{:04x}", adapter.device_id),
                subsys = format_args!("{:08x}", adapter.subsys_id),
                dedicated_bytes = adapter.dedicated_bytes,
                "GPU adapter"
            );
            let mut gpu = device(adapter, &id);
            merge_properties(
                &mut gpu.properties,
                layers.iter().map(|layer| layer.properties(index)),
            );
            inventory.devices.push(gpu);
            let per_layer: Vec<_> = supported.iter().map(|s| s[index].clone()).collect();
            let owners = merge::assign(&per_layer);
            for field in GpuField::ALL {
                let Some(&layer) = owners.get(&field) else {
                    continue;
                };
                let owner = &layers[layer];
                let mut sensor = Sensor::new(
                    &id,
                    field.kind(),
                    field.name(),
                    field.unit(),
                    Label::new(field.label_key()),
                    owner.source(),
                );
                if owner.is_experimental(field) {
                    sensor = sensor.experimental();
                }
                inventory.sensors.push(sensor);
                slots.push(Slot {
                    layer,
                    adapter: index,
                    field,
                });
            }
        }
        let failing = vec![false; layers.len()];
        self.state = State {
            adapters: count,
            topology: adapters,
            topology_checked: Some(std::time::Instant::now()),
            vendor_on,
            slots,
            failing,
        };
        Ok(inventory)
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        // Re-enabling vendor libraries ("Riattiva") is observed only here. While the
        // engine backs this provider off (repeated failures, or repeated Rediscovers
        // per decision D8) poll is not called, so the switch can take up to the
        // 60 s maximum backoff to be picked up.
        if self.switch.enabled() != self.state.vendor_on {
            return Err(ProviderError::Rediscover);
        }
        // A newly connected GPU cannot invalidate a handle that we never opened.
        // Check topology even when no adapter/layer currently offers sensors.
        let now = std::time::Instant::now();
        if self
            .state
            .topology_checked
            .is_none_or(|last| now.duration_since(last) >= std::time::Duration::from_secs(5))
        {
            let mut topology = (self.enumerate)()?;
            restore_pci(&mut self.known_pci, &mut topology);
            self.state.topology_checked = Some(now);
            if topology != self.state.topology {
                return Err(ProviderError::Rediscover);
            }
        }
        let count = self.state.adapters;
        let vendor_on = self.state.vendor_on;
        let mut samples: Vec<Option<Vec<Readings>>> = Vec::new();
        let mut failed = Vec::new();
        for layer in self.active_layers(vendor_on) {
            match layer.sample() {
                Ok(readings) if readings.len() == count => {
                    samples.push(Some(readings));
                    failed.push(None);
                }
                Ok(_) => {
                    samples.push(None);
                    failed.push(Some((layer.source(), "wrong adapter count".to_owned())));
                }
                Err(ProviderError::Rediscover) => return Err(ProviderError::Rediscover),
                Err(err) => {
                    samples.push(None);
                    failed.push(Some((layer.source(), err.to_string())));
                }
            }
        }
        // Warn once per failure streak, per layer.
        for (warned, failure) in self.state.failing.iter_mut().zip(failed) {
            match failure {
                Some((source, err)) if !*warned => {
                    tracing::warn!(?source, %err, "GPU layer sample failed");
                    *warned = true;
                }
                Some(_) => {}
                None => *warned = false,
            }
        }
        Ok(self
            .state
            .slots
            .iter()
            .map(|slot| {
                samples[slot.layer]
                    .as_ref()
                    .and_then(|readings| readings[slot.adapter].get(&slot.field).copied())
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::adapter::PciAddress;
    use super::*;
    use oma_core::model::Source;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    /// Scripted behaviour of a fake layer, shared with the test.
    #[derive(Default)]
    struct Script {
        /// Per adapter: supported fields and the value returned for each.
        values: Vec<Readings>,
        /// Returned (once each) by `sample` before the values.
        errors: Vec<ProviderError>,
        /// When set, `sample` returns this many adapters instead of `values.len()`.
        wrong_len: Option<usize>,
        attach_calls: usize,
        sample_calls: usize,
        /// Per adapter: the static properties the layer reports.
        properties: Vec<BTreeMap<String, String>>,
    }

    struct FakeLayer {
        source: Source,
        experimental: Vec<GpuField>,
        script: Arc<Mutex<Script>>,
    }

    impl GpuLayer for FakeLayer {
        fn source(&self) -> Source {
            self.source
        }

        fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>> {
            let mut s = self.script.lock().unwrap();
            s.attach_calls += 1;
            (0..adapters.len())
                .map(|i| {
                    s.values
                        .get(i)
                        .map(|r| r.keys().copied().collect())
                        .unwrap_or_default()
                })
                .collect()
        }

        fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
            let mut s = self.script.lock().unwrap();
            s.sample_calls += 1;
            if !s.errors.is_empty() {
                return Err(s.errors.remove(0));
            }
            let mut values = s.values.clone();
            if let Some(len) = s.wrong_len {
                values.resize(len, Readings::new());
            }
            Ok(values)
        }

        fn is_experimental(&self, field: GpuField) -> bool {
            self.experimental.contains(&field)
        }

        fn properties(&self, adapter: usize) -> BTreeMap<String, String> {
            let s = self.script.lock().unwrap();
            s.properties.get(adapter).cloned().unwrap_or_default()
        }
    }

    /// A fake layer; `per_adapter[i]` lists the fields (and values) it has for adapter `i`.
    fn fake(
        source: Source,
        per_adapter: &[&[(GpuField, f64)]],
    ) -> (Box<dyn GpuLayer>, Arc<Mutex<Script>>) {
        let script = Arc::new(Mutex::new(Script {
            values: per_adapter
                .iter()
                .map(|fields| fields.iter().copied().collect())
                .collect(),
            ..Default::default()
        }));
        let layer = FakeLayer {
            source,
            experimental: Vec::new(),
            script: script.clone(),
        };
        (Box::new(layer), script)
    }

    fn nvidia() -> Adapter {
        Adapter {
            luid: 0x0001_0000_0000_1234,
            name: "NVIDIA GeForce RTX 4080".into(),
            vendor_id: 0x10DE,
            device_id: 0x2704,
            subsys_id: 0x167A_10DE,
            pci: Some(PciAddress {
                bus: 1,
                device: 0,
                function: 0,
            }),
            integrated: false,
            dedicated_bytes: 16 << 30,
        }
    }

    fn amd_igpu() -> Adapter {
        Adapter {
            luid: 0x0000_0000_0000_5678,
            name: "AMD Radeon(TM) Graphics".into(),
            vendor_id: 0x1002,
            device_id: 0x164E,
            subsys_id: 0,
            pci: Some(PciAddress {
                bus: 0x11,
                device: 0,
                function: 0,
            }),
            integrated: true,
            dedicated_bytes: 512 << 20,
        }
    }

    fn virtual_adapter(vendor_id: u32) -> Adapter {
        Adapter {
            luid: 0x9999,
            name: "Virtual GPU".into(),
            vendor_id,
            device_id: 0x0001,
            subsys_id: 0,
            pci: None,
            integrated: false,
            dedicated_bytes: 0,
        }
    }

    /// Builds a provider; the returned counter counts `make_vendor` calls.
    fn provider(
        adapters: Vec<Adapter>,
        base: Vec<Box<dyn GpuLayer>>,
        vendor: Vec<Box<dyn GpuLayer>>,
        switch: &VendorSwitch,
    ) -> (GpuProvider, Arc<AtomicUsize>) {
        let made = Arc::new(AtomicUsize::new(0));
        let counter = made.clone();
        let mut vendor = Some(vendor);
        let provider = GpuProvider::with_layers(
            Box::new(move || Ok(adapters.clone())),
            base,
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                vendor.take().unwrap_or_default()
            }),
            switch.clone(),
        );
        (provider, made)
    }

    fn ids_and_sources(inventory: &Inventory) -> Vec<(&str, Source)> {
        inventory
            .sensors
            .iter()
            .map(|s| (s.id.as_str(), s.source))
            .collect()
    }

    #[test]
    fn vendor_layer_wins_and_base_layers_fill_the_gaps() {
        use GpuField::*;
        let (nvml, _) = fake(
            Source::Nvml,
            &[&[(TemperatureCore, 54.0), (ClockCore, 2_610.0)]],
        );
        let (d3dkmt, _) = fake(Source::D3dkmt, &[&[(TemperatureCore, 50.0), (FanRpm, 0.0)]]);
        let (pdh, _) = fake(Source::Pdh, &[&[(LoadCore, 12.5)]]);
        let (mut p, _) = provider(
            vec![nvidia()],
            vec![d3dkmt, pdh],
            vec![nvml],
            &VendorSwitch::new(true),
        );
        let inventory = p.discover().unwrap();
        assert_eq!(
            ids_and_sources(&inventory),
            vec![
                ("gpu/pci-0000:01:00.0/load/core", Source::Pdh),
                ("gpu/pci-0000:01:00.0/temperature/core", Source::Nvml),
                ("gpu/pci-0000:01:00.0/clock/core", Source::Nvml),
                ("gpu/pci-0000:01:00.0/fan/rpm", Source::D3dkmt),
            ]
        );
        assert_eq!(
            inventory.sensors[1].label.key,
            "gpu.temperature.core".to_owned()
        );
        assert_eq!(
            p.poll().unwrap(),
            vec![Some(12.5), Some(54.0), Some(2_610.0), Some(0.0)]
        );
    }

    #[test]
    fn merge_is_decided_per_adapter() {
        use GpuField::*;
        let (nvml, _) = fake(Source::Nvml, &[&[(TemperatureCore, 54.0)], &[]]);
        let (d3dkmt, _) = fake(
            Source::D3dkmt,
            &[&[(TemperatureCore, 50.0)], &[(TemperatureCore, 45.0)]],
        );
        let (mut p, _) = provider(
            vec![nvidia(), amd_igpu()],
            vec![d3dkmt],
            vec![nvml],
            &VendorSwitch::new(true),
        );
        let inventory = p.discover().unwrap();
        assert_eq!(
            ids_and_sources(&inventory),
            vec![
                ("gpu/pci-0000:01:00.0/temperature/core", Source::Nvml),
                ("gpu/pci-0000:11:00.0/temperature/core", Source::D3dkmt),
            ]
        );
        assert_eq!(p.poll().unwrap(), vec![Some(54.0), Some(45.0)]);
    }

    #[test]
    fn devices_have_stable_ids_names_vendors_and_properties() {
        let (mut p, _) = provider(
            vec![
                nvidia(),
                virtual_adapter(0x1414),
                amd_igpu(),
                virtual_adapter(0x8086),
            ],
            Vec::new(),
            Vec::new(),
            &VendorSwitch::new(false),
        );
        let inventory = p.discover().unwrap();
        let ids: Vec<_> = inventory.devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "gpu/pci-0000:01:00.0",
                "gpu/ven-1414-dev-0001-0",
                "gpu/pci-0000:11:00.0",
                "gpu/ven-8086-dev-0001-1",
            ]
        );
        let nv = &inventory.devices[0];
        assert_eq!(nv.kind, DeviceKind::Gpu);
        assert_eq!(nv.name, "NVIDIA GeForce RTX 4080");
        assert_eq!(nv.vendor.as_deref(), Some("NVIDIA"));
        assert_eq!(nv.properties["pciAddress"], "0000:01:00.0");
        assert_eq!(nv.properties["integrated"], "false");
        let amd = &inventory.devices[2];
        assert_eq!(amd.vendor.as_deref(), Some("AMD"));
        assert_eq!(amd.properties["integrated"], "true");
        let unknown = &inventory.devices[1];
        assert_eq!(unknown.vendor, None);
        assert!(!unknown.properties.contains_key("pciAddress"));
        assert_eq!(inventory.devices[3].vendor.as_deref(), Some("Intel"));
        assert!(inventory.sensors.is_empty());
        assert_eq!(p.poll().unwrap(), Vec::<Option<f64>>::new());
    }

    #[test]
    fn experimental_flag_comes_from_the_owning_layer() {
        use GpuField::*;
        let script = Arc::new(Mutex::new(Script {
            values: vec![[(TemperatureHotspot, 70.0), (TemperatureCore, 60.0)].into()],
            ..Default::default()
        }));
        let nvapi = FakeLayer {
            source: Source::Nvapi,
            experimental: vec![TemperatureHotspot],
            script,
        };
        let (mut p, _) = provider(
            vec![nvidia()],
            Vec::new(),
            vec![Box::new(nvapi)],
            &VendorSwitch::new(true),
        );
        let inventory = p.discover().unwrap();
        let flags: Vec<_> = inventory
            .sensors
            .iter()
            .map(|s| (s.id.as_str(), s.experimental))
            .collect();
        assert_eq!(
            flags,
            vec![
                ("gpu/pci-0000:01:00.0/temperature/core", false),
                ("gpu/pci-0000:01:00.0/temperature/hotspot", true),
            ]
        );
    }

    #[test]
    fn rediscover_from_a_layer_is_propagated() {
        let (d3dkmt, script) = fake(Source::D3dkmt, &[&[(GpuField::TemperatureCore, 50.0)]]);
        let (mut p, _) = provider(
            vec![nvidia()],
            vec![d3dkmt],
            Vec::new(),
            &VendorSwitch::new(false),
        );
        p.discover().unwrap();
        script
            .lock()
            .unwrap()
            .errors
            .push(ProviderError::Rediscover);
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
        p.discover().unwrap();
        assert_eq!(p.poll().unwrap(), vec![Some(50.0)]);
    }

    #[test]
    fn failing_layer_blanks_only_its_own_fields() {
        use GpuField::*;
        let (d3dkmt, script) = fake(Source::D3dkmt, &[&[(TemperatureCore, 50.0)]]);
        let (pdh, _) = fake(Source::Pdh, &[&[(LoadCore, 30.0)]]);
        let (mut p, _) = provider(
            vec![nvidia()],
            vec![d3dkmt, pdh],
            Vec::new(),
            &VendorSwitch::new(false),
        );
        p.discover().unwrap();
        script
            .lock()
            .unwrap()
            .errors
            .push(ProviderError::Failed("boom".into()));
        assert_eq!(p.poll().unwrap(), vec![Some(30.0), None]);
        assert_eq!(p.poll().unwrap(), vec![Some(30.0), Some(50.0)]);
    }

    #[test]
    fn sample_with_a_wrong_adapter_count_counts_as_missing() {
        let (d3dkmt, script) = fake(Source::D3dkmt, &[&[(GpuField::TemperatureCore, 50.0)]]);
        let (pdh, _) = fake(Source::Pdh, &[&[(GpuField::LoadCore, 30.0)]]);
        let (mut p, _) = provider(
            vec![nvidia()],
            vec![d3dkmt, pdh],
            Vec::new(),
            &VendorSwitch::new(false),
        );
        p.discover().unwrap();
        script.lock().unwrap().wrong_len = Some(2);
        assert_eq!(p.poll().unwrap(), vec![Some(30.0), None]);
    }

    #[test]
    fn missing_vendor_layer_keeps_base_fields() {
        // Vendor libraries absent (make_vendor returns no layer): base layers still report.
        use GpuField::*;
        let (d3dkmt, _) = fake(Source::D3dkmt, &[&[(TemperatureCore, 50.0)]]);
        let (dxgi, _) = fake(Source::Dxgi, &[&[(MemoryDedicatedTotal, 17_171_480_576.0)]]);
        let (mut p, made) = provider(
            vec![nvidia()],
            vec![d3dkmt, dxgi],
            Vec::new(),
            &VendorSwitch::new(true),
        );
        let inventory = p.discover().unwrap();
        assert_eq!(made.load(Ordering::SeqCst), 1);
        assert_eq!(
            ids_and_sources(&inventory),
            vec![
                (
                    "gpu/pci-0000:01:00.0/data/memory-dedicated-total",
                    Source::Dxgi
                ),
                ("gpu/pci-0000:01:00.0/temperature/core", Source::D3dkmt),
            ]
        );
        assert_eq!(p.poll().unwrap(), vec![Some(17_171_480_576.0), Some(50.0)]);
    }

    #[test]
    fn switch_off_never_loads_vendor_layers() {
        let (nvml, nvml_script) = fake(Source::Nvml, &[&[(GpuField::TemperatureCore, 54.0)]]);
        let (d3dkmt, _) = fake(Source::D3dkmt, &[&[(GpuField::TemperatureCore, 50.0)]]);
        let (mut p, made) = provider(
            vec![nvidia()],
            vec![d3dkmt],
            vec![nvml],
            &VendorSwitch::new(false),
        );
        for _ in 0..2 {
            let inventory = p.discover().unwrap();
            assert_eq!(
                ids_and_sources(&inventory),
                vec![("gpu/pci-0000:01:00.0/temperature/core", Source::D3dkmt)]
            );
            assert_eq!(p.poll().unwrap(), vec![Some(50.0)]);
        }
        assert_eq!(made.load(Ordering::SeqCst), 0);
        assert_eq!(nvml_script.lock().unwrap().attach_calls, 0);
    }

    #[test]
    fn enabling_switch_triggers_rediscover() {
        let switch = VendorSwitch::new(false);
        let (nvml, nvml_script) = fake(Source::Nvml, &[&[(GpuField::TemperatureCore, 54.0)]]);
        let (d3dkmt, d3dkmt_script) = fake(Source::D3dkmt, &[&[(GpuField::TemperatureCore, 50.0)]]);
        let (mut p, made) = provider(vec![nvidia()], vec![d3dkmt], vec![nvml], &switch);
        p.discover().unwrap();
        assert_eq!(p.poll().unwrap(), vec![Some(50.0)]);

        switch.clone().enable(); // e.g. the UI's "Re-enable" button, on another clone
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
        let inventory = p.discover().unwrap();
        assert_eq!(
            ids_and_sources(&inventory),
            vec![("gpu/pci-0000:01:00.0/temperature/core", Source::Nvml)]
        );
        assert_eq!(p.poll().unwrap(), vec![Some(54.0)]);

        // A later rediscovery reuses the same vendor layers: no second load (D1).
        d3dkmt_script
            .lock()
            .unwrap()
            .errors
            .push(ProviderError::Rediscover);
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
        p.discover().unwrap();
        assert_eq!(p.poll().unwrap(), vec![Some(54.0)]);
        assert_eq!(made.load(Ordering::SeqCst), 1);
        assert_eq!(nvml_script.lock().unwrap().attach_calls, 2);
    }

    #[test]
    fn zero_adapters_give_an_empty_inventory() {
        let (d3dkmt, _) = fake(Source::D3dkmt, &[]);
        let (mut p, _) = provider(
            Vec::new(),
            vec![d3dkmt],
            Vec::new(),
            &VendorSwitch::new(true),
        );
        assert_eq!(p.discover().unwrap(), Inventory::default());
        assert_eq!(p.poll().unwrap(), Vec::<Option<f64>>::new());
    }

    #[test]
    fn enumeration_error_is_returned_and_loads_nothing() {
        let made = Arc::new(AtomicUsize::new(0));
        let counter = made.clone();
        let mut p = GpuProvider::with_layers(
            Box::new(|| Err(ProviderError::Failed("DXGI unavailable".into()))),
            Vec::new(),
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Vec::new()
            }),
            VendorSwitch::new(true),
        );
        assert_eq!(
            p.discover(),
            Err(ProviderError::Failed("DXGI unavailable".into()))
        );
        assert_eq!(made.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn vendor_switch_is_shared_between_clones() {
        assert!(!VendorSwitch::default().enabled());
        let a = VendorSwitch::new(false);
        let b = a.clone();
        b.enable();
        assert!(a.enabled());
        assert!(VendorSwitch::new(true).enabled());
    }

    fn props(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn layer_properties_are_merged_by_priority() {
        let (nvml, nvml_script) = fake(Source::Nvml, &[&[], &[]]);
        let (pnp, pnp_script) = fake(Source::Pnp, &[&[], &[]]);
        nvml_script.lock().unwrap().properties = vec![props(&[
            ("pcieMaxGen", "4"),
            ("powerLimitDefaultW", "320"),
            ("pciAddress", "9999:99:99.9"),
        ])];
        pnp_script.lock().unwrap().properties = vec![
            props(&[("pcieMaxGen", "3"), ("pcieMaxWidth", "16")]),
            props(&[("pcieMaxGen", "4"), ("pcieMaxWidth", "16")]),
        ];
        let (mut p, _) = provider(
            vec![nvidia(), amd_igpu()],
            vec![pnp],
            vec![nvml],
            &VendorSwitch::new(true),
        );
        let inventory = p.discover().unwrap();
        let nv = &inventory.devices[0].properties;
        assert_eq!(nv["pcieMaxGen"], "4", "NVML outranks PnP");
        assert_eq!(nv["pcieMaxWidth"], "16", "PnP fills the gap");
        assert_eq!(nv["powerLimitDefaultW"], "320");
        assert_eq!(nv["pciAddress"], "0000:01:00.0", "not overridable");
        assert_eq!(nv["integrated"], "false");
        let amd = &inventory.devices[1].properties;
        assert_eq!(
            amd,
            &props(&[
                ("integrated", "true"),
                ("pciAddress", "0000:11:00.0"),
                ("pcieMaxGen", "4"),
                ("pcieMaxWidth", "16"),
            ])
        );
        assert!(inventory.sensors.is_empty(), "properties declare no sensor");
    }

    #[test]
    fn safe_mode_keeps_base_layer_properties_only() {
        let (nvml, nvml_script) = fake(Source::Nvml, &[&[]]);
        let (pnp, pnp_script) = fake(Source::Pnp, &[&[]]);
        nvml_script.lock().unwrap().properties = vec![props(&[("tempMaxC", "90")])];
        pnp_script.lock().unwrap().properties = vec![props(&[("pcieMaxGen", "4")])];
        let (mut p, _) = provider(
            vec![nvidia()],
            vec![pnp],
            vec![nvml],
            &VendorSwitch::new(false),
        );
        let device = &p.discover().unwrap().devices[0];
        assert_eq!(device.properties["pcieMaxGen"], "4");
        assert!(!device.properties.contains_key("tempMaxC"));
    }

    #[test]
    fn pci_address_is_kept_per_luid_when_the_kernel_query_fails() {
        let topology = Arc::new(Mutex::new(vec![nvidia()]));
        let enumerated = topology.clone();
        let mut gpu = GpuProvider::with_layers(
            Box::new(move || Ok(enumerated.lock().unwrap().clone())),
            vec![],
            Box::new(Vec::new),
            VendorSwitch::new(false),
        );
        assert_eq!(
            gpu.discover().unwrap().devices[0].id,
            "gpu/pci-0000:01:00.0"
        );

        // The next enumeration loses the PCI address of the same adapter (same LUID).
        topology.lock().unwrap()[0].pci = None;
        gpu.state.topology_checked =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
        assert_eq!(gpu.poll(), Ok(vec![]), "not a topology change");
        let device = &gpu.discover().unwrap().devices[0];
        assert_eq!(device.id, "gpu/pci-0000:01:00.0");
        assert_eq!(device.properties["pciAddress"], "0000:01:00.0");

        // A different LUID without an address still gets the ordinal id.
        let mut other = nvidia();
        other.luid = 0x4242;
        other.pci = None;
        topology.lock().unwrap().push(other);
        let ids: Vec<_> = gpu
            .discover()
            .unwrap()
            .devices
            .iter()
            .map(|d| d.id.clone())
            .collect();
        assert_eq!(ids, ["gpu/pci-0000:01:00.0", "gpu/ven-10de-dev-2704-0"]);
    }

    #[test]
    fn restore_pci_records_and_restores_by_luid() {
        let mut known = HashMap::new();
        let mut first = [nvidia(), amd_igpu()];
        restore_pci(&mut known, &mut first);
        assert_eq!(known.len(), 2);
        let mut second = [nvidia(), amd_igpu()];
        second[1].pci = None;
        restore_pci(&mut known, &mut second);
        assert_eq!(second[1].pci, amd_igpu().pci);
        let mut unknown = [virtual_adapter(0x1414)];
        restore_pci(&mut known, &mut unknown);
        assert_eq!(unknown[0].pci, None);
    }

    #[test]
    fn topology_changes_are_detected_without_live_handles() {
        let topology = Arc::new(Mutex::new(Vec::<Adapter>::new()));
        let enumerated = topology.clone();
        let mut gpu = GpuProvider::with_layers(
            Box::new(move || Ok(enumerated.lock().unwrap().clone())),
            vec![],
            Box::new(Vec::new),
            VendorSwitch::new(false),
        );
        assert!(gpu.discover().unwrap().devices.is_empty());
        topology.lock().unwrap().push(virtual_adapter(0x10DE));
        gpu.state.topology_checked =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
        assert_eq!(gpu.poll(), Err(ProviderError::Rediscover));
        assert_eq!(gpu.discover().unwrap().devices.len(), 1);
        gpu.state.topology_checked =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
        assert_eq!(gpu.poll(), Ok(vec![])); // unchanged topology does not rediscover
        topology.lock().unwrap().clear();
        gpu.state.topology_checked =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
        assert_eq!(gpu.poll(), Err(ProviderError::Rediscover));
        assert!(gpu.discover().unwrap().devices.is_empty());
    }
}
