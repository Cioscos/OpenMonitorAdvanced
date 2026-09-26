//! Byte-for-byte fixture parity tests against `protocol/fixtures/*.msgpack`.
//!
//! With `OMA_WRITE_FIXTURES=1` set, `fixtures_match_the_encoder_byte_for_byte`
//! (over)writes the fixture files from the encoder instead of comparing
//! against them. See `protocol/fixtures/README.md` for the regeneration
//! procedure and the logical content of each fixture.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use oma_ipc::Hello;
use oma_ipc::{
    decode_payload, encode_payload, IdentityHint, Message, Subscribe, WireDevice, WireError,
    WireSchema, WireSensor, WireSnapshot,
};

const NAMES: &[&str] = &[
    "hello",
    "subscribe",
    "schema",
    "snapshot",
    "snapshot_empty",
    "error",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol/fixtures")
}

fn reference(name: &str) -> Message {
    match name {
        "hello" => Message::Hello(Hello {
            protocol_version: 1,
            service_version: "0.1.0".to_owned(),
        }),
        "subscribe" => Message::Subscribe(Subscribe { interval_ms: 1000 }),
        "schema" => {
            let mut nvme_props = BTreeMap::new();
            nvme_props.insert("firmware".to_owned(), "4B2QJXD7".to_owned());

            let mut ram_props = BTreeMap::new();
            ram_props.insert("dimm0.size".to_owned(), "32 GB".to_owned());
            ram_props.insert("dimm0.speedMts".to_owned(), "6000".to_owned());

            let devices = vec![
                WireDevice {
                    id: "lhm-cpu".to_owned(),
                    kind: "cpu".to_owned(),
                    name: "AMD Ryzen 9 7950X3D".to_owned(),
                    vendor: Some("AMD".to_owned()),
                    properties: BTreeMap::new(),
                    hint: Some(IdentityHint::Cpu { index: 0 }),
                },
                WireDevice {
                    id: "lhm-nvme0".to_owned(),
                    kind: "storage".to_owned(),
                    name: "Samsung SSD 990 PRO 2TB".to_owned(),
                    vendor: None,
                    properties: nvme_props,
                    hint: Some(IdentityHint::Storage {
                        physical_drive: 0,
                        model: Some("Samsung SSD 990 PRO 2TB".to_owned()),
                        serial: Some("0025_38B1_4150_2A6C.".to_owned()),
                    }),
                },
                WireDevice {
                    id: "lhm-hdd1".to_owned(),
                    kind: "storage".to_owned(),
                    name: "ST2000DM008".to_owned(),
                    vendor: None,
                    properties: BTreeMap::new(),
                    hint: Some(IdentityHint::Storage {
                        physical_drive: 1,
                        model: Some("ST2000DM008-2UB102".to_owned()),
                        serial: None,
                    }),
                },
                WireDevice {
                    id: "lhm-ram".to_owned(),
                    kind: "memory".to_owned(),
                    name: "Memory".to_owned(),
                    vendor: None,
                    properties: ram_props,
                    hint: Some(IdentityHint::Memory {}),
                },
                WireDevice {
                    id: "lhm-mb".to_owned(),
                    kind: "motherboard".to_owned(),
                    name: "Nuvoton NCT6799D".to_owned(),
                    vendor: None,
                    properties: BTreeMap::new(),
                    hint: None,
                },
            ];

            let sensors = vec![
                WireSensor {
                    device_id: "lhm-cpu".to_owned(),
                    kind: "temperature".to_owned(),
                    name: "package".to_owned(),
                    unit: "celsius".to_owned(),
                    label_key: "cpu.temperature.package".to_owned(),
                    label_arg: None,
                    category: "temperature".to_owned(),
                },
                WireSensor {
                    device_id: "lhm-mb".to_owned(),
                    kind: "fan".to_owned(),
                    name: "lhm-fan-1".to_owned(),
                    unit: "rpm".to_owned(),
                    label_key: "lhm.raw".to_owned(),
                    label_arg: Some("Ventola n.1 \u{2014} \u{b0}C".to_owned()),
                    category: "fan".to_owned(),
                },
                WireSensor {
                    device_id: "lhm-nvme0".to_owned(),
                    kind: "percent".to_owned(),
                    name: "wear".to_owned(),
                    unit: "percent".to_owned(),
                    label_key: "storage.percentUsed".to_owned(),
                    label_arg: None,
                    category: "percent".to_owned(),
                },
            ];

            Message::Schema(WireSchema { devices, sensors })
        }
        "snapshot" => Message::Snapshot(WireSnapshot {
            seq: 4_294_967_301,
            timestamp_ms: 1_790_000_000_000,
            values: vec![Some(45.0), None, Some(-12.5), Some(0.0)],
        }),
        "snapshot_empty" => Message::Snapshot(WireSnapshot {
            seq: 1,
            timestamp_ms: 0,
            values: vec![],
        }),
        "error" => Message::Error(WireError {
            code: "bad_request".to_owned(),
            message: "Messaggio non valido: \u{e8} atteso Subscribe".to_owned(),
        }),
        other => panic!("unknown fixture name: {other}"),
    }
}

#[test]
fn fixtures_match_the_encoder_byte_for_byte() {
    let write_mode = std::env::var("OMA_WRITE_FIXTURES").as_deref() == Ok("1");
    let dir = fixtures_dir();

    for name in NAMES {
        let msg = reference(name);
        let encoded = encode_payload(&msg).expect("encode reference message");
        let path = dir.join(format!("{name}.msgpack"));

        if write_mode {
            fs::write(&path, &encoded).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
        } else {
            let on_disk = fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
            assert_eq!(
                encoded, on_disk,
                "{name}: encoder output does not match protocol/fixtures/{name}.msgpack byte-for-byte"
            );
        }
    }
}

#[test]
fn fixtures_decode_to_the_reference_messages() {
    let dir = fixtures_dir();

    for name in NAMES {
        let path = dir.join(format!("{name}.msgpack"));
        let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let decoded = decode_payload(&bytes).unwrap_or_else(|e| panic!("decode {name}: {e}"));
        assert_eq!(decoded, reference(name), "{name}: decoded message mismatch");
    }
}
