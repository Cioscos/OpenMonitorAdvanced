# Spike S2 — MessagePack byte-identity between Rust and .NET

**Status:** feasible, verified. **Scratch code:** throwaway, not merged; lived under
`<scratchpad>\m4\msgpack\` (Rust crate `rs/`, .NET console app
`dotnet/MsgpackSpike/`, shared `protocol/fixtures/`). Nothing in the repo was
changed except this report. Source excerpts kept for reproduction:
[`code/s2-msgpack-rust/`](code/s2-msgpack-rust/) (`gen_fixtures.rs`, `check_decode.rs`, `Cargo.toml`)
and [`code/s2-msgpack-dotnet/`](code/s2-msgpack-dotnet/) (`Program.cs`, `MsgpackSpike.csproj`).

## Result

All 6 representative protocol messages (`Hello`, `Subscribe`, `Schema`,
`Snapshot`, `Snapshot` with empty `values`, `Error`) — including the required
edge cases — serialize to **byte-for-byte identical** MessagePack in Rust
(`rmp-serde` 1.3.1) and .NET (`MessagePack` / MessagePack-CSharp 3.1.10).
Cross-decoding works both ways:
- `rmp-serde` decodes the .NET-produced bytes and re-encodes them to the same
  bytes (round trip verified for all 6 fixtures).
- `MessagePackReader` decodes the Rust-produced bytes correctly (`hello` and
  the full nested `schema`, including the `hint` tagged union and unicode
  strings).

Package versions: `rmp-serde = "1"` → resolved to **1.3.1** (`rmp 0.8.15`),
`serde = "1"` (1.0.229) on Rust 1.90.0; **MessagePack 3.1.10** on .NET 10.0.303.

## Envelope design chosen: adjacently tagged `{ "type", "body" }`

Two designs were evaluated:

- **(A) `{"type": "hello", "body": {...}}`** (serde: `#[serde(tag = "type",
  content = "body")]`, an *adjacently tagged* enum) — **chosen**.
- **(B) `{"hello": {...}}`** (serde's default *externally tagged* enum,
  single-key map) — works in Rust for free but has no first-class
  representation in MessagePack-CSharp; replicating it on the .NET side needs
  fully custom formatter code with no attribute support, and matching the
  single dynamic key name against Rust's enum-variant renaming is one more
  place to get subtly wrong. Not implemented for this spike beyond this
  assessment.

(A) wins because both fields have fixed, known names — it degrades to "two
ordinary struct fields" on both sides, which is trivial to hand-encode (or to
drive from attributes) identically, and it is easy to reason about
correctness for: unknown message type → read `type` as a plain string, switch
on it, produce a clean `Err`/exception for anything unrecognized (verified,
see below) — no need to know the full closed set of variant names up front to
even get *that far* into decoding, unlike (B) where an unrecognized single key
fails at the point of choosing which variant to deserialize into with less
diagnostic context.

The same pattern (`tag`/`content`) was reused for the nested `IdentityHint`
union, with field names `kind` (the tag) and `value` (the content), to avoid
colliding with `Device.kind`:

```json
{ "kind": "cpu", "value": { "index": 0 } }
{ "kind": "storage", "value": { "physical_drive": 0, "serial": "S6XPNX0T..." } }
{ "kind": "memory", "value": {} }
```

## Wire encoding recap (per spec §6)

- Structs are maps with string keys **in declared order** — not alphabetical.
- Integers use the shortest MessagePack form (fixint/uint8/16/32/64 as
  needed) — both libraries do this automatically for unsigned/plain integer
  writes; no special configuration needed.
- `f64` is **always** float64 (0xcb), never float32, even for whole numbers
  (`45.0`, `0.0`). Both libraries default to this for `f64`/`double`;
  confirmed for `45.0`, `-12.5`, `0.0`, `100.0`.
- Absent optional values are `nil` (`0xc0`), and the **key is still present**
  — nothing is omitted. See pitfall #1 below; this is the one place the two
  sides can silently diverge.
- Empty arrays/maps are `0x90`/`0x80` (zero-length), not omitted or `nil`
  (verified for `Snapshot.values = []` and `Device.properties = {}`).
- `properties: map<string, string>` is a `BTreeMap` in Rust — its `Serialize`
  impl iterates in sorted key order for free, matching "sorted by key" from
  the spec without any extra code.

## Rust side — exact calls that worked

See [`code/s2-msgpack-rust/gen_fixtures.rs`](code/s2-msgpack-rust/gen_fixtures.rs)
for the full working spike code. Key excerpt:

```rust
// Cargo.toml
// serde = { version = "1", features = ["derive"] }
// rmp-serde = "1"

