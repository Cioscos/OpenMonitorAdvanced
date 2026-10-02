#![cfg(windows)]

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use oma_core::model::{Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use oma_win::cpu::CpuProvider;
use oma_win::gpu::{GpuProcessTable, GpuProvider, Vendor, VendorMask, VendorSwitch};
use oma_win::memory::MemoryProvider;
use oma_win::network::NetworkProvider;
use oma_win::storage::StorageProvider;

/// Label keys of the PDH rate sensors: the first poll after a discover only
/// primes their counters, so it must report `None` instead of a value
/// computed over a few milliseconds.
const PRIMED_RATE_KEYS: &[&str] = &[
    "cpu.load.total",
    "cpu.load.thread",
    "cpu.clock.effective",
    "storage.read",
    "storage.write",
    "storage.active",
    "gpu.load.core",
    "gpu.load.3d",
    "gpu.load.compute",
    "gpu.load.copy",
    "gpu.load.videoDecode",
    "gpu.load.videoEncode",
];

fn assert_first_poll_has_no_rates(inventory: &Inventory, first: &[Option<f64>]) {
    for (sensor, value) in inventory.sensors.iter().zip(first) {
        if PRIMED_RATE_KEYS.contains(&sensor.label.key.as_str()) {
            assert_eq!(
                *value, None,
                "{} must have no rate on the first poll",
                sensor.id
            );
        }
    }
}

/// Discovers, polls twice a second apart, and checks alignment. The first
/// poll after a discover only primes the PDH rate counters (fresh baseline,
/// no elapsed interval yet): its rate sensors must be `None`. The second poll
/// carries real rate values.
fn discover_and_poll(p: &mut dyn Provider) -> (Inventory, Vec<Option<f64>>) {
    discover_and_poll_once(p).expect("second poll asked for a rediscovery")
}

/// [`discover_and_poll`] for the storage provider alone: a hard disk at work
/// declares its temperature sensor after its first authorized read, through
/// a rediscovery on the second poll. Starts over once.
fn discover_and_poll_storage(p: &mut StorageProvider) -> (Inventory, Vec<Option<f64>>) {
    discover_and_poll_once(p)
        .unwrap_or_else(|| discover_and_poll_once(p).expect("a second rediscovery in a row"))
}

/// `None` when the second poll asks for a rediscovery.
fn discover_and_poll_once(p: &mut dyn Provider) -> Option<(Inventory, Vec<Option<f64>>)> {
    let inventory = p.discover().expect("discover");
    std::thread::sleep(Duration::from_millis(1_100));
    let first = p.poll().expect("first poll");
    assert_eq!(
        first.len(),
        inventory.sensors.len(),
        "values must align with sensors"
    );
    assert_first_poll_has_no_rates(&inventory, &first);
    std::thread::sleep(Duration::from_millis(1_100));
    let values = match p.poll() {
        Err(ProviderError::Rediscover) => return None,
        polled => polled.expect("second poll"),
    };
    assert_eq!(
        values.len(),
        inventory.sensors.len(),
        "values must align with sensors"
    );
    Some((inventory, values))
}

#[test]
#[ignore = "requires real Windows hardware"]
fn pdh_english_paths_resolve() {
    // Discovery adds every counter with PdhAddEnglishCounterW: on a non-English
    // Windows this fails if a localized API is used by mistake.
    CpuProvider::new()
        .discover()
        .expect("english PDH counter paths must resolve");
}

#[test]
#[ignore = "requires real Windows hardware"]
fn cpu_provider_reports_load_and_clock() {
    let mut p = CpuProvider::new();
    let (inventory, values) = discover_and_poll(&mut p);
    assert_eq!(inventory.devices.len(), 1);
    assert!(!inventory.devices[0].name.is_empty());
    let thread_sensors = inventory
        .sensors
        .iter()
        .filter(|s| s.id.contains("/load/thread-"))
        .count();
    assert!(thread_sensors > 0); // available_parallelism may be restricted by affinity/job limits.
    let total = values[0].expect("total load");
    assert!((0.0..=100.0).contains(&total));
    if let Some(clock) = values.last().copied().flatten() {
        assert!((0.0..=20_000.0).contains(&clock), "clock {clock} MHz");
    }
}

#[test]
#[ignore = "requires real Windows hardware"]
fn memory_provider_reports_usage() {
    let mut p = MemoryProvider;
    let (_, values) = discover_and_poll(&mut p);
    let pct = values[0].expect("load");
    assert!((0.0..=100.0).contains(&pct));
    let used = values[1].expect("used");
    let total = values[2].expect("total");
    assert!(total > 0.0 && used <= total);
}

#[test]
#[ignore = "requires real Windows hardware"]
fn storage_provider_reports_disks_and_volumes() {
    let mut p = StorageProvider::default();
    let (inventory, values) = discover_and_poll_storage(&mut p);
    assert!(!inventory.devices.is_empty(), "at least the system disk");
    for (sensor, value) in inventory.sensors.iter().zip(&values) {
        if sensor.label.key == "storage.volumeUsed" {
            let pct = value.expect("volume usage");
            assert!((0.0..=100.0).contains(&pct), "{} = {pct}", sensor.id);
        }
        if sensor.kind == SensorKind::Temperature {
            assert_eq!(sensor.unit, Unit::Celsius, "{}", sensor.id);
            // Read at discovery (a hard disk only declares its sensor after
            // an authorized read) and repeated until the 30 s refresh.
            let celsius = value.expect("disk temperature");
            assert!((5.0..=90.0).contains(&celsius), "{} = {celsius}", sensor.id);
        }
    }
    let temperatures = inventory
        .sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Temperature)
        .count();
    println!("{temperatures} disk temperature sensors");
    // Every disk says whether its SMART can be switched off on its own (Settings › Data sources).
    for device in &inventory.devices {
        let selectable = device
            .properties
            .get(oma_win::storage::SMART_SELECTABLE)
            .map(String::as_str);
        assert!(
            matches!(selectable, Some("true" | "false")),
            "{}: {selectable:?}",
            device.id
        );
        println!("{}: smartSelectable = {}", device.name, selectable.unwrap());
    }
    assert!(temperatures > 0, "at least one disk reports a temperature");
    for device in &inventory.devices {
        for key in ["tempWarningC", "tempCriticalC"] {
            if let Some(value) = device.properties.get(key) {
                let celsius: i16 = value.parse().expect("integer °C");
                assert!(
                    (40..=150).contains(&celsius),
                    "{} {key} = {celsius}",
                    device.id
                );
            }
        }
    }
}

