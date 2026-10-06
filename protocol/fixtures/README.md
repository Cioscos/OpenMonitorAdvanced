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

Protocol version 2 (M5a) added the fields marked *v2* below. Protocol version 3 (M6b) added
`Subscribe.smart_enabled_drives`, replaced `service.smart_blocked_by` with the per-drive list
`service.drives`, and added `held` to the snapshot (fields marked *v3*). Protocol version 4 (M7b) added
the five frame messages at the end of the logical content below (fields marked *v4*). Every one is always
present on the wire; the .NET decoder requires them, like the Rust one.

- **`hello.msgpack`**: `Hello { protocol_version: 4, service_version: "0.1.0", pawn_io: "rebootPending" }`.
  *v2*: `pawn_io` is `"ok"`, `"missing"`, `"unavailable"`, `"unknown"` or `"rebootPending"`.
  The Rust decoder alone defaults an absent `pawn_io` to `"unknown"`: a protocol v1 service's `Hello` must
  still decode, so the app reports `Incompatible` instead of retrying a failed decode forever.
- **`subscribe.msgpack`**: `Subscribe { interval_ms: 1000, disabled_modules: ["memory", "psu"], smart_disabled_drives: [KEY_A], smart_enabled_drives: [KEY_B] }`
  with `KEY_A = 589488fb…4d83` (the key of `Samsung SSD 990 PRO 2TB` / `0025_38B1_4150_2A6C.`) and
  `KEY_B = 3ed905bd…4ea6` (`ST2000DM008-2UB102` / `WFL4ABCD`). *v2*: `disabled_modules` (names from
  `MODULES`: `cpu`, `motherboard`, `memory`, `storage`, `controller`, `psu`) and `smart_disabled_drives`
  (at most 64 keys of 64 lowercase hexadecimal characters). *v3*: `smart_enabled_drives` follows the
  same rules and names the drives that are off by default and that the client wants on. The
  service answers `bad_request` to a `Subscribe` with an unknown module, more than 64 keys in
  either list, a malformed key, or a key in both lists.
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

  And the `service` block
  `{active_modules: ["cpu", "motherboard", "storage", "controller"], smart_disabled_drives: [KEY_A], reconfiguration: "pending", drives: [...]}`
  (`reconfiguration` is `"applied"`, `"pending"` or `"failed"`). *v3*: `drives` replaces the v2
  `smart_blocked_by`; each entry is `{physical_drive, key, model, state, blocks_smart}`, in
  `physical_drive` order, with `key` and `model` possibly `nil` and `state` one of `"active"`,
  `"idle"`, `"standby"`, `"unknown"`, `"smartOff"`, `"noMedia"` (a client treats any other value as
  `"unknown"`). `"idle"` is a drive that needs a power check, that Windows reports on and that
  showed no recent activity: the service sends it nothing. The fixture has two:
  1. `{physical_drive: 0, key: Some(KEY_A), model: Some("Samsung SSD 990 PRO 2TB"), state: "smartOff", blocks_smart: false}`.
  2. `{physical_drive: 1, key: None, model: Some("ST2000DM008-2UB102"), state: "standby", blocks_smart: true}`.
- **`snapshot.msgpack`**: `WireSnapshot { seq: 4294967301, timestamp_ms: 1790000000000, values: [Some(45.0), None, Some(-12.5), Some(0.0)], held: [false, false, true, false] }`.
  *v3*: `held` has the length and order of `values`; `true` means the value is kept from an earlier
  measurement. A `held` of another length, or `true` for a `nil` value, is a protocol error; a
  non-finite value becomes `nil` and loses its `held` flag.
- **`snapshot_empty.msgpack`**: `WireSnapshot { seq: 1, timestamp_ms: 0, values: [], held: [] }`.
- **`error.msgpack`**: `WireError { code: "bad_request", message: "Messaggio non valido: è atteso Subscribe" }`.
- **`frames_configure.msgpack`** (*v4*, app → servizio): `FramesConfigure { enabled: true, track_pc_latency: true, track_gpu: false }`.
- **`frames_target.msgpack`** (*v4*, app → servizio): `FramesTarget { pid: Some(25848) }`.
- **`frames_target_none.msgpack`** (*v4*): `FramesTarget { pid: None }`; la chiave `pid` c'è sempre, con `nil`.
- **`frames_status.msgpack`** (*v4*, servizio → app): `FramesStatus { state: "running", detail: None, presentmon_version: Some("2.6.0") }`.
  `state` è `"off"`, `"starting"`, `"running"`, `"denied"`, `"tampered"`, `"missing"` o `"failed"` (costanti
  `frames_state`); un client tratta ogni altro valore come `"failed"`.
