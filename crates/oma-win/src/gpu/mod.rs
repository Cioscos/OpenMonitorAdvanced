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
pub(crate) mod processes;
pub(crate) mod procname;
pub(crate) mod trim;

pub use processes::{GpuProcess, GpuProcessTable};

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

use oma_core::merge;
use oma_core::model::{Device, DeviceKind, Label, Sensor};
use oma_core::provider::{Inventory, Provider, ProviderError};

use adapter::{Adapter, PciAddress};
use field::GpuField;
use layer::{GpuLayer, Readings};

/// A GPU vendor library, in merge priority order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vendor {
    Nvml,
    Nvapi,
    Adl,
    Igcl,
}

impl Vendor {
    const ALL: [Vendor; 4] = [Vendor::Nvml, Vendor::Nvapi, Vendor::Adl, Vendor::Igcl];

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// Which vendor libraries are switched on: one bit per [`Vendor`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct VendorMask(u8);

impl VendorMask {
    pub const NONE: VendorMask = VendorMask(0);
    pub const ALL: VendorMask = VendorMask(0b1111);

    pub fn contains(self, vendor: Vendor) -> bool {
        self.0 & vendor.bit() != 0
    }

    #[must_use]
    pub fn with(self, vendor: Vendor, on: bool) -> Self {
        if on {
            Self(self.0 | vendor.bit())
        } else {
            Self(self.0 & !vendor.bit())
        }
    }
}

/// Shared switches for the GPU vendor libraries. The master is safe mode
/// (spec §8) and wins over the per-library switches of the settings
/// (spec M5 §2.5). Every operation is a single atomic access, so any thread
/// may call them; the GPU provider notices a change on its next poll.
#[derive(Debug, Clone, Default)]
pub struct VendorSwitch(Arc<Switches>);

#[derive(Debug, Default)]
struct Switches {
    master: AtomicBool,
    libraries: AtomicU8,
}

impl VendorSwitch {
    /// `master` is `!safe_mode`; `libraries` are the per-library switches.
    pub fn new(master: bool, libraries: VendorMask) -> Self {
        Self(Arc::new(Switches {
            master: AtomicBool::new(master),
            libraries: AtomicU8::new(libraries.0),
        }))
    }

    /// The master switch: false while in safe mode.
    pub fn enabled(&self) -> bool {
        self.0.master.load(Ordering::Acquire)
    }

    /// Turns the master on for every clone of this switch ("Re-enable"); the
    /// per-library switches still apply.
    pub fn enable(&self) {
        self.0.master.store(true, Ordering::Release);
    }

    /// Replaces the per-library switches. A library already loaded stays
    /// loaded (D1): switching it off only leaves it out of the next discovery.
    pub fn set_libraries(&self, libraries: VendorMask) {
        self.0.libraries.store(libraries.0, Ordering::Release);
    }

    /// The libraries that may be used now: the switches while the master is
    /// on, none in safe mode.
    pub fn effective(&self) -> VendorMask {
        if self.enabled() {
            VendorMask(self.0.libraries.load(Ordering::Acquire))
        } else {
            VendorMask::NONE
        }
    }
}

type Enumerate = Box<dyn FnMut() -> Result<Vec<Adapter>, ProviderError> + Send>;
/// Loads one vendor library and wraps it in its layer; `None` when the
/// library is absent or fails to initialise. Called at most once.
type MakeLayer = Box<dyn FnOnce() -> Option<Box<dyn GpuLayer>> + Send>;

/// One vendor library of the provider: created by the first discovery that
/// sees it switched on, then kept for the process lifetime (D1).
enum VendorSlot {
    /// Not created yet.
    Pending(MakeLayer),
    /// Creation was tried: the library is absent or failed to initialise.
    Absent,
    Loaded(Box<dyn GpuLayer>),
}

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
    /// Effective vendor mask seen at discover; a change means rediscover.
    vendor_mask: VendorMask,
    /// One entry per sensor, in inventory order.
    slots: Vec<Slot>,
    /// Per active layer: already warned about a failing sample in this streak.
    failing: Vec<bool>,
}

pub struct GpuProvider {
    enumerate: Enumerate,
    base: Vec<Box<dyn GpuLayer>>,
    /// One slot per vendor library, in [`Vendor::ALL`] order.
    vendor: Vec<VendorSlot>,
    switch: VendorSwitch,
    state: State,
    /// PCI address last seen per LUID, kept across discovers (see `restore_pci`).
    known_pci: HashMap<u64, PciAddress>,
    /// Per-process table filled by the PDH layer; `discover` maps device ids to LUIDs.
    processes: GpuProcessTable,
}