/// The unprivileged core reads the NVMe SMART/Health log itself (spike M5
/// S1 §2.5): this machine has two NVMe disks and SATA disks.
#[test]
#[ignore = "requires real Windows hardware"]
fn reads_nvme_health_on_this_machine() {
    const HEALTH: [&str; 3] = [
        "flag/critical-warning",
        "percent/wear",
        "percent/available-spare",
    ];
    let mut p = StorageProvider::default();
    let (inventory, values) = discover_and_poll_storage(&mut p);
    let mut with_health = 0;
    for device in &inventory.devices {
        let found: Vec<(&Sensor, Option<f64>)> = inventory
            .sensors
            .iter()
            .zip(&values)
            .filter(|(s, _)| {
                s.device_id == device.id
                    && HEALTH.iter().any(|h| s.id == format!("{}/{h}", device.id))
            })
            .map(|(s, v)| (s, *v))
            .collect();
        println!("{}: {found:?}", device.name);
        if found.is_empty() {
            assert!(
                !device.properties.contains_key("availableSpareThresholdPct"),
                "{}",
                device.id
            );
            continue;
        }
        assert_eq!(found.len(), 3, "{}: all three or none", device.id);
        with_health += 1;
        for (sensor, value) in found {
            assert_eq!(sensor.source, Source::Win32);
            let value = value.unwrap_or_else(|| panic!("{} has a value", sensor.id));
            match sensor.id.rsplit('/').next() {
                Some("critical-warning") => assert_eq!(value, 0.0, "healthy disks"),
                Some("wear") => assert!((0.0..=255.0).contains(&value)),
                _ => assert!((0.0..=100.0).contains(&value)),
            }
        }
        let threshold: u8 = device.properties["availableSpareThresholdPct"]
            .parse()
            .expect("integer %");
        assert!(threshold <= 100);
    }
    assert_eq!(with_health, 2, "the two NVMe disks, never the SATA ones");
    assert!(
        inventory.devices.len() > with_health,
        "SATA disks have none"
    );
}

