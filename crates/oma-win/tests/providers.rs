#![cfg(windows)]

use std::time::Duration;

use oma_core::provider::{Inventory, Provider};
use oma_win::cpu::CpuProvider;
use oma_win::memory::MemoryProvider;
use oma_win::storage::StorageProvider;

/// Discovers, waits for a second PDH sample, polls, and checks alignment.
fn discover_and_poll(p: &mut dyn Provider) -> (Inventory, Vec<Option<f64>>) {
    let inventory = p.discover().expect("discover");
    std::thread::sleep(Duration::from_millis(1_100));
    let values = p.poll().expect("poll");
    assert_eq!(
        values.len(),
        inventory.sensors.len(),
        "values must align with sensors"
    );
    (inventory, values)
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
    let (inventory, values) = discover_and_poll(&mut p);
    assert!(!inventory.devices.is_empty(), "at least the system disk");
    for (sensor, value) in inventory.sensors.iter().zip(&values) {
        if sensor.label.key == "storage.volumeUsed" {
            let pct = value.expect("volume usage");
            assert!((0.0..=100.0).contains(&pct), "{} = {pct}", sensor.id);
        }
    }
}