impl GpuProvider {
    /// Test/assembly constructor. A vendor layer is created by its `MakeLayer`
    /// on the first discover that sees its library in the effective mask, and
    /// then stays for the process lifetime (D1), used or not.
    pub(crate) fn with_layers(
        enumerate: Enumerate,
        base: Vec<Box<dyn GpuLayer>>, // priority order: d3dkmt, dxgi, pdh, pnp
        make_vendor: [(Vendor, MakeLayer); 4],
        switch: VendorSwitch,
    ) -> Self {
        let mut make_vendor = make_vendor;
        make_vendor.sort_by_key(|(vendor, _)| *vendor as u8);
        Self {
            enumerate,
            base,
            vendor: make_vendor
                .into_iter()
                .map(|(_, make)| VendorSlot::Pending(make))
                .collect(),
            switch,
            state: State::default(),
            known_pci: HashMap::new(),
            processes: GpuProcessTable::default(),
        }
    }

    /// Creates the layers of the libraries in `mask` that were never tried.
    fn load_vendors(&mut self, mask: VendorMask) {
        for (slot, vendor) in self.vendor.iter_mut().zip(Vendor::ALL) {
            if mask.contains(vendor) && matches!(slot, VendorSlot::Pending(_)) {
                let VendorSlot::Pending(make) = std::mem::replace(slot, VendorSlot::Absent) else {
                    continue;
                };
                if let Some(layer) = make() {
                    *slot = VendorSlot::Loaded(layer);
                }
            }
        }
    }

    /// Active layers in priority order: the loaded vendor layers whose library
    /// is in `mask` (nvml, nvapi, adl, igcl), then the base layers.
    fn active_layers(&mut self, mask: VendorMask) -> Vec<&mut Box<dyn GpuLayer>> {
        let vendor =
            self.vendor
                .iter_mut()
                .zip(Vendor::ALL)
                .filter_map(|(slot, vendor)| match slot {
                    VendorSlot::Loaded(layer) if mask.contains(vendor) => Some(layer),
                    _ => None,
                });
        vendor.chain(self.base.iter_mut()).collect()
    }
}

impl GpuProvider {
    /// The real provider: DXGI/DXCore/D3DKMT enumeration, the base layers
    /// (D3DKMT, DXGI, PDH, PnP) always on, and the vendor libraries (NVML,
    /// NVAPI, ADL, IGCL) each loaded on the first discover that sees it
    /// switched on in `switch`.
    /// The PDH layer publishes the per-process GPU usage into `processes`.
    pub fn new(switch: VendorSwitch, processes: GpuProcessTable) -> Self {
        let mut provider = Self::with_layers(
            Box::new(enumerate::enumerate),
            vec![
                Box::new(d3dkmt::D3dkmtLayer::default()),
                Box::new(dxgi::DxgiLayer::default()),
                Box::new(pdh::PdhLayer::new(processes.clone())),
                Box::new(pnp::PnpLayer::default()),
            ],
            [
                (Vendor::Nvml, load_vendor(nvml::NvmlLayer::load)),
                (Vendor::Nvapi, load_vendor(nvapi::NvapiLayer::load)),
                (Vendor::Adl, load_vendor(adl::AdlLayer::load)),
                (Vendor::Igcl, load_vendor(igcl::IgclLayer::load)),
            ],
            switch,
        );
        provider.processes = processes;
        provider
    }
}