#[test]
#[ignore = "requires real Windows hardware"]
fn network_provider_values_align_with_sensors() {
    // CI runners may expose no physical adapter: only alignment is guaranteed.
    // discover() seeds fresh baselines (no priming), so the first poll after
    // discover must yield None for down/up; poll a second time to also observe
    // a real, non-negative rate.
    let mut p = NetworkProvider::default();
    let inventory = p.discover().expect("discover");
    assert_eq!(inventory.sensors.len(), inventory.devices.len() * 3);

    std::thread::sleep(Duration::from_millis(1_100));
    let first = p.poll().expect("first poll");
    assert_eq!(first.len(), inventory.sensors.len());
    for (sensor, value) in inventory.sensors.iter().zip(&first) {
        if sensor.label.key != "network.linkSpeed" {
            assert_eq!(
                *value, None,
                "{} must have no rate on the first poll",
                sensor.id
            );
        }
    }

    std::thread::sleep(Duration::from_millis(1_100));
    let second = p.poll().expect("second poll");
    for v in second.into_iter().flatten() {
        assert!(v >= 0.0);
    }
}

/// Vendor libraries are process-wide state: the GPU tests run one at a time.
static GPU_TESTS: Mutex<()> = Mutex::new(());

/// The development machine: RTX 4080 (discrete) and a Raphael iGPU (integrated).
const NVIDIA: &str = "gpu/pci-0000:01:00.0";
const AMD: &str = "gpu/pci-0000:11:00.0";
const VENDOR_SOURCES: [Source; 4] = [Source::Nvml, Source::Nvapi, Source::Adl, Source::Igcl];

/// Index and definition of the sensor with id `<device>/<rest>`.
fn gpu_sensor<'a>(inventory: &'a Inventory, device: &str, rest: &str) -> (usize, &'a Sensor) {
    let id = format!("{device}/{rest}");
    inventory
        .sensors
        .iter()
        .enumerate()
        .find(|(_, s)| s.id == id)
        .unwrap_or_else(|| panic!("missing sensor {id}"))
}

fn assert_gpu_devices(inventory: &Inventory) {
    let mut ids: Vec<&str> = inventory.devices.iter().map(|d| d.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, [NVIDIA, AMD]);
    for (id, vendor, pci, integrated) in [
        (NVIDIA, "NVIDIA", "0000:01:00.0", "false"),
        (AMD, "AMD", "0000:11:00.0", "true"),
    ] {
        let device = inventory
            .devices
            .iter()
            .find(|d| d.id == id)
            .expect("device");
        assert_eq!(device.vendor.as_deref(), Some(vendor), "{id}");
        assert_eq!(
            device.properties.get("pciAddress").map(String::as_str),
            Some(pci),
            "{id}"
        );
        assert_eq!(
            device.properties.get("integrated").map(String::as_str),
            Some(integrated),
            "{id}"
        );
    }
}

