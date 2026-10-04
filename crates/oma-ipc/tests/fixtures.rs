//! Byte-for-byte fixture parity tests against `protocol/fixtures/*.msgpack`.
//!
//! With `OMA_WRITE_FIXTURES=1` set, `fixtures_match_the_encoder_byte_for_byte`
//! (over)writes the fixture files from the encoder instead of comparing
//! against them. See `protocol/fixtures/README.md` for the regeneration
//! procedure and the logical content of each fixture.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use oma_ipc::{
    decode_payload, drive_key, encode_payload, Hello, IdentityHint, Message, Subscribe, WireDevice,
    WireDrive, WireError, WireSchema, WireSensor, WireServiceState, WireSnapshot, MAX_DRIVE_KEYS,
    MODULES, PROTOCOL_VERSION,
};

/// `drive_key("Samsung SSD 990 PRO 2TB", "0025_38B1_4150_2A6C.")`, from `drive_key.json`.
const KEY_A: &str = "589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d83";
/// `drive_key("ST2000DM008-2UB102", "WFL4ABCD")`, from `drive_key.json`.
const KEY_B: &str = "3ed905bde72420026a8d0268d0faa314158c7f6d24c853abd4cc97cf55904ea6";

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
            protocol_version: PROTOCOL_VERSION,
            service_version: "0.1.0".to_owned(),
            pawn_io: "rebootPending".to_owned(),
        }),
        "subscribe" => Message::Subscribe(Subscribe {
            interval_ms: 1000,
            disabled_modules: vec!["memory".to_owned(), "psu".to_owned()],
            smart_disabled_drives: vec![KEY_A.to_owned()],
            smart_enabled_drives: vec![KEY_B.to_owned()],
        }),
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

            let service = WireServiceState {
                active_modules: ["cpu", "motherboard", "storage", "controller"]
                    .map(str::to_owned)
                    .to_vec(),
                smart_disabled_drives: vec![KEY_A.to_owned()],
                reconfiguration: "pending".to_owned(),
                drives: vec![
                    WireDrive {
                        physical_drive: 0,
                        key: Some(KEY_A.to_owned()),
                        model: Some("Samsung SSD 990 PRO 2TB".to_owned()),
                        state: "smartOff".to_owned(),
                        blocks_smart: false,
                    },
                    WireDrive {
                        physical_drive: 1,
                        key: None,
                        model: Some("ST2000DM008-2UB102".to_owned()),
                        state: "standby".to_owned(),
                        blocks_smart: true,
                    },
                ],
            };

            Message::Schema(WireSchema {
                devices,
                sensors,
                service,
            })
        }
        "snapshot" => Message::Snapshot(WireSnapshot {
            seq: 4_294_967_301,
            timestamp_ms: 1_790_000_000_000,
            values: vec![Some(45.0), None, Some(-12.5), Some(0.0)],
            held: vec![false, false, true, false],
        }),
        "snapshot_empty" => Message::Snapshot(WireSnapshot {
            seq: 1,
            timestamp_ms: 0,
            values: vec![],
            held: vec![],
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

#[test]
fn drive_key_matches_the_shared_vector() {
    let path = fixtures_dir().join("drive_key.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let cases: Vec<serde_json::Value> = serde_json::from_str(&text).expect("parse drive_key.json");
    assert!(
        cases.len() >= 10,
        "the vector should stay a meaningful size"
    );

    let mut with_key = 0;
    for case in &cases {
        let model = case["model"].as_str().expect("model");
        let serial = case["serial"].as_str().expect("serial");
        let expected = case["key"].as_str().map(str::to_owned);
        assert_eq!(
            drive_key(model, serial),
            expected,
            "drive_key({model:?}, {serial:?})"
        );
        if expected.is_some() {
            with_key += 1;
        }
    }
    assert!(
        with_key > 0 && with_key < cases.len(),
        "keys and no-key cases"
    );
}

#[test]
fn subscribe_v3_keeps_every_key() {
    let msg = Message::Subscribe(Subscribe {
        interval_ms: 1000,
        disabled_modules: vec![],
        smart_disabled_drives: vec![],
        smart_enabled_drives: vec![],
    });
    let bytes = encode_payload(&msg).expect("encode");
    let value: serde_json::Value = rmp_serde::from_slice(&bytes).expect("decode as a value");
    let body = value["body"].as_object().expect("body is a map");
    let keys: Vec<&str> = body.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "disabled_modules",
            "interval_ms",
            "smart_disabled_drives",
            "smart_enabled_drives"
        ],
        "keys are sorted by serde_json only; the point is that none is missing"
    );
    assert_eq!(body["disabled_modules"], serde_json::json!([]));
    assert_eq!(body["smart_disabled_drives"], serde_json::json!([]));
    assert_eq!(body["smart_enabled_drives"], serde_json::json!([]));
    assert_eq!(decode_payload(&bytes).expect("round trip"), msg);
}

#[test]
fn protocol_constants_are_v3() {
    assert_eq!(PROTOCOL_VERSION, 3);
    assert_eq!(
        MODULES,
        [
            "cpu",
            "motherboard",
            "memory",
            "storage",
            "controller",
            "psu"
        ]
    );
    assert_eq!(MAX_DRIVE_KEYS, 64);
}

/// `protocol/fixtures/hello.msgpack` as protocol v1 wrote it (commit 46a024f): no `pawn_io`.
const V1_HELLO: [u8; 58] = [
    0x82, 0xa4, 0x74, 0x79, 0x70, 0x65, 0xa5, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0xa4, 0x62, 0x6f, 0x64,
    0x79, 0x82, 0xb0, 0x70, 0x72, 0x6f, 0x74, 0x6f, 0x63, 0x6f, 0x6c, 0x5f, 0x76, 0x65, 0x72, 0x73,
    0x69, 0x6f, 0x6e, 0x1, 0xaf, 0x73, 0x65, 0x72, 0x76, 0x69, 0x63, 0x65, 0x5f, 0x76, 0x65, 0x72,
    0x73, 0x69, 0x6f, 0x6e, 0xa5, 0x30, 0x2e, 0x31, 0x2e, 0x30,
];

#[test]
fn a_v1_hello_still_decodes_so_the_version_can_be_checked() {
    let decoded = decode_payload(&V1_HELLO).expect("a v1 hello must decode");
    assert_eq!(
        decoded,
        Message::Hello(Hello {
            protocol_version: 1,
            service_version: "0.1.0".to_owned(),
            pawn_io: "unknown".to_owned(),
        })
    );
}