/// Wraps the loader of one vendor library. A library that is absent or fails
/// to initialise gives no layer. Called at most once per provider: a loaded
/// library stays for the process lifetime (decision D1).
fn load_vendor<L: GpuLayer + 'static>(load: fn() -> Option<L>) -> MakeLayer {
    Box::new(move || {
        let layer = load().map(|layer| Box::new(layer) as Box<dyn GpuLayer>);
        tracing::info!(
            source = ?layer.as_ref().map(|layer| layer.source()),
            "GPU vendor library loaded"
        );
        // A vendor DLL may have installed its own top-level exception filter.
        crate::crash::rearm_crash_marker();
        layer
    })
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
        self.processes.publish(HashMap::new());
        self.processes.set_devices(Vec::new());
        self.state = State::default();
        let mut adapters = (self.enumerate)()?;
        restore_pci(&mut self.known_pci, &mut adapters);
        let vendor_mask = self.switch.effective();
        self.load_vendors(vendor_mask);
        let count = adapters.len();
        let mut layers = self.active_layers(vendor_mask);
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
        let mut luids = Vec::new();
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
            luids.push((id.clone(), adapter.luid));
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
        self.processes.set_devices(luids);
        self.state = State {
            adapters: count,
            topology: adapters,
            topology_checked: Some(std::time::Instant::now()),
            vendor_mask,
            slots,
            failing,
        };
        Ok(inventory)
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let result = self.poll_inner();
        if result.is_err() {
            // Also clear on failures before PDH is reached, and on Rediscover.
            self.processes.publish(HashMap::new());
        }
        result
    }
}