#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_provider_finds_both_gpus_with_merged_sources() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut p = GpuProvider::new(
        VendorSwitch::new(true, VendorMask::ALL),
        GpuProcessTable::new(),
    );
    let (inventory, values) = discover_and_poll(&mut p);
    assert_gpu_devices(&inventory);

    let (i, temperature) = gpu_sensor(&inventory, NVIDIA, "temperature/core");
    assert_eq!(temperature.source, Source::Nvml);
    let celsius = values[i].expect("NVIDIA core temperature");
    assert!((20.0..=100.0).contains(&celsius), "{celsius} °C");

    let (i, hotspot) = gpu_sensor(&inventory, NVIDIA, "temperature/hotspot");
    assert_eq!(hotspot.source, Source::Nvapi);
    assert!(
        hotspot.experimental,
        "NVAPI hotspot is an undocumented call"
    );
    let celsius = values[i].expect("NVIDIA hotspot temperature");
    assert!((20.0..=110.0).contains(&celsius), "{celsius} °C");

    let (i, power) = gpu_sensor(&inventory, NVIDIA, "power/board");
    assert_eq!(power.source, Source::Nvml);
    let watt = values[i].expect("NVIDIA board power");
    assert!((1.0..=600.0).contains(&watt), "{watt} W");

    let (i, load) = gpu_sensor(&inventory, NVIDIA, "load/core");
    assert_eq!(load.source, Source::Pdh);
    let pct = values[i].expect("NVIDIA load on the second poll");
    assert!((0.0..=100.0).contains(&pct), "{pct} %");

    let (i, temperature) = gpu_sensor(&inventory, AMD, "temperature/core");
    assert_eq!(temperature.source, Source::Adl);
    let celsius = values[i].expect("AMD core temperature");
    assert!((20.0..=100.0).contains(&celsius), "{celsius} °C");

    let (_, clock) = gpu_sensor(&inventory, AMD, "clock/core");
    assert_eq!(clock.source, Source::Adl);
    let (_, used) = gpu_sensor(&inventory, AMD, "data/memory-dedicated-used");
    assert_eq!(used.source, Source::Pdh);
}

#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_provider_in_safe_mode_uses_only_base_layers() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut p = GpuProvider::new(
        VendorSwitch::new(false, VendorMask::ALL),
        GpuProcessTable::new(),
    );
    let (inventory, values) = discover_and_poll(&mut p);
    assert_gpu_devices(&inventory);
    for sensor in &inventory.sensors {
        assert!(
            !VENDOR_SOURCES.contains(&sensor.source),
            "{} from {:?}",
            sensor.id,
            sensor.source
        );
        assert!(!sensor.experimental, "{}", sensor.id);
    }

    let (i, temperature) = gpu_sensor(&inventory, NVIDIA, "temperature/core");
    assert_eq!(temperature.source, Source::D3dkmt);
    let celsius = values[i].expect("NVIDIA core temperature from D3DKMT");
    assert!((20.0..=100.0).contains(&celsius), "{celsius} °C");

    let (_, total) = gpu_sensor(&inventory, NVIDIA, "data/memory-dedicated-total");
    assert_eq!(total.source, Source::Dxgi);

    let (_, temperature) = gpu_sensor(&inventory, AMD, "temperature/core");
    assert_eq!(temperature.source, Source::D3dkmt);
}

/// Value of device property `key` of GPU `id`.
fn gpu_property<'a>(inventory: &'a Inventory, id: &str, key: &str) -> Option<&'a str> {
    inventory
        .devices
        .iter()
        .find(|d| d.id == id)
        .and_then(|d| d.properties.get(key))
        .map(String::as_str)
}

