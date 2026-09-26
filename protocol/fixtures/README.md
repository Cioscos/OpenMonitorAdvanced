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

`WireDevice.hint` uses the same adjacently-tagged shape, but with field
names `kind`/`value` instead of `type`/`body` (to avoid colliding with
`WireDevice.kind`): `{"kind": <snake_case tag>, "value": <payload>}`. The
three tags are `cpu` (`{"index": <u32>}`), `storage`
(`{"physical_drive": <u32>, "model": <string or nil>, "serial": <string or
nil>}`) and `memory`, whose `value` is an **empty map** (`0x80`), not
`nil` — `Memory {}` has no fields, but the `value` key is still a
(zero-length) map, exactly as any other struct-as-map would be encoded
with no fields.

## Absent optional fields are tolerated on decode, but never on encode (ruling R10)

Every encoder (both languages) must always write every declared key, using
`nil` for an absent `Option`/nullable value — this is what makes the
fixtures byte-identical, see "Encoding rules" below. The **decoder**,
however, is intentionally more lenient than the encoder: a MessagePack map
that omits an optional field's key entirely (not just sets it to `nil`)
still decodes successfully, with that field defaulting to `None`/absent.
This is `serde`'s ordinary behaviour for `Option<T>` fields with no
`#[serde(deny_unknown_fields)]` or `#[serde(default)]` needed, and the
.NET codec should match it (accept a missing key for an `Option`-shaped
field as `null`) for forward compatibility — a future encoder version is
allowed to stop sending a since-retired optional field without breaking
older readers. This asymmetry (strict encoder, lenient decoder) is
deliberate: it is *never* an excuse to add `skip_serializing_if` back to
an encoder.

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

## Decoder hardening limits (ruling R9 — both codecs must agree)

Both the Rust decoder (`crates/oma-ipc/src/frame.rs`) and the .NET decoder
(Task 3) validate the raw MessagePack structure before deserializing, to
make sure a small header can never trigger an oversized allocation from a
declared length. The limits, and precisely what they count:

- **Nesting depth: 64.** Each array or map entered adds one level; a value
  nested 64 levels deep is the deepest accepted.
- **Element count: 100 000, counted as *entries*, not raw values scanned.**
  For an **array**, this is the number of items (an `array32` header
  declaring more than 100 000 items is rejected outright, before reading
  any of them). For a **map**, this is the number of key/value **pairs** —
  a `map32` header declaring exactly 100 000 pairs passes the count check
  (even though the scanner then visits 200 000 individual values, two per
  pair); only a header declaring *more* than 100 000 pairs is rejected.
  Do **not** double the map limit to "200 000 values" on the .NET side —
  the check is against the pair count the header itself declares.
- **Duplicate keys are rejected.** A map (including the outer envelope
  `{"type", "body"}` and any nested struct-as-map) that repeats the same
  string key twice is malformed and rejected, before deserializing.
- A **trailing byte** after the one top-level MessagePack value is
  malformed and rejected (frames never contain more than one payload).
- An **empty payload** (zero bytes) is rejected.

These are structural/hostile-input checks, independent from the ordinary
serde-level checks that already reject a **missing required field** and
tolerate (ignore) an **unrecognized extra field** — see "Absent optional
fields" above for the one exception (an *optional* field's key may be
omitted, decoding as `None`).

## Regenerating the fixtures

The Rust crate (`crates/oma-ipc`) is the source of truth. To regenerate
all files, from the repository root, in PowerShell. Use `--test-threads=1`
for the write pass: the two tests in `tests/fixtures.rs` otherwise run
concurrently, and with `OMA_WRITE_FIXTURES=1` the decode-and-compare test
can race the write test and read a file before it exists.

```powershell
$env:OMA_WRITE_FIXTURES = '1'
try {
    cargo test -p oma-ipc --test fixtures -- --test-threads=1
} finally {
    Remove-Item Env:OMA_WRITE_FIXTURES
}
cargo test -p oma-ipc --test fixtures
```

The second run (without the environment variable, default threading) must
pass, asserting the freshly written files still match the encoder
byte-for-byte and decode back to the reference messages.

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
