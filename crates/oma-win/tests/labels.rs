//! Every label key the providers emit (crates/oma-win/src/{cpu,memory,storage,network}.rs
//! and `GpuField::label_key` in crates/oma-win/src/gpu/field.rs)
//! must exist as `sensor.<key>` in both i18n catalogs, so a provider cannot
//! ship a key with no translation. Runs on any platform, no hardware needed.

const KEYS: &[&str] = &[
    "cpu.load.total",
    "cpu.load.thread",
    "cpu.clock.effective",
    "memory.load",
    "memory.used",
    "memory.total",
    "storage.read",
    "storage.write",
    "storage.active",
    "storage.volumeUsed",
    "storage.volumeFree",
    "storage.temperature",
    "storage.temperatureSensor",
    "network.down",
    "network.up",
    "network.linkSpeed",
    "gpu.load.core",
    "gpu.load.3d",
    "gpu.load.compute",
    "gpu.load.copy",
    "gpu.load.videoDecode",
    "gpu.load.videoEncode",
    "gpu.memory.dedicatedUsed",
    "gpu.memory.dedicatedTotal",
    "gpu.memory.sharedUsed",
    "gpu.temperature.core",
    "gpu.temperature.hotspot",
    "gpu.temperature.memory",
    "gpu.clock.core",
    "gpu.clock.memory",
    "gpu.power.board",
    "gpu.power.limit",
    "gpu.power.limitPercent",
    "gpu.fan.percent",
    "gpu.fan.rpm",
    "gpu.voltage.core",
    "gpu.throttle.power",
    "gpu.throttle.thermal",
];

#[test]
fn every_provider_label_key_has_a_translation() {
    let en: serde_json::Value =
        serde_json::from_str(include_str!("../../../app/src/lib/i18n/en.json"))
            .expect("en catalog");
    let it: serde_json::Value =
        serde_json::from_str(include_str!("../../../app/src/lib/i18n/it.json"))
            .expect("it catalog");
    for key in KEYS {
        let sensor_key = format!("sensor.{key}");
        assert!(
            en.get(&sensor_key).and_then(|v| v.as_str()).is_some(),
            "missing en.json key {sensor_key}"
        );
        assert!(
            it.get(&sensor_key).and_then(|v| v.as_str()).is_some(),
            "missing it.json key {sensor_key}"
        );
    }
}