#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_provider_reports_pcie_link_and_static_limits() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut p = GpuProvider::new(
        VendorSwitch::new(true, VendorMask::ALL),
        GpuProcessTable::new(),
    );
    let (inventory, values) = discover_and_poll(&mut p);
    assert_gpu_devices(&inventory);

    // Live link from NVML: Gen 1 at idle (ASPM), up to Gen 4 under load; x16 on this board.
    let (i, generation) = gpu_sensor(&inventory, NVIDIA, "link/pcie-gen");
    assert_eq!(generation.source, Source::Nvml);
    let gen = values[i].expect("NVIDIA PCIe generation");
    assert!((1.0..=4.0).contains(&gen), "Gen {gen}");
    let (i, width) = gpu_sensor(&inventory, NVIDIA, "link/pcie-width");
    assert_eq!(width.source, Source::Nvml);
    assert_eq!(values[i], Some(16.0));
    for name in ["load/encoder", "load/decoder"] {
        let (i, sensor) = gpu_sensor(&inventory, NVIDIA, name);
        assert_eq!(sensor.source, Source::Nvml, "{name}");
        let pct = values[i].expect(name);
        assert!((0.0..=100.0).contains(&pct), "{name} {pct}");
    }
    // No live link source for the AMD iGPU (PnP "current" is not live, ADL 40/41 excluded).
    assert!(!inventory
        .sensors
        .iter()
        .any(|s| s.device_id == AMD && s.kind == SensorKind::Link));

    // pcieMaxGen/pcieMaxWidth are the device's own link capability, sourced from PnP only
    // (NVML's max-link getters mix in the current slot and are not used for this property).
    for (key, value) in [
        ("pcieMaxGen", "4"),
        ("pcieMaxWidth", "16"),
        ("powerLimitMinW", "150"),
        ("powerLimitMaxW", "370"),
        ("powerLimitDefaultW", "320"),
        ("tempSlowdownC", "94"),
        ("tempShutdownC", "99"),
        ("tempMaxC", "90"),
    ] {
        assert_eq!(gpu_property(&inventory, NVIDIA, key), Some(value), "{key}");
    }
    // The iGPU gets the vendor-neutral PnP maximum link only.
    assert_eq!(gpu_property(&inventory, AMD, "pcieMaxGen"), Some("4"));
    assert_eq!(gpu_property(&inventory, AMD, "pcieMaxWidth"), Some("16"));
    assert_eq!(gpu_property(&inventory, AMD, "powerLimitMaxW"), None);
}

#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_provider_in_safe_mode_keeps_the_pnp_max_link() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut p = GpuProvider::new(
        VendorSwitch::new(false, VendorMask::ALL),
        GpuProcessTable::new(),
    );
    let inventory = p.discover().expect("discover");
    for id in [NVIDIA, AMD] {
        assert_eq!(
            gpu_property(&inventory, id, "pcieMaxGen"),
            Some("4"),
            "{id}"
        );
        assert_eq!(
            gpu_property(&inventory, id, "pcieMaxWidth"),
            Some("16"),
            "{id}"
        );
        assert_eq!(gpu_property(&inventory, id, "tempMaxC"), None, "{id}");
    }
    assert!(!inventory.sensors.iter().any(|s| s.id.contains("/link/")));
}

#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_provider_publishes_per_process_usage() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let processes = GpuProcessTable::new();
    let mut p = GpuProvider::new(VendorSwitch::new(true, VendorMask::ALL), processes.clone());
    discover_and_poll(&mut p);

    let rows = processes.processes(NVIDIA);
    assert!(!rows.is_empty() && rows.len() <= 20, "{} rows", rows.len());
    for pair in rows.windows(2) {
        let load = |i: usize| pair[i].load_percent.unwrap_or(-1.0);
        assert!(load(0) >= load(1), "sorted by load: {pair:?}");
    }
    let dwm = rows
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case("dwm.exe"))
        .expect("dwm.exe uses the primary GPU");
    assert!(dwm.dedicated_bytes.is_some_and(|b| b > 0), "{dwm:?}");
    assert!(
        dwm.load_percent.is_some(),
        "loads exist from the second poll"
    );
    assert!(processes.processes("gpu/pci-0000:99:00.0").is_empty());
}

#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_provider_loads_vendor_libraries_when_reenabled() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let switch = VendorSwitch::new(false, VendorMask::ALL);
    let mut p = GpuProvider::new(switch.clone(), GpuProcessTable::new());
    p.discover().expect("discover in safe mode");
    p.poll().expect("poll in safe mode");

    switch.enable();
    assert_eq!(p.poll(), Err(ProviderError::Rediscover));
    let inventory = p.discover().expect("discover with vendor libraries");
    let (_, temperature) = gpu_sensor(&inventory, NVIDIA, "temperature/core");
    assert_eq!(temperature.source, Source::Nvml);
}