#[derive(Serialize, Deserialize)]
struct Hello { protocol_version: u32, service_version: String }

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum IdentityHint {
    Cpu { index: u32 },
    Storage { physical_drive: u32, serial: Option<String> }, // NOT skip_serializing_if — see pitfall #1
    Memory {},
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
enum Envelope {
    Hello(Hello),
    Subscribe(Subscribe),
    Schema(Schema),
    Snapshot(Snapshot),
    Error(Error),
}

let bytes = rmp_serde::to_vec_named(&envelope)?;   // struct-as-map, declared field order
let envelope: Envelope = rmp_serde::from_slice(&bytes)?;
```

`rmp_serde::to_vec_named` is the one call needed — it is the "encode structs
as maps keyed by field name, in declared order" mode (as opposed to
`to_vec`, which encodes structs as plain arrays positionally and would not be
readable by a MessagePack-CSharp side expecting maps). No
`Serializer::new(..).with_struct_map()` boilerplate needed unless writing to
a custom `io::Write`; `to_vec_named` wraps that for you.

Field names are already `snake_case` in the struct definitions here, so no
`rename_all` was needed on `Hello`/`Subscribe`/`Schema`/`Sensor`/`Device`/
`Snapshot`/`Error` — only the two tagged enums needed
`rename_all = "snake_case"` (for variant names becoming lowercase strings on
the wire) and explicit `tag`/`content`.

**Note:** the shared `oma-core::model` structs (`Device`, `Sensor`, `Schema`,
`Snapshot`, `Label`) use `#[serde(rename_all = "camelCase")]` for the
Rust↔UI JSON contract — deliberately different from the wire IPC contract.
IPC structs must be **separate types** in a `protocol` crate/module with
their own `snake_case` derives; do not reuse `oma-core::model` types directly
for IPC serialization, or a rename on one side will silently break the other.

## .NET side — exact approach that worked

Attribute-based `[MessagePackObject]`/`[Key("name")]` mapping was **not**
used for the final approach. Given the union types (`Envelope`,
`IdentityHint`) have no native representation in MessagePack-CSharp's
resolvers, and cross-language field-order guarantees from reflection-based
resolvers are not something to rely on for a "prove it's byte-identical"
spike, the working code instead builds every message with the **low-level
`MessagePackWriter` API** directly, mirroring the Rust field order verbatim
(full working code: [`code/s2-msgpack-dotnet/Program.cs`](code/s2-msgpack-dotnet/Program.cs)):

```csharp
using MessagePack; // MessagePack 3.1.10, MessagePack.MessagePackWriter

// MessagePackWriter is a mutable struct — pass it with a ref-taking
// delegate, never System.Action<MessagePackWriter> (which copies it and
// silently discards all writes). See pitfall #2.
delegate void WriteFn(ref MessagePackWriter w);

static void WriteEnvelope(ref MessagePackWriter w, string type, WriteFn writeBody)
{
    w.WriteMapHeader(2);
    w.Write("type"); w.Write(type);
    w.Write("body");
    var inner = new ArrayBufferWriter<byte>();
    var iw = new MessagePackWriter(inner);
    writeBody(ref iw);
    iw.Flush();
    w.WriteRaw(inner.WrittenSpan);
}

// e.g. Hello:
w.WriteMapHeader(2);
w.Write("protocol_version"); w.Write(1u);       // MessagePackWriter.Write(uint) -> shortest form
w.Write("service_version"); w.Write("0.4.0");   // Write(string) -> str8/16/32 as needed
// Option<f64>::None -> w.WriteNil();
// f64 always -> w.Write(double) always emits float64 (0xcb), never float32.
```

Decoding uses the mirror-image `MessagePackReader` (`ReadMapHeader`,
`ReadArrayHeader`, `ReadString`, `ReadUInt32`/`ReadUInt64`, `TryReadNil`,
`Skip()` for forward-compatible ignoring of unknown keys) — this was used to
decode Rust's `hello.msgpack` and the full nested `schema.msgpack`
(devices/sensors/hint union/unicode names), both successfully.

**Recommendation for the real implementation** (not built in this spike,
since it's out of scope for a throwaway feasibility check): write one small
hand-rolled `IMessagePackFormatter<T>` per message/sub-message type using
this same `MessagePackWriter`/`MessagePackReader` pattern, registered in a
custom resolver, rather than relying on attribute-driven automatic mapping.
This keeps the field order and shape under explicit version control on both
sides of the wire and is what `protocol/fixtures/*.msgpack` should be
generated/asserted against in unit tests on both languages.

`MessagePackSerializerOptions`/`MessagePackSecurity` were exercised only for
the untrusted-input question (see below); they were not needed for the
low-level writer/reader path used for the fixtures themselves.

## Byte-mismatch pitfalls hit (and fixes)

1. **`skip_serializing_if` desyncs the two sides.** First draft had
   `#[serde(skip_serializing_if = "Option::is_none")]` on
   `IdentityHint::Storage.serial`. When `serial` is `None`, Rust then *omits*
   the key entirely (map size drops from 2 to 1), while the natural .NET
   encoding (or any spec-compliant reader) always writes the key with `nil`.
   Result: same logical value, different byte length and map header — the
   `schema` fixture came out 912 vs 920 bytes.
   **Fix:** never use `skip_serializing_if` on wire-protocol structs; every
   declared field is always present, using `nil` for "absent", exactly as
   spec §6 says. This should probably be enforced as a lint/review rule for
   the real protocol module — a single field getting an errant
   `skip_serializing_if` breaks byte-identity in a way tests will only catch
   if a fixture asserts on the exact byte length/hash, not just round-trip
   equality within one language.
2. **`MessagePackWriter` is a mutable struct; capturing it by `Action<T>`
   silently no-ops.** First C# draft passed `MessagePackWriter` into
   `Action<MessagePackWriter>` callbacks; because the delegate parameter is
   passed **by value**, every write inside the callback happened to a copy,
   and the outer writer's `Flush()` produced 0 bytes. Fixed by declaring a
   custom `delegate void WriteFn(ref MessagePackWriter w)` and threading `ref`
   through every helper. Anyone doing manual `MessagePackWriter` encoding in
   C# needs to know this — it's an easy, silent (no exception) bug.
3. **NaN bit patterns differ between Rust and .NET, confirming NaN must never
   be sent.** `f64::NAN.to_bits()` in Rust = `0x7FF8000000000000` (quiet NaN,
   sign bit 0). `double.NaN` bit pattern in .NET (`BitConverter.
   DoubleToUInt64Bits`) = `0xFFF8000000000000` (sign bit **1**). Both are
   valid IEEE-754 quiet NaNs and both libraries happily serialize them as an
   ordinary float64 — MessagePack itself has no special NaN handling — but
   the two languages' default NaN constants are bit-for-bit different, so a
   naive "serialize whatever f64 you have" would break byte-identity even
   before considering that NaN is semantically meaningless on this wire.
   `double.PositiveInfinity`/`NegativeInfinity` do round-trip identically to
   Rust's `f64::INFINITY`/`NEG_INFINITY` (`0x7FF0000000000000` /
   `0xFFF0000000000000` on both sides) — only NaN's payload/sign differs.
   **Recommendation (already implied by the schema):** `values: Vec<Option<f64>>`
   is the correct design — reject/assert that no `Some(f64)` pushed onto a
   `Snapshot` or read from a provider is NaN or infinite; a provider that
   can't produce a valid reading must emit `None`, not `Some(NaN)`. Add a
   debug assertion (`debug_assert!(v.is_finite())`) at the point values are
   inserted into a `Snapshot`, and treat a NaN/Infinite reading from any
   provider as a bug to fix at the source, not something the protocol layer
   should paper over.

## Untrusted-input limits (.NET side)

`MessagePack.MessagePackSecurity` is the relevant API:
- `MessagePackSecurity.UntrustedData` and `.TrustedData` both currently
  default `MaximumObjectGraphDepth` to **500** in MessagePack-CSharp 3.1.10
  (verified by printing both at runtime) — the meaningful difference between
  the two presets is in other protections (e.g. resistance to
  hash-collision-DoS on dictionary/string keys during deserialization), not
  this depth number.
- `security.WithMaximumObjectGraphDepth(n)` returns a new immutable
  `MessagePackSecurity` with a custom depth cap (verified: `.WithMaximumObjectGraphDepth(50)` → `50`).
- Apply via `MessagePackSerializerOptions.Standard.WithSecurity(MessagePackSecurity.UntrustedData)`
  and pass those options to every `MessagePackSerializer.Deserialize` call
  that reads from the named pipe (the client always should, since a
  compromised/pre-service-start impostor could be on the other end per spec
  §6's "server verification" note).
- This only matters if/when the real implementation uses the
  attribute+resolver-driven `MessagePackSerializer.Serialize/Deserialize<T>`
  API. The manual `MessagePackWriter`/`MessagePackReader` path used for this
  spike's fixtures does not go through `MessagePackSecurity` at all — depth
  and size protection for that path has to be enforced by hand (e.g. the
  spec's existing 4 MB frame-size cap already bounds the attack surface
  significantly, and a hand-written reader can simply cap array/map header
  counts against a sane maximum before allocating).

Rust's `rmp-serde` has no analogous configurable depth/size cap API; nesting
depth is bounded only by the recursion limit of the `Deserialize` impls
generated by `serde_derive` (effectively the call stack) and, again, by the
spec's 4 MB frame cap and any additional array/map-length sanity checks the
real client should add explicitly when decoding data from an
`Administrators`+interactive-writable named pipe.

## rmp-serde behaviour on schema mismatches (verified)

Using hand-crafted MessagePack maps decoded via `rmp_serde::from_slice::<Envelope>`
(see [`code/s2-msgpack-rust/check_decode.rs`](code/s2-msgpack-rust/check_decode.rs)):

| Case | Result |
|---|---|
| Unknown envelope tag (`"type": "ping"`) | Clean `Err`: `unknown variant `ping`, expected one of `hello`, `subscribe`, `schema`, `snapshot`, `error`` |
| Missing required field (`Hello` body without `service_version`) | Clean `Err`: `missing field `service_version`` |
| Extra/unknown field in a body map (`Subscribe` body with an extra `future_field_not_yet_known` key) | **Tolerated** — decodes successfully, extra key ignored, since no `#[serde(deny_unknown_fields)]` is set. This is the forward-compatible behaviour the spec wants; it must **not** be added to the real protocol structs. |

No panics in any case — all three are ordinary `Result::Err` or `Ok`,
suitable for the client to log-and-disconnect (spec §6 already asks for this
on server-identity mismatch; the same pattern applies to malformed frames).
MessagePack-CSharp's attribute-driven deserializer has equivalent behaviour
(unknown keys skipped by default under `MessagePackSerializerOptions.Standard`;
missing required constructor/property values throw `MessagePackSerializationException`)
but this was not separately exercised in this spike since the .NET side used
the manual reader/writer path throughout.

## Fixture files produced (for reference — not committed; regenerate from the plan)

`hello.msgpack` (58 B), `subscribe.msgpack` (37 B), `schema.msgpack` (920 B,
5 devices incl. one of each `IdentityHint` variant + one without a hint, 2
sensors incl. unicode `label_text`), `snapshot.msgpack` (101 B, `seq` and
`timestamp_ms` both above `u32::MAX`, a negative value, a `nil` value, `0.0`
and a positive value), `snapshot_empty_values.msgpack` (48 B, `values: []`),
`error.msgpack` (121 B, unicode message). All six were regenerated on the
.NET side and diffed byte-for-byte against the Rust originals — identical
every time across repeated runs (integer/float encoding and map/array
headers are deterministic on both sides; no problematic HashMap-iteration-
order or similar nondeterminism was involved anywhere since `properties` is
a `BTreeMap` and no other unordered collection appears on the wire). The
`.msgpack` binary fixtures themselves are not kept in this reference folder
(binary, throwaway); regenerate them from the code in `code/s2-msgpack-rust/`
and `code/s2-msgpack-dotnet/` if needed.

## Recommendation

1. Use the adjacently-tagged envelope, `{"type": <str>, "body": <map>}`, for
   both the outer message envelope and the `IdentityHint` union (with
   `kind`/`value` instead of `type`/`body` there, to avoid the field-name
   clash with `Device.kind`).
2. On Rust: a dedicated `oma-core::protocol` (or similar) module with its own
   snake_case structs + `#[serde(tag = "...", content = "...")]` enums,
   entirely separate from `oma-core::model`'s camelCase UI-facing types.
   Encode with `rmp_serde::to_vec_named`, decode with `rmp_serde::from_slice`.
   Never annotate wire structs with `skip_serializing_if`.
3. On .NET: hand-write `IMessagePackFormatter<T>` implementations per
   message/union type using `MessagePackWriter`/`MessagePackReader` directly,
   mirroring the Rust field order field-for-field, rather than trusting
   attribute-driven resolver ordering across languages. Wrap the mutable
   `MessagePackWriter`/`MessagePackReader` structs behind `ref`-taking
   delegates, not `Action<T>`/`Func<T>`.
4. Guard `Option<f64>` values at the point of construction (both providers
   and any test/mock data) to reject NaN/Infinity — `debug_assert!(v.is_finite())`
   in Rust, an equivalent guard clause in the .NET client if it ever
   constructs outgoing values (it currently only reads `Snapshot`s, so this
   is primarily a Rust-service-side rule).
5. `protocol/fixtures/*.msgpack` (spec §6) should be generated once by the
   Rust side (source of truth) and both sides' test suites should assert
   their own encoding of the same logical fixture is byte-identical to the
   file (not just "my own round trip works"), exactly as this spike did.
6. Apply `MessagePackSecurity.UntrustedData` (with `WithMaximumObjectGraphDepth`
   tightened if the real message nesting depth is much shallower than 500,
   which it is — 3 levels deep at most) to every .NET-side deserialize call
   reading from the pipe, on top of the existing 4 MB frame-size cap.
