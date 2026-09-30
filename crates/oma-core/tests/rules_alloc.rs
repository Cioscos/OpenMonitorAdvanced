//! A steady rules evaluation must not allocate. This binary holds a single
//! test with a counting allocator, so nothing else runs beside it; the
//! counter is enabled only around `RuleEngine::evaluate`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use oma_core::engine::Quality;
use oma_core::model::{Device, DeviceKind, Label, Schema, Sensor, SensorKind, Source, Unit};
use oma_core::rules::{
    default_rules, Condition, Hysteresis, LevelSpec, Notify, Rule, RuleEngine, Target, Threshold,
};

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

struct Counting;

fn count() {
    // `try_with`: the flag is gone while the thread is being torn down.
    if COUNTING.try_with(Cell::get).unwrap_or(false) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: every method forwards to the system allocator with the same
// arguments; the counting only reads a thread-local flag and bumps an atomic,
// neither of which allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: same contract as ours, forwarded unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: same contract as ours, forwarded unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        // SAFETY: same contract as ours, forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as ours, forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const SENSORS: usize = 2000;
const CUSTOM_RULES: usize = 200;
const WARM_UP_TICKS: u64 = 120;
const MEASURED_TICKS: u64 = 100;
const WALL_START_MS: u64 = 1_700_000_000_000;
/// Sensors before the filler loads: one per name of the default rules.
const NAMED_SENSORS: usize = 12;

fn device(id: &str, kind: DeviceKind, properties: &[(&str, &str)]) -> Device {
    Device {
        id: id.into(),
        kind,
        name: id.into(),
        vendor: None,
        properties: properties
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }
}

/// The sensors the default rules match, then load sensors up to `SENSORS`.
fn schema() -> Schema {
    let devices = vec![
        device("cpu/0", DeviceKind::Cpu, &[("tjMaxC", "100")]),
        device("gpu/0", DeviceKind::Gpu, &[]),
        device(
            "storage/0",
            DeviceKind::Storage,
            &[("tempWarningC", "70"), ("tempCriticalC", "80")],
        ),
        device("memory/0", DeviceKind::Memory, &[]),
        device("battery/0", DeviceKind::Battery, &[]),
    ];
    let mut sensors = Vec::new();
    let mut add = |device: &str, kind: SensorKind, name: &str, unit: Unit| {
        sensors.push(Sensor::new(
            device,
            kind,
            name,
            unit,
            Label::new("test"),
            Source::Mock,
        ));
    };
    add("cpu/0", SensorKind::Temperature, "tctl", Unit::Celsius);
    add("cpu/0", SensorKind::Flag, "throttle-thermal", Unit::Boolean);
    add("gpu/0", SensorKind::Temperature, "core", Unit::Celsius);
    add("gpu/0", SensorKind::Temperature, "hotspot", Unit::Celsius);
    add("gpu/0", SensorKind::Temperature, "memory", Unit::Celsius);
    add("gpu/0", SensorKind::Flag, "throttle-thermal", Unit::Boolean);
    add("storage/0", SensorKind::Temperature, "drive", Unit::Celsius);
    add("storage/0", SensorKind::Percent, "wear", Unit::Percent);
    add(
        "storage/0",
        SensorKind::Flag,
        "critical-warning",
        Unit::Boolean,
    );
    add("storage/0", SensorKind::Percent, "volume-c", Unit::Percent);
    add("memory/0", SensorKind::Load, "used", Unit::Percent);
    add("battery/0", SensorKind::Percent, "charge", Unit::Percent);
    for n in 0..SENSORS - NAMED_SENSORS {
        add(
            "cpu/0",
            SensorKind::Load,
            &format!("core-{n}"),
            Unit::Percent,
        );
    }
    Schema {
        revision: 1,
        devices,
        sensors,
    }
}

/// Custom rules, each on one load sensor that exists in the schema.
fn custom_rules() -> Vec<Rule> {
    let level = |value: f64| {
        Some(LevelSpec {
            threshold: Some(Threshold::Fixed { fixed: value }),
            duration_s: 5,
        })
    };
    (0..CUSTOM_RULES)
        .map(|n| Rule {
            id: format!("custom-00000000-0000-4000-8000-{n:012}"),
            target: Target::Sensor {
                sensor: format!("cpu/0/load/core-{n}"),
            },
            unit: Unit::Percent,
            condition: Condition::Above,
            warn: level(80.0),
            crit: level(90.0),
            hysteresis: Hysteresis::default(),
            enabled: true,
            notify: Notify::default(),
        })
        .collect()
}

/// Values of every sensor: calm, or with every rule in alarm.
fn values(schema: &Schema, alarm: bool) -> Vec<Option<f64>> {
    schema
        .sensors
        .iter()
        .enumerate()
        .map(|(i, sensor)| {
            let name = sensor.id.rsplit('/').next().unwrap_or_default();
            let (calm, hot) = match name {
                "tctl" => (45.0, 99.0),
                "core" => (45.0, 95.0),
                "hotspot" => (55.0, 97.0),
                "memory" => (50.0, 101.0),
                "drive" => (40.0, 75.0),
                "wear" => (5.0, 95.0),
                "volume-c" => (40.0, 99.0),
                "used" => (50.0, 98.0),
                "charge" => (80.0, 4.0),
                "throttle-thermal" | "critical-warning" => (0.0, 1.0),
                // The loads with a custom rule alarm, the others stay calm.
                _ if i < NAMED_SENSORS + CUSTOM_RULES => (10.0, 95.0),
                _ => (10.0, 10.0),
            };
            Some(if alarm { hot } else { calm })
        })
        .collect()
}

/// Warms an engine up, then counts the allocations of `MEASURED_TICKS`
/// steady evaluations and returns them with the average time of one.
fn measure(label: &str, alarm: bool) -> (usize, f64) {
    let schema = schema();
    let values = values(&schema, alarm);
    let quality = vec![Quality::Fresh; schema.sensors.len()];
    let mut rules = default_rules();
    rules.extend(custom_rules());
    let mut engine = RuleEngine::new();
    engine.set_rules(rules);

    let mut tick = 0u64;
    let mut step = |engine: &mut RuleEngine| {
        tick += 1;
        engine.evaluate(
            &schema,
            false,
            &values,
            &quality,
            tick * 1000,
            WALL_START_MS + tick * 1000,
        )
    };
    for _ in 0..WARM_UP_TICKS {
        step(&mut engine);
    }
    let alerts = engine.report().alerts.len();
    let revision = engine.report().revision;
    if alarm {
        assert!(alerts > CUSTOM_RULES, "{label}: only {alerts} alerts");
    } else {
        assert_eq!(alerts, 0, "{label}");
    }

    let mut changed = 0;
    let started = Instant::now();
    COUNTING.with(|c| c.set(true));
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    for _ in 0..MEASURED_TICKS {
        let evaluation = step(&mut engine);
        changed += usize::from(evaluation.report.is_some()) + evaluation.entries.len();
    }
    let allocations = ALLOCATIONS.load(Ordering::Relaxed) - before;
    COUNTING.with(|c| c.set(false));
    let average_us = started.elapsed().as_secs_f64() * 1e6 / MEASURED_TICKS as f64;

    assert_eq!(changed, 0, "{label}: the report or the levels changed");
    assert_eq!(engine.report().revision, revision, "{label}");
    (allocations, average_us)
}

/// The counter must see an allocation, or a zero would prove nothing.
fn counter_sees_allocations() -> bool {
    COUNTING.with(|c| c.set(true));
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let boxed = std::hint::black_box(Box::new(7u64));
    let after = ALLOCATIONS.load(Ordering::Relaxed);
    COUNTING.with(|c| c.set(false));
    drop(boxed);
    after > before
}

#[test]
fn steady_evaluation_does_not_allocate() {
    assert!(counter_sees_allocations());
    let (ok_allocations, ok_us) = measure("ok", false);
    let (alarm_allocations, alarm_us) = measure("alarms", true);
    println!(
        "{SENSORS} sensors, {} rules: steady ok {ok_us:.1} us/eval, steady alarms {alarm_us:.1} us/eval",
        default_rules().len() + CUSTOM_RULES
    );
    assert_eq!(ok_allocations, 0, "allocations in the ok state");
    assert_eq!(alarm_allocations, 0, "allocations with stable alarms");
}