#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_provider_honours_per_library_switches() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let switch = VendorSwitch::new(true, VendorMask::ALL.with(Vendor::Nvml, false));
    let mut p = GpuProvider::new(switch.clone(), GpuProcessTable::new());
    let inventory = p.discover().expect("discover without NVML");
    let (_, temperature) = gpu_sensor(&inventory, NVIDIA, "temperature/core");
    assert_ne!(temperature.source, Source::Nvml);
    p.poll().expect("poll without NVML");

    switch.set_libraries(VendorMask::ALL);
    assert_eq!(p.poll(), Err(ProviderError::Rediscover));
    let inventory = p.discover().expect("discover with NVML");
    let (_, temperature) = gpu_sensor(&inventory, NVIDIA, "temperature/core");
    assert_eq!(temperature.source, Source::Nvml);

    // Switching NVML off again leaves the loaded library alone and falls back.
    switch.set_libraries(VendorMask::ALL.with(Vendor::Nvml, false));
    assert_eq!(p.poll(), Err(ProviderError::Rediscover));
    let inventory = p.discover().expect("discover with NVML off again");
    let (_, temperature) = gpu_sensor(&inventory, NVIDIA, "temperature/core");
    assert_ne!(temperature.source, Source::Nvml);
    p.poll().expect("poll with NVML off again");
}

/// Where the recorded schema lives: next to the rules tests that read it.
const SCHEMA_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../oma-core/tests/fixtures/this-machine-schema.json"
);

/// Runs the default providers with the sensor service linked, as the app
/// does, and (with `OMA_WRITE_FIXTURES=1`, single-threaded) records the merged
/// schema for `oma-core`'s `default_rules_fixture` test. The link starts the
/// installed service if it is stopped; without it there would be no `tctl`.
/// Disk ids are already hashes and the recorded properties hold models only,
/// never serial numbers.
#[test]
#[ignore = "requires real Windows hardware"]
fn records_this_machine_schema() {
    use std::sync::Arc;
    use std::time::Instant;

    use oma_core::engine::Engine;
    use oma_core::sampler::unix_ms;
    use oma_ipc::ServiceState;
    use oma_win::svc::{
        pipe_connector, LinkSettings, ServiceLink, ServiceStatusTable, WindowsScm, SERVICE_NAME,
    };
    use oma_win::ServiceHandles;

    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let handles = ServiceHandles::default();
    let status = ServiceStatusTable::default();
    let link = ServiceLink::spawn(
        Arc::new(WindowsScm::new(SERVICE_NAME)),
        pipe_connector(),
        LinkSettings {
            drives: handles.drives.clone(),
            ..LinkSettings::new(oma_ipc::PIPE_NAME, 1_000)
        },
        false,
        status.clone(),
        handles.feed.clone(),
    );

    // Opening LibreHardwareMonitor alone takes a few seconds after the start.
    let deadline = Instant::now() + Duration::from_secs(60);
    while status.get().1.state != ServiceState::Connected {
        assert!(
            Instant::now() < deadline,
            "the sensor service did not connect: {:?}",
            status.get().1
        );
        std::thread::sleep(Duration::from_millis(250));
    }

    let providers = oma_win::default_providers(
        VendorSwitch::new(true, VendorMask::ALL),
        GpuProcessTable::new(),
        handles,
    );
    let mut engine = Engine::new(providers, 16);
    let start = Instant::now();
    for _ in 0..8 {
        engine.tick(unix_ms(), start.elapsed().as_millis() as u64);
        std::thread::sleep(Duration::from_secs(1));
    }
    link.shutdown();

    let schema = engine.schema();
    let lhm = schema
        .sensors
        .iter()
        .filter(|s| s.source == Source::Lhm)
        .count();
    assert!(lhm > 0, "the merged schema has sensors from the service");

    if std::env::var_os("OMA_WRITE_FIXTURES").is_some_and(|v| v == "1") {
        let mut json = serde_json::to_string_pretty(schema).expect("schema serialises");
        json.push('\n');
        std::fs::create_dir_all(std::path::Path::new(SCHEMA_FIXTURE).parent().unwrap())
            .expect("fixtures folder");
        std::fs::write(SCHEMA_FIXTURE, json).expect("write the fixture");
    }
}
