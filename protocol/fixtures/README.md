# Protocol fixtures

Reference MessagePack payloads for the `oma-ipc` sensor protocol (spec
§6). Each `.msgpack` file holds exactly one encoded `Message` **payload**
(no `u32` length prefix — that prefix only exists on the wire, inside a
frame, not in these files). Both the Rust crate (`crates/oma-ipc`) and the
.NET service test suite must produce byte-for-byte identical output for
the logical messages described below, and must decode these files back to
the same logical values.

Envelope shape: adjacently tagged `{"type": <tag>, "body": <payload>}`
(`#[serde(tag = "type", content = "body", rename_all = "snake_case")]`).
Struct fields are encoded as maps with string keys **in declared field
order**, never alphabetical. `BTreeMap<String, String>` fields (such as
`WireDevice.properties`) are the one exception: their keys are sorted by
ordinal UTF-8 byte order, which `BTreeMap`'s own iteration order already
gives for free in Rust.

## Logical content

- **`hello.msgpack`**: `Hello { protocol_version: 1, service_version: "0.1.0" }`.
- **`subscribe.msgpack`**: `Subscribe { interval_ms: 1000 }`.
- **`schema.msgpack`**: `WireSchema` with 5 devices, in this order:
  1. `lhm-cpu`, kind `cpu`, name `AMD Ryzen 9 7950X3D`, vendor `AMD`,
     properties `{}`, hint `Cpu { index: 0 }`.
  2. `lhm-nvme0`, kind `storage`, name `Samsung SSD 990 PRO 2TB`, vendor
     nil, properties `{"firmware": "4B2QJXD7"}`, hint
     `Storage { physical_drive: 0, model: Some("Samsung SSD 990 PRO 2TB"), serial: Some("0025_38B1_4150_2A6C.") }`.
  3. `lhm-hdd1`, kind `storage`, name `ST2000DM008`, vendor nil,
     properties `{}`, hint
     `Storage { physical_drive: 1, model: Some("ST2000DM008-2UB102"), serial: None }`.
  4. `lhm-ram`, kind `memory`, name `Memory`, vendor nil, properties
     `{"dimm0.size": "32 GB", "dimm0.speedMts": "6000"}`, hint `Memory {}`.
  5. `lhm-mb`, kind `motherboard`, name `Nuvoton NCT6799D`, vendor nil,
     properties `{}`, hint `None`.

  And 3 sensors:
  1. `{device_id: "lhm-cpu", kind: "temperature", name: "package", unit: "celsius", label_key: "cpu.temperature.package", label_arg: None, category: "temperature"}`.
  2. `{device_id: "lhm-mb", kind: "fan", name: "lhm-fan-1", unit: "rpm", label_key: "lhm.raw", label_arg: Some("Ventola n.1 — °C"), category: "fan"}`.
  3. `{device_id: "lhm-nvme0", kind: "percent", name: "wear", unit: "percent", label_key: "storage.percentUsed", label_arg: None, category: "percent"}`.
- **`snapshot.msgpack`**: `WireSnapshot { seq: 4294967301, timestamp_ms: 1790000000000, values: [Some(45.0), None, Some(-12.5), Some(0.0)] }`.
- **`snapshot_empty.msgpack`**: `WireSnapshot { seq: 1, timestamp_ms: 0, values: [] }`.
- **`error.msgpack`**: `WireError { code: "bad_request", message: "Messaggio non valido: è atteso Subscribe" }`.

## Encoding rules (spec §6, reaffirmed by the S2 spike)

- Structs are maps with string keys, in declared order.
- `f64` is always encoded as float64 (`0xcb`), never float32, even for
  whole numbers.
- Absent optional values are `nil` — the key is still present, never
  omitted. No wire-protocol type uses `#[serde(skip_serializing_if)]`.
- Empty arrays/maps are zero-length (`0x90`/`0x80`), not omitted or nil.
- Values must never be NaN or infinite; the service converts non-finite
  readings to `nil` before sending, and `decode_payload` on the receiving
  side treats any `Some(x)` with `!x.is_finite()` as `None` defensively.

## Regenerating the fixtures

The Rust crate (`crates/oma-ipc`) is the source of truth. To regenerate
all files, from the repository root, in PowerShell:

```powershell
$env:OMA_WRITE_FIXTURES = '1'
try { cargo test -p oma-ipc --test fixtures } finally { Remove-Item Env:OMA_WRITE_FIXTURES }
cargo test -p oma-ipc --test fixtures
```

The second run (without the environment variable) must pass, asserting
the freshly written files still match the encoder byte-for-byte and
decode back to the reference messages.

## File sizes (current fixtures)

| File | Bytes |
|---|---|
| `hello.msgpack` | 58 |
| `subscribe.msgpack` | 37 |
| `schema.msgpack` | 1016 |
| `snapshot.msgpack` | 92 |
| `snapshot_empty.msgpack` | 48 |
| `error.msgpack` | 86 |

`hello.msgpack` starts with `82 a4 74 79 70 65` (a 2-entry fixmap, then the
fixstr `"type"`), as expected for the `{"type": "hello", "body": {...}}`
envelope.
