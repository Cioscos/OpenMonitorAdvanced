//! Generates reference MessagePack fixtures for the IPC protocol spike.
//! Envelope design: adjacently tagged `{ "type": <tag>, "body": <payload> }`.

use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Serialize)]
struct Hello {
    protocol_version: u32,
    service_version: String,
}

#[derive(Serialize)]
struct Subscribe {
    interval_ms: u32,
}

#[derive(Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum IdentityHint {
    Cpu { index: u32 },
    // NOTE: `serial` intentionally has NO `skip_serializing_if`. The wire
    // convention is "absent value -> nil", never "omit the field": every
    // struct is a map with a fixed, declared set of keys. Skipping the key
    // when `None` (as we first tried here) desynchronizes the byte layout
    // from a .NET side that always writes the key with a `nil` value -- see
    // the "pitfalls" section of the spike report.
    Storage {
        physical_drive: u32,
        serial: Option<String>,
    },
    Memory {},
}

#[derive(Serialize)]
struct Device {
    id: String,
    kind: String,
    name: String,
    vendor: Option<String>,
    properties: BTreeMap<String, String>,
    hint: Option<IdentityHint>,
}

#[derive(Serialize)]
struct Sensor {
    id: String,
    device_id: String,
    kind: String,
    unit: String,
    label_key: String,
    label_text: Option<String>,
    category: String,
    source: String,
}

#[derive(Serialize)]
struct Schema {
    devices: Vec<Device>,
    sensors: Vec<Sensor>,
}

#[derive(Serialize)]
struct Snapshot {
    seq: u64,
    timestamp_ms: u64,
    values: Vec<Option<f64>>,
}

