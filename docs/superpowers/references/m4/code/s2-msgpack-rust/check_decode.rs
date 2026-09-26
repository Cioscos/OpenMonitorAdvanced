//! Cross-decode + error-handling checks:
//! 1. Decode the .NET-produced fixtures with rmp-serde and re-encode them,
//!    to prove round-trip equality (not just Rust -> Rust).
//! 2. Probe rmp-serde's behaviour on: unknown envelope tag, missing required
//!    field, and an extra/unknown field in a body map.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Hello {
    protocol_version: u32,
    service_version: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Subscribe {
    interval_ms: u32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum IdentityHint {
    Cpu { index: u32 },
    Storage {
        physical_drive: u32,
        serial: Option<String>,
    },
    Memory {},
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Device {
    id: String,
    kind: String,
    name: String,
    vendor: Option<String>,
    properties: BTreeMap<String, String>,
    hint: Option<IdentityHint>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Schema {
    devices: Vec<Device>,
    sensors: Vec<Sensor>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Snapshot {
    seq: u64,
    timestamp_ms: u64,
    values: Vec<Option<f64>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Error {
    code: String,
    message: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
enum Envelope {
    Hello(Hello),
    Subscribe(Subscribe),
    Schema(Schema),
    Snapshot(Snapshot),
    Error(Error),
}

fn roundtrip(dir: &Path, name: &str) {
    let dotnet_path = dir.join(format!("{name}_dotnet.msgpack"));
    let bytes = fs::read(&dotnet_path).expect("read dotnet fixture");
    let decoded: Envelope = rmp_serde::from_slice(&bytes).unwrap_or_else(|e| {
        panic!("failed to decode {name}_dotnet.msgpack with rmp-serde: {e}")
    });
    let reencoded = rmp_serde::to_vec_named(&decoded).expect("reencode");
    let same = reencoded == bytes;
    println!(
        "{name}: decode(.NET bytes) -> re-encode identical to .NET bytes = {same}"
    );
    assert!(same, "{name}: round trip mismatch");
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../protocol/fixtures");

    println!("=== Cross-decode: rmp-serde reading MessagePack-CSharp output ===");
    for name in [
        "hello",
        "subscribe",
        "schema",
        "snapshot",
        "snapshot_empty_values",
        "error",
    ] {
        roundtrip(&dir, name);
    }

    println!();
    println!("=== Error handling probes ===");

    // 1. Unknown envelope tag -> clean error (not a panic).
    let mut buf = Vec::new();
    {
        let mut ser = rmp_serde::Serializer::new(&mut buf).with_struct_map();
        use serde::ser::{SerializeMap, Serializer as _};
        let mut map = ser.serialize_map(Some(2)).unwrap();
        map.serialize_entry("type", "ping").unwrap(); // not a known variant
        map.serialize_entry("body", &BTreeMap::<String, String>::new())
            .unwrap();
        map.end().unwrap();
    }
    match rmp_serde::from_slice::<Envelope>(&buf) {
        Ok(v) => println!("UNEXPECTED: unknown tag decoded as {v:?}"),
        Err(e) => println!("unknown tag 'ping' -> Err: {e}"),
    }

    // 2. Missing required field (Hello without service_version).
    let mut buf2 = Vec::new();
    {
        let mut ser = rmp_serde::Serializer::new(&mut buf2).with_struct_map();
        use serde::ser::{SerializeMap, Serializer as _};
        let mut map = ser.serialize_map(Some(2)).unwrap();
        map.serialize_entry("type", "hello").unwrap();
        // body has only protocol_version, missing service_version
        let mut inner = BTreeMap::new();
        inner.insert("protocol_version".to_string(), 1u32);
        map.serialize_entry("body", &inner).unwrap();
        map.end().unwrap();
    }
    match rmp_serde::from_slice::<Envelope>(&buf2) {
        Ok(v) => println!("UNEXPECTED: missing field decoded as {v:?}"),
        Err(e) => println!("missing required field 'service_version' -> Err: {e}"),
    }

    // 3. Extra/unknown field in a body map -> should be tolerated (ignored)
    //    by default with serde's derive (no #[serde(deny_unknown_fields)]).
    let mut buf3 = Vec::new();
    {
        let mut ser = rmp_serde::Serializer::new(&mut buf3).with_struct_map();
        use serde::ser::{SerializeMap, Serializer as _};
        let mut map = ser.serialize_map(Some(2)).unwrap();
        map.serialize_entry("type", "subscribe").unwrap();
        let mut inner = rmpv::Value::Map(vec![
            (
                rmpv::Value::String("interval_ms".into()),
                rmpv::Value::from(1000u32),
            ),
            (
                rmpv::Value::String("future_field_not_yet_known".into()),
                rmpv::Value::from("some-value"),
            ),
        ]);
        // Keep declared order stable: interval_ms first (already is).
        if let rmpv::Value::Map(ref mut entries) = inner {
            entries.sort_by(|_, _| std::cmp::Ordering::Equal); // no-op, keep insertion order
        }
        map.serialize_entry("body", &inner).unwrap();
        map.end().unwrap();
    }
    match rmp_serde::from_slice::<Envelope>(&buf3) {
        Ok(v) => println!("extra unknown field in body -> tolerated, decoded as {v:?}"),
        Err(e) => println!("UNEXPECTED: extra field rejected -> Err: {e}"),
    }

    println!();
    println!("ALL DECODE CHECKS PASSED");
}