impl GpuProvider {
    fn poll_inner(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        // A change of the vendor libraries in use (a per-library switch, or
        // "Riattiva") is observed only here. While the engine backs this provider
        // off (repeated failures, or repeated Rediscovers per decision D8) poll is
        // not called, so a change can take up to the 60 s maximum backoff to be
        // picked up.
        if self.switch.effective() != self.state.vendor_mask {
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
        let vendor_mask = self.state.vendor_mask;
        let mut samples: Vec<Option<Vec<Readings>>> = Vec::new();
        let mut failed = Vec::new();
        for layer in self.active_layers(vendor_mask) {
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
        /// Times a `FakeLayer` sharing this script was dropped.
        drops: usize,
        /// Per adapter: the static properties the layer reports.
        properties: Vec<BTreeMap<String, String>>,
    }

    struct FakeLayer {
        source: Source,
        experimental: Vec<GpuField>,
        script: Arc<Mutex<Script>>,
    }

    impl Drop for FakeLayer {
        fn drop(&mut self) {
            self.script.lock().unwrap().drops += 1;
        }
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

    type Shared = Arc<Mutex<Script>>;

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

    /// Creation counts per vendor library, shared with the `make` closures.
    #[derive(Clone, Default)]
    struct Made(Arc<[AtomicUsize; 4]>);

    impl Made {
        fn of(&self, vendor: Vendor) -> usize {
            self.0[vendor as usize].load(Ordering::SeqCst)
        }

        fn total(&self) -> usize {
            self.0.iter().map(|n| n.load(Ordering::SeqCst)).sum()
        }
    }

    /// One maker per vendor library: it counts its calls and hands out the layer
    /// whose `source()` matches (none for a library "not installed").
    fn makers(layers: Vec<Box<dyn GpuLayer>>, made: &Made) -> [(Vendor, MakeLayer); 4] {
        let mut layers: Vec<Option<Box<dyn GpuLayer>>> = layers.into_iter().map(Some).collect();
        let mut take = |source: Source| {
            layers
                .iter_mut()
                .find(|slot| slot.as_ref().is_some_and(|l| l.source() == source))
                .and_then(Option::take)
        };
        let mut maker = |vendor: Vendor, source: Source| -> (Vendor, MakeLayer) {
            let layer = take(source);
            let made = made.clone();
            (
                vendor,
                Box::new(move || {
                    made.0[vendor as usize].fetch_add(1, Ordering::SeqCst);
                    layer
                }),
            )
        };
        [
            maker(Vendor::Nvml, Source::Nvml),
            maker(Vendor::Nvapi, Source::Nvapi),
            maker(Vendor::Adl, Source::Adl),
            maker(Vendor::Igcl, Source::Igcl),
        ]
    }

    /// Builds a provider; vendor layers are matched to their library by `source()`.
    fn provider(
        adapters: Vec<Adapter>,
        base: Vec<Box<dyn GpuLayer>>,
        vendor: Vec<Box<dyn GpuLayer>>,
        switch: &VendorSwitch,
    ) -> (GpuProvider, Made) {
        let made = Made::default();
        let provider = GpuProvider::with_layers(
            Box::new(move || Ok(adapters.clone())),
            base,
            makers(vendor, &made),
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
            &VendorSwitch::new(true, VendorMask::ALL),
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
            &VendorSwitch::new(true, VendorMask::ALL),
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
            &VendorSwitch::new(false, VendorMask::ALL),
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
            &VendorSwitch::new(true, VendorMask::ALL),
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
            &VendorSwitch::new(false, VendorMask::ALL),
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
            &VendorSwitch::new(false, VendorMask::ALL),
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
            &VendorSwitch::new(false, VendorMask::ALL),
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
            &VendorSwitch::new(true, VendorMask::ALL),
        );
        let inventory = p.discover().unwrap();
        assert_eq!(
            made.total(),
            4,
            "each library is tried once and found absent"
        );
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
            &VendorSwitch::new(false, VendorMask::ALL),
        );
        for _ in 0..2 {
            let inventory = p.discover().unwrap();
            assert_eq!(
                ids_and_sources(&inventory),
                vec![("gpu/pci-0000:01:00.0/temperature/core", Source::D3dkmt)]
            );
            assert_eq!(p.poll().unwrap(), vec![Some(50.0)]);
        }
        assert_eq!(made.total(), 0);
        assert_eq!(nvml_script.lock().unwrap().attach_calls, 0);
    }

    #[test]
    fn enabling_switch_triggers_rediscover() {
        let switch = VendorSwitch::new(false, VendorMask::ALL);
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
        assert_eq!(made.of(Vendor::Nvml), 1);
        assert_eq!(nvml_script.lock().unwrap().attach_calls, 2);
    }

    #[test]
    fn zero_adapters_give_an_empty_inventory() {
        let (d3dkmt, _) = fake(Source::D3dkmt, &[]);
        let (mut p, _) = provider(
            Vec::new(),
            vec![d3dkmt],
            Vec::new(),
            &VendorSwitch::new(true, VendorMask::ALL),
        );
        assert_eq!(p.discover().unwrap(), Inventory::default());
        assert_eq!(p.poll().unwrap(), Vec::<Option<f64>>::new());
    }

    #[test]
    fn enumeration_error_is_returned_and_loads_nothing() {
        let made = Made::default();
        let mut p = GpuProvider::with_layers(
            Box::new(|| Err(ProviderError::Failed("DXGI unavailable".into()))),
            Vec::new(),
            makers(Vec::new(), &made),
            VendorSwitch::new(true, VendorMask::ALL),
        );
        assert_eq!(
            p.discover(),
            Err(ProviderError::Failed("DXGI unavailable".into()))
        );
        assert_eq!(made.total(), 0);
    }

    #[test]
    fn vendor_switch_is_shared_between_clones() {
        assert!(!VendorSwitch::default().enabled());
        assert_eq!(VendorSwitch::default().effective(), VendorMask::NONE);
        let a = VendorSwitch::new(false, VendorMask::ALL);
        let b = a.clone();
        b.enable();
        assert!(a.enabled());
        b.set_libraries(VendorMask::NONE.with(Vendor::Adl, true));
        assert_eq!(a.effective(), VendorMask::NONE.with(Vendor::Adl, true));
        assert!(VendorSwitch::new(true, VendorMask::ALL).enabled());
    }

    #[test]
    fn vendor_mask_has_one_bit_per_library() {
        let vendors = [Vendor::Nvml, Vendor::Nvapi, Vendor::Adl, Vendor::Igcl];
        for v in vendors {
            assert!(VendorMask::ALL.contains(v));
            assert!(!VendorMask::NONE.contains(v));
            let only = VendorMask::NONE.with(v, true);
            for other in vendors {
                assert_eq!(only.contains(other), other == v);
            }
            assert_eq!(
                VendorMask::ALL.with(v, false).with(v, true),
                VendorMask::ALL
            );
        }
        assert_eq!(VendorMask::default(), VendorMask::NONE);
    }

    /// All four libraries installed, each with a distinct temperature.
    fn four_vendors() -> Vec<(Box<dyn GpuLayer>, Shared)> {
        [
            (Source::Nvml, 54.0),
            (Source::Nvapi, 55.0),
            (Source::Adl, 56.0),
            (Source::Igcl, 57.0),
        ]
        .into_iter()
        .map(|(source, celsius)| fake(source, &[&[(GpuField::TemperatureCore, celsius)]]))
        .collect()
    }

    fn temperature_source(inventory: &Inventory) -> Source {
        inventory
            .sensors
            .iter()
            .find(|s| s.id.ends_with("temperature/core"))
            .expect("a core temperature")
            .source
    }

    #[test]
    fn disabled_vendor_is_never_created() {
        let switch = VendorSwitch::new(true, VendorMask::ALL.with(Vendor::Nvml, false));
        let (layers, _): (Vec<_>, Vec<_>) = four_vendors().into_iter().unzip();
        let (mut p, made) = provider(vec![nvidia()], Vec::new(), layers, &switch);
        let inventory = p.discover().unwrap();
        assert_eq!(made.of(Vendor::Nvml), 0);
        for v in [Vendor::Nvapi, Vendor::Adl, Vendor::Igcl] {
            assert_eq!(made.of(v), 1);
        }
        assert_eq!(temperature_source(&inventory), Source::Nvapi);
    }

    #[test]
    fn enabling_one_vendor_creates_only_that_layer() {
        let switch = VendorSwitch::new(true, VendorMask::NONE.with(Vendor::Adl, true));
        let (layers, scripts): (Vec<_>, Vec<_>) = four_vendors().into_iter().unzip();
        let (mut p, made) = provider(vec![nvidia()], Vec::new(), layers, &switch);
        let inventory = p.discover().unwrap();
        assert_eq!(made.total(), 1);
        assert_eq!(made.of(Vendor::Adl), 1);
        assert_eq!(temperature_source(&inventory), Source::Adl);
        assert_eq!(p.poll().unwrap(), vec![Some(56.0)]);
        let attached: Vec<_> = scripts
            .iter()
            .map(|s| s.lock().unwrap().attach_calls)
            .collect();
        assert_eq!(attached, [0, 0, 1, 0]);
    }

    #[test]
    fn disabling_a_loaded_vendor_excludes_it_without_dropping() {
        let switch = VendorSwitch::new(true, VendorMask::ALL);
        let (nvml, nvml_script) = fake(Source::Nvml, &[&[(GpuField::TemperatureCore, 54.0)]]);
        let (d3dkmt, _) = fake(Source::D3dkmt, &[&[(GpuField::TemperatureCore, 50.0)]]);
        let (mut p, made) = provider(vec![nvidia()], vec![d3dkmt], vec![nvml], &switch);
        let inventory = p.discover().unwrap();
        assert_eq!(temperature_source(&inventory), Source::Nvml);

        switch.set_libraries(VendorMask::ALL.with(Vendor::Nvml, false));
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
        let inventory = p.discover().unwrap();
        assert_eq!(temperature_source(&inventory), Source::D3dkmt);
        assert_eq!(p.poll().unwrap(), vec![Some(50.0)]);
        {
            let script = nvml_script.lock().unwrap();
            assert_eq!(script.drops, 0, "a loaded library is never unloaded");
            assert_eq!(script.attach_calls, 1, "excluded from discovery");
            assert_eq!(script.sample_calls, 0, "and from polling");
        }

        // Switching it back on reuses the layer: no second creation, no drop.
        switch.set_libraries(VendorMask::ALL);
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
        let inventory = p.discover().unwrap();
        assert_eq!(temperature_source(&inventory), Source::Nvml);
        assert_eq!(made.of(Vendor::Nvml), 1);
        assert_eq!(nvml_script.lock().unwrap().drops, 0);
    }

    #[test]
    fn mask_change_requests_rediscover() {
        let switch = VendorSwitch::new(true, VendorMask::ALL);
        let (d3dkmt, _) = fake(Source::D3dkmt, &[&[(GpuField::TemperatureCore, 50.0)]]);
        let (mut p, _) = provider(vec![nvidia()], vec![d3dkmt], Vec::new(), &switch);
        p.discover().unwrap();
        assert_eq!(p.poll().unwrap(), vec![Some(50.0)]);
        switch.set_libraries(VendorMask::ALL); // same mask: nothing to do
        assert_eq!(p.poll().unwrap(), vec![Some(50.0)]);
        switch.set_libraries(VendorMask::ALL.with(Vendor::Igcl, false));
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
        p.discover().unwrap();
        assert_eq!(p.poll().unwrap(), vec![Some(50.0)]);
    }

    #[test]
    fn safe_mode_master_overrides_libraries() {
        let switch = VendorSwitch::new(false, VendorMask::ALL);
        assert!(!switch.enabled());
        assert_eq!(switch.effective(), VendorMask::NONE);
        let (layers, _): (Vec<_>, Vec<_>) = four_vendors().into_iter().unzip();
        let (d3dkmt, _) = fake(Source::D3dkmt, &[&[(GpuField::TemperatureCore, 50.0)]]);
        let (mut p, made) = provider(vec![nvidia()], vec![d3dkmt], layers, &switch);
        let inventory = p.discover().unwrap();
        assert_eq!(temperature_source(&inventory), Source::D3dkmt);
        // Changing switches in safe mode changes nothing that is visible.
        switch.set_libraries(VendorMask::NONE);
        switch.set_libraries(VendorMask::ALL);
        assert_eq!(p.poll().unwrap(), vec![Some(50.0)]);
        assert_eq!(made.total(), 0);
    }

    #[test]
    fn reenable_respects_library_switches() {
        let switch = VendorSwitch::new(false, VendorMask::NONE.with(Vendor::Igcl, true));
        let (layers, _): (Vec<_>, Vec<_>) = four_vendors().into_iter().unzip();
        let (d3dkmt, _) = fake(Source::D3dkmt, &[&[(GpuField::TemperatureCore, 50.0)]]);
        let (mut p, made) = provider(vec![nvidia()], vec![d3dkmt], layers, &switch);
        p.discover().unwrap();
        assert_eq!(made.total(), 0);

        switch.enable();
        assert_eq!(
            switch.effective(),
            VendorMask::NONE.with(Vendor::Igcl, true)
        );
        assert_eq!(p.poll(), Err(ProviderError::Rediscover));
        let inventory = p.discover().unwrap();
        assert_eq!(temperature_source(&inventory), Source::Igcl);
        assert_eq!(made.total(), 1);
        assert_eq!(made.of(Vendor::Igcl), 1);
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
            &VendorSwitch::new(true, VendorMask::ALL),
        );
        let inventory = p.discover().unwrap();
        let nv = &inventory.devices[0].properties;
        assert_eq!(
            nv["pcieMaxGen"], "3",
            "PnP is the only source of the max link"
        );
        assert_eq!(
            nv["pcieMaxWidth"], "16",
            "PnP is the only source of the max link"
        );
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
            &VendorSwitch::new(false, VendorMask::ALL),
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
            makers(Vec::new(), &Made::default()),
            VendorSwitch::new(false, VendorMask::ALL),
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
    fn discover_maps_device_ids_to_luids_for_the_process_table() {
        let (mut p, _) = provider(
            vec![nvidia(), amd_igpu()],
            Vec::new(),
            Vec::new(),
            &VendorSwitch::new(false, VendorMask::ALL),
        );
        let table = GpuProcessTable::new();
        p.processes = table.clone();
        p.discover().unwrap();
        let dwm = GpuProcess {
            pid: 2096,
            name: "dwm.exe".to_owned(),
            load_percent: Some(1.0),
            engine: Some("3D".to_owned()),
            dedicated_bytes: Some(1 << 30),
            shared_bytes: None,
        };
        table.publish(HashMap::from([(nvidia().luid, vec![dwm.clone()])]));
        assert_eq!(table.processes("gpu/pci-0000:01:00.0"), vec![dwm]);
        assert!(table.processes("gpu/pci-0000:11:00.0").is_empty());
        assert!(table.processes("gpu/pci-0000:02:00.0").is_empty());
    }

    #[test]
    fn provider_enumeration_failures_clear_process_rows() {
        for fail_during_discover in [false, true] {
            let (mut p, _) = provider(
                vec![nvidia()],
                Vec::new(),
                Vec::new(),
                &VendorSwitch::new(false, VendorMask::ALL),
            );
            let table = GpuProcessTable::new();
            p.processes = table.clone();
            p.discover().unwrap();
            table.publish(HashMap::from([(
                nvidia().luid,
                vec![GpuProcess {
                    pid: 99,
                    name: "old.exe".into(),
                    load_percent: Some(50.0),
                    engine: Some("3D".into()),
                    dedicated_bytes: None,
                    shared_bytes: None,
                }],
            )]));
            assert_eq!(table.processes("gpu/pci-0000:01:00.0").len(), 1);
            p.enumerate = Box::new(|| Err(ProviderError::Failed("enumeration failed".into())));
            if fail_during_discover {
                assert!(p.discover().is_err());
            } else {
                p.state.topology_checked = None; // fail before the PDH layer runs
                assert!(p.poll().is_err());
            }
            assert!(table.processes("gpu/pci-0000:01:00.0").is_empty());
        }
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
            makers(Vec::new(), &Made::default()),
            VendorSwitch::new(false, VendorMask::ALL),
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