#[derive(Serialize)]
struct Error {
    code: String,
    message: String,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
enum Envelope {
    Hello(Hello),
    Subscribe(Subscribe),
    Schema(Schema),
    Snapshot(Snapshot),
    Error(Error),
}

fn write_fixture(dir: &Path, name: &str, env: &Envelope) {
    let bytes = rmp_serde::to_vec_named(env).expect("encode");
    let path = dir.join(format!("{name}.msgpack"));
    fs::write(&path, &bytes).expect("write fixture");
    println!("{name}: {} bytes -> {}", bytes.len(), path.display());
    println!("  hex: {}", hex(&bytes));
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../protocol/fixtures");
    fs::create_dir_all(&out_dir).expect("mkdir fixtures");

    // 1. Hello
    write_fixture(
        &out_dir,
        "hello",
        &Envelope::Hello(Hello {
            protocol_version: 1,
            service_version: "0.4.0".to_owned(),
        }),
    );

    // 2. Subscribe
    write_fixture(
        &out_dir,
        "subscribe",
        &Envelope::Subscribe(Subscribe { interval_ms: 1000 }),
    );

    // 3. Schema, with edge cases:
    //    - device without vendor (nil), without hint (nil)
    //    - device with vendor, empty properties map
    //    - device with non-empty sorted properties map, unicode name
    //    - cpu hint, storage hint (with & without serial), memory hint ({})
    let mut props_empty: BTreeMap<String, String> = BTreeMap::new();
    let mut props_sorted: BTreeMap<String, String> = BTreeMap::new();
    // Insert out of order on purpose; BTreeMap keeps them sorted by key.
    props_sorted.insert("zeta".to_owned(), "9".to_owned());
    props_sorted.insert("alpha".to_owned(), "1".to_owned());
    props_sorted.insert("mid".to_owned(), "5".to_owned());
    props_empty.clear();

    let devices = vec![
        Device {
            id: "cpu/0".to_owned(),
            kind: "cpu".to_owned(),
            name: "Ryzen 9 9950X".to_owned(),
            vendor: None,
            properties: props_empty.clone(),
            hint: Some(IdentityHint::Cpu { index: 0 }),
        },
        Device {
            id: "motherboard/0".to_owned(),
            kind: "motherboard".to_owned(),
            name: "Temperatura °C sensor board".to_owned(), // unicode
            vendor: Some("ASUS".to_owned()),
            properties: props_sorted,
            hint: None,
        },
        Device {
            id: "storage/0".to_owned(),
            kind: "storage".to_owned(),
            name: "NVMe SSD".to_owned(),
            vendor: Some("Samsung".to_owned()),
            properties: props_empty.clone(),
            hint: Some(IdentityHint::Storage {
                physical_drive: 0,
                serial: Some("S6XPNX0T123456".to_owned()),
            }),
        },
        Device {
            id: "storage/1".to_owned(),
            kind: "storage".to_owned(),
            name: "Unknown drive".to_owned(),
            vendor: None,
            properties: props_empty.clone(),
            hint: Some(IdentityHint::Storage {
                physical_drive: 1,
                serial: None, // nil serial: edge case
            }),
        },
        Device {
            id: "memory/0".to_owned(),
            kind: "memory".to_owned(),
            name: "DDR5 64GB".to_owned(),
            vendor: None,
            properties: props_empty,
            hint: Some(IdentityHint::Memory {}),
        },
    ];

    let sensors = vec![
        Sensor {
            id: "cpu/0/temperature/package".to_owned(),
            device_id: "cpu/0".to_owned(),
            kind: "temperature".to_owned(),
            unit: "celsius".to_owned(),
            label_key: "cpu.temperature.package".to_owned(),
            label_text: None,
            category: "temperature".to_owned(),
            source: "lhm".to_owned(),
        },
        Sensor {
            id: "motherboard/0/temperature/vrm".to_owned(),
            device_id: "motherboard/0".to_owned(),
            kind: "temperature".to_owned(),
            unit: "celsius".to_owned(),
            label_key: "motherboard.temperature.vrm".to_owned(),
            label_text: Some("Temperatura °C VRM".to_owned()), // unicode label_text
            category: "temperature".to_owned(),
            source: "lhm".to_owned(),
        },
    ];

    write_fixture(&out_dir, "schema", &Envelope::Schema(Schema { devices, sensors }));

    // 4. Snapshot, edge cases:
    //    - seq/timestamp_ms above u32::MAX (must be u64)
    //    - negative float
    //    - a nil value among present ones
    //    - empty values list (separate fixture)
    write_fixture(
        &out_dir,
        "snapshot",
        &Envelope::Snapshot(Snapshot {
            seq: 9_876_543_210, // > u32::MAX (4_294_967_295)
            timestamp_ms: 17_000_000_000_123, // > u32::MAX
            values: vec![Some(45.0), Some(-12.5), None, Some(0.0), Some(100.0)],
        }),
    );

    write_fixture(
        &out_dir,
        "snapshot_empty_values",
        &Envelope::Snapshot(Snapshot {
            seq: 0,
            timestamp_ms: 0,
            values: vec![],
        }),
    );

    // 5. Error
    write_fixture(
        &out_dir,
        "error",
        &Envelope::Error(Error {
            code: "service_unreachable".to_owned(),
            message: "Impossibile connettersi al servizio: Temperatura °C non disponibile"
                .to_owned(),
        }),
    );

    // --- NaN / Infinity experiment (NOT part of the protocol fixtures; values
    // must never be NaN/Infinity on the wire, this only documents what the
    // libraries do if validation is skipped). Written to a side file.
    let nan_probe = rmp_serde::to_vec_named(&Snapshot {
        seq: 1,
        timestamp_ms: 1,
        values: vec![Some(f64::NAN), Some(f64::INFINITY), Some(f64::NEG_INFINITY)],
    })
    .unwrap();
    fs::write(out_dir.join("_nan_probe_rust.msgpack"), &nan_probe).unwrap();
    println!("nan_probe (rust): {}", hex(&nan_probe));
    println!("f64::NAN bits = {:016x}", f64::NAN.to_bits());
}