- **`presenting_processes.msgpack`** (*v4*, servizio → app): `PresentingProcesses { at_qpc: 380058775270, processes: [...] }` con due voci
  `{pid, name, displayed_fps, present_mode, swapchains}`:
  1. `{25848, "CONTROLResonant.exe", 61.5, "Hardware Composed: Independent Flip", 1}`.
  2. `{1852, "dwm.exe", 20.0, "Hardware: Legacy Flip", 1}`.

  Al massimo 32 voci (`MAX_PRESENTING_PROCESSES`); `displayed_fps` deve essere finito.
- **`frame_batch.msgpack`** (*v4*, servizio → app): `FrameBatch { pid: 25848, frames: [...], dropped: 3 }`. I due frame vengono dalle prime
  due righe di dati di `testdata/presentmon/dlssfg-pcl.csv`; ogni `WireFrame` ha `qpc, swapchain, frame_type, displayed,
  ms_between_presents, ms_between_display_change, ms_until_displayed, ms_app_frametime, ms_pc_latency, ms_gpu_busy, pcl_frame_id`
  (in quest'ordine), con `frame_type` `"app"` in entrambi, `swapchain` 0x22A3569E270 e `displayed` true:
  1. `qpc` 369166005856, `ms_between_presents` 17.1266, `ms_between_display_change` 7.1667, `ms_until_displayed` 11.7554,
     `ms_app_frametime` 17.1706, `ms_pc_latency` 35.5189, `ms_gpu_busy` 16.1228, `pcl_frame_id` 43715.
  2. `qpc` 369166008179, `ms_between_presents` 0.2323, `ms_between_display_change` 10.4468, `ms_until_displayed` 21.9699,
     `ms_app_frametime` 0.1817, `ms_pc_latency` 45.9657, `ms_gpu_busy` 0.2476, `pcl_frame_id` nil (la colonna vale 0).

  `frame_type` è `"app"`, `"generated_intel_xefg"`, `"generated_amd_afmf"`, `"generated_other"` o `"unknown"`. Al massimo 512 frame
  (`MAX_FRAMES_PER_BATCH`); `ms_between_presents` deve essere finito, un `f64` facoltativo non finito diventa `nil`.
  I decoder rifiutano un lotto con più di 512 frame o un elenco con più di 32 processi.

## Drive key vector (`drive_key.json`)

`drive_key.json` is not a message: it is the vector both `drive_key` implementations
(`crates/oma-ipc/src/drive_key.rs`, `service/OpenMonitorAdvanced.Service/Sensors/DriveKey.cs`) must
match. It is a JSON array of `{ "model", "serial", "key" }`. The key of a disk is
`sha256(trim(model) + "\0" + trim(serial))` in lowercase hexadecimal, where `"\0"` is a single NUL
character between the two trimmed texts, UTF-8 encoded, and `trim` is Rust's `str::trim` (Unicode
`White_Space`, so U+001F is kept). `key` is `null` when either text is empty after trimming. The cases
cover leading and trailing spaces, tabs and line breaks, U+00A0 and U+3000, non-ASCII text, U+001F (not
trimmed) and empty texts. It was computed with an independent implementation (Python `hashlib`), not
with either of the two, and is edited by hand; it is not regenerated by `OMA_WRITE_FIXTURES`.

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
| `hello.msgpack` | 80 |
| `subscribe.msgpack` | 243 |
| `schema.msgpack` | 1418 |
| `snapshot.msgpack` | 102 |
| `snapshot_empty.msgpack` | 54 |
| `error.msgpack` | 86 |
| `frames_configure.msgpack` | 67 |
| `frames_target.msgpack` | 33 |
| `frames_target_none.msgpack` | 31 |
| `frames_status.msgpack` | 73 |
| `presenting_processes.msgpack` | 269 |
| `frame_batch.msgpack` | 520 |

`hello.msgpack` starts with `82 a4 74 79 70 65` (a 2-entry fixmap, then the
fixstr `"type"`), as expected for the `{"type": "hello", "body": {...}}`
envelope.

## `load/` (app <-> `oma-load.exe`, load protocol v1)

The stress-test helper protocol (`crates/oma-ipc/src/load.rs`) has its own fixtures in
`protocol/fixtures/load/`: `hello`, `run`, `stop`, `topology`, `progress`, `error`,
`notice`, `phase_done` and `finished` (`.msgpack`, the payload without the length
prefix). Regenerate them only with `OMA_WRITE_FIXTURES=1`, single-threaded:

```powershell
$env:OMA_WRITE_FIXTURES = '1'
cargo test -p oma-ipc --test load_fixtures -- --test-threads=1
Remove-Item Env:OMA_WRITE_FIXTURES
```
