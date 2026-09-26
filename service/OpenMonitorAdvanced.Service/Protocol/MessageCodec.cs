using System.Buffers;
using System.Buffers.Binary;
using System.Text;
using MessagePack;

namespace OpenMonitorAdvanced.Service.Protocol;

/// <summary>
/// Encodes and decodes sensor IPC messages as MessagePack, hand-written with
/// <see cref="MessagePackWriter"/>/<see cref="MessagePackReader"/> to mirror
/// the Rust crate <c>oma-ipc</c> (<c>crates/oma-ipc/src/{message,frame}.rs</c>)
/// field-for-field. See <c>protocol/fixtures/README.md</c> for the wire
/// contract (envelope shape, encoding rules, decoder hardening limits).
/// </summary>
public static class MessageCodec
{
    /// <summary>
    /// Encodes <paramref name="message"/> as a MessagePack payload (no
    /// length prefix — see <see cref="EncodeFrame"/> for that).
    /// </summary>
    public static byte[] EncodePayload(IMessage message)
    {
        var buffer = new ArrayBufferWriter<byte>();
        var writer = new MessagePackWriter(buffer);
        WriteEnvelope(ref writer, message);
        writer.Flush();
        return buffer.WrittenSpan.ToArray();
    }

    /// <summary>
    /// Encodes <paramref name="message"/> as a full frame: a little-endian
    /// <c>u32</c> payload length, followed by the MessagePack payload.
    /// </summary>
    /// <exception cref="ProtocolException">The payload exceeds <see cref="ProtocolConstants.MaxFrameBytes"/>.</exception>
    public static byte[] EncodeFrame(IMessage message)
    {
        var payload = EncodePayload(message);
        if (payload.Length > ProtocolConstants.MaxFrameBytes)
        {
            throw new ProtocolException(
                $"payload of {payload.Length} bytes exceeds the maximum of {ProtocolConstants.MaxFrameBytes} bytes");
        }

        var frame = new byte[4 + payload.Length];
        BinaryPrimitives.WriteUInt32LittleEndian(frame, (uint)payload.Length);
        payload.CopyTo(frame.AsSpan(4));
        return frame;
    }

    /// <summary>
    /// Decodes a MessagePack payload (no length prefix) into an <see cref="IMessage"/>.
    /// Validates the raw bytes' structure (nesting depth, element counts,
    /// duplicate keys) before reading any field, rejects an empty payload
    /// and trailing bytes after the message, and defensively replaces any
    /// non-finite <see cref="SnapshotMessage"/> value with <c>null</c>.
    /// </summary>
    /// <exception cref="ProtocolException">The payload is malformed.</exception>
    public static IMessage DecodePayload(ReadOnlySequence<byte> payload)
    {
        // Both the structural validation pass and the field-reading pass are
        // wrapped in the same try/catch so that DecodePayload's contract
        // ("throws only ProtocolException") holds regardless of which pass a
        // given malformed input is caught by (e.g. invalid UTF-8 in a map
        // key is caught during validation; invalid UTF-8 in a recognized
        // string field is caught during field reading).
        try
        {
            MessageStructureValidator.Validate(payload);
            var reader = new MessagePackReader(payload);
            return ReadEnvelope(ref reader);
        }
        catch (ProtocolException)
        {
            throw;
        }
        catch (Exception ex)
        {
            throw new ProtocolException("failed to decode message", ex);
        }
    }

    // ---- Encoding ----------------------------------------------------

    private static void WriteEnvelope(ref MessagePackWriter w, IMessage message)
    {
        switch (message)
        {
            case Hello hello:
                WriteEnvelopeHeader(ref w, "hello", 2);
                w.Write("protocol_version");
                w.Write(hello.ProtocolVersion);
                w.Write("service_version");
                w.Write(hello.ServiceVersion);
                break;
            case Subscribe subscribe:
                WriteEnvelopeHeader(ref w, "subscribe", 1);
                w.Write("interval_ms");
                w.Write(subscribe.IntervalMs);
                break;
            case SchemaMessage schema:
                WriteEnvelopeHeader(ref w, "schema", 2);
                w.Write("devices");
                w.WriteArrayHeader(schema.Devices.Count);
                foreach (var device in schema.Devices)
                {
                    WriteDevice(ref w, device);
                }

                w.Write("sensors");
                w.WriteArrayHeader(schema.Sensors.Count);
                foreach (var sensor in schema.Sensors)
                {
                    WriteSensor(ref w, sensor);
                }

                break;
            case SnapshotMessage snapshot:
                WriteEnvelopeHeader(ref w, "snapshot", 3);
                w.Write("seq");
                w.Write(snapshot.Seq);
                w.Write("timestamp_ms");
                w.Write(snapshot.TimestampMs);
                w.Write("values");
                w.WriteArrayHeader(snapshot.Values.Count);
                foreach (var value in snapshot.Values)
                {
                    WriteNullableDouble(ref w, value);
                }

                break;
            case ErrorMessage error:
                WriteEnvelopeHeader(ref w, "error", 2);
                w.Write("code");
                w.Write(error.Code);
                w.Write("message");
                w.Write(error.Message);
                break;
            default:
                throw new ProtocolException($"unsupported message type {message.GetType()}");
        }
    }

    private static void WriteEnvelopeHeader(ref MessagePackWriter w, string type, int bodyFieldCount)
    {
        w.WriteMapHeader(2);
        w.Write("type");
        w.Write(type);
        w.Write("body");
        w.WriteMapHeader(bodyFieldCount);
    }

    private static void WriteDevice(ref MessagePackWriter w, WireDevice device)
    {
        w.WriteMapHeader(6);
        w.Write("id");
        w.Write(device.Id);
        w.Write("kind");
        w.Write(device.Kind);
        w.Write("name");
        w.Write(device.Name);
        w.Write("vendor");
        WriteNullableString(ref w, device.Vendor);
        w.Write("properties");
        WriteProperties(ref w, device.Properties);
        w.Write("hint");
        WriteHint(ref w, device.Hint);
    }

    private static void WriteProperties(ref MessagePackWriter w, IReadOnlyDictionary<string, string> properties)
    {
        w.WriteMapHeader(properties.Count);
        foreach (var pair in properties.OrderBy(p => p.Key, Utf8OrdinalComparer.Instance))
        {
            w.Write(pair.Key);
            w.Write(pair.Value);
        }
    }

    /// <summary>
    /// Sorts strings by their UTF-8 byte representation, lexicographically —
    /// exactly what Rust's <c>BTreeMap&lt;String, _&gt;</c> does for free
    /// (ruling R11), since a Rust <c>String</c>'s <c>Ord</c> impl compares
    /// its underlying UTF-8 bytes directly. This is deliberately *not*
    /// <see cref="StringComparer.Ordinal"/>, which compares UTF-16 code
    /// units: for a character outside the Basic Multilingual Plane (encoded
    /// as a surrogate pair in UTF-16, e.g. U+1F600 = <c>D83D DE00</c>) the
    /// two orderings can disagree with a character inside it (e.g. U+FF61 =
    /// <c>FF61</c>) — UTF-16 ordinal would sort U+1F600 first (0xD83D &lt;
    /// 0xFF61), while UTF-8 byte order sorts U+FF61 first (its UTF-8 lead
    /// byte 0xEF is less than U+1F600's lead byte 0xF0).
    /// </summary>
    private sealed class Utf8OrdinalComparer : IComparer<string>
    {
        public static readonly Utf8OrdinalComparer Instance = new();

        public int Compare(string? x, string? y)
        {
            if (ReferenceEquals(x, y))
            {
                return 0;
            }

            if (x is null)
            {
                return -1;
            }

            if (y is null)
            {
                return 1;
            }

            return Encoding.UTF8.GetBytes(x).AsSpan().SequenceCompareTo(Encoding.UTF8.GetBytes(y));
        }
    }

    private static void WriteHint(ref MessagePackWriter w, IdentityHint? hint)
    {
        switch (hint)
        {
            case null:
                w.WriteNil();
                break;
            case CpuHint cpu:
                w.WriteMapHeader(2);
                w.Write("kind");
                w.Write("cpu");
                w.Write("value");
                w.WriteMapHeader(1);
                w.Write("index");
                w.Write(cpu.Index);
                break;
            case StorageHint storage:
                w.WriteMapHeader(2);
                w.Write("kind");
                w.Write("storage");
                w.Write("value");
                w.WriteMapHeader(3);
                w.Write("physical_drive");
                w.Write(storage.PhysicalDrive);
                w.Write("model");
                WriteNullableString(ref w, storage.Model);
                w.Write("serial");
                WriteNullableString(ref w, storage.Serial);
                break;
            case MemoryHint:
                w.WriteMapHeader(2);
                w.Write("kind");
                w.Write("memory");
                w.Write("value");
                w.WriteMapHeader(0);
                break;
            default:
                throw new ProtocolException($"unsupported identity hint type {hint.GetType()}");
        }
    }

    private static void WriteSensor(ref MessagePackWriter w, WireSensor sensor)
    {
        w.WriteMapHeader(7);
        w.Write("device_id");
        w.Write(sensor.DeviceId);
        w.Write("kind");
        w.Write(sensor.Kind);
        w.Write("name");
        w.Write(sensor.Name);
        w.Write("unit");
        w.Write(sensor.Unit);
        w.Write("label_key");
        w.Write(sensor.LabelKey);
        w.Write("label_arg");
        WriteNullableString(ref w, sensor.LabelArg);
        w.Write("category");
        w.Write(sensor.Category);
    }

    private static void WriteNullableString(ref MessagePackWriter w, string? value)
    {
        if (value is null)
        {
            w.WriteNil();
        }
        else
        {
            w.Write(value);
        }
    }

    private static void WriteNullableDouble(ref MessagePackWriter w, double? value)
    {
        if (value is null || !double.IsFinite(value.Value))
        {
            w.WriteNil();
        }
        else
        {
            w.Write(value.Value);
        }
    }

    // ---- Decoding ------------------------------------------------------
    //
    // The whole payload was already structurally validated (depth, element
    // counts, duplicate keys, exactly one top-level value) by
    // MessageStructureValidator before any of this runs, so every
    // ReadMapHeader/ReadArrayHeader/Skip call below is reading bytes already
    // known to be well-formed and bounded.

    private delegate T ReadItem<out T>(ref MessagePackReader reader);

    private static IMessage ReadEnvelope(ref MessagePackReader reader)
    {
        string? type = null;
        ReadOnlySequence<byte>? bodySlice = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "map key");
            switch (key)
            {
                case "type":
                    type = ReadRequiredString(ref reader, "\"type\"");
                    break;
                case "body":
                    var start = reader.Position;
                    reader.Skip();
                    bodySlice = reader.Sequence.Slice(start, reader.Position);
                    break;
                default:
                    reader.Skip();
                    break;
            }
        }

        if (type is null)
        {
            throw new ProtocolException("missing required field \"type\"");
        }

        if (bodySlice is null)
        {
            throw new ProtocolException("missing required field \"body\"");
        }

        var bodyReader = new MessagePackReader(bodySlice.Value);
        return type switch
        {
            "hello" => ReadHello(ref bodyReader),
            "subscribe" => ReadSubscribe(ref bodyReader),
            "schema" => ReadSchema(ref bodyReader),
            "snapshot" => ReadSnapshot(ref bodyReader),
            "error" => ReadError(ref bodyReader),
            _ => throw new ProtocolException($"unknown message type \"{type}\""),
        };
    }

    private static Hello ReadHello(ref MessagePackReader reader)
    {
        uint? protocolVersion = null;
        string? serviceVersion = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "hello body key");
            switch (key)
            {
                case "protocol_version":
                    protocolVersion = reader.ReadUInt32();
                    break;
                case "service_version":
                    serviceVersion = ReadRequiredString(ref reader, "\"service_version\"");
                    break;
                default:
                    reader.Skip();
                    break;
            }
        }

        if (protocolVersion is null)
        {
            throw new ProtocolException("missing required field \"protocol_version\"");
        }

        if (serviceVersion is null)
        {
            throw new ProtocolException("missing required field \"service_version\"");
        }

        return new Hello(protocolVersion.Value, serviceVersion);
    }

    private static Subscribe ReadSubscribe(ref MessagePackReader reader)
    {
        uint? intervalMs = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "subscribe body key");
            if (key == "interval_ms")
            {
                intervalMs = reader.ReadUInt32();
            }
            else
            {
                reader.Skip();
            }
        }

        if (intervalMs is null)
        {
            throw new ProtocolException("missing required field \"interval_ms\"");
        }

        return new Subscribe(intervalMs.Value);
    }

    private static SchemaMessage ReadSchema(ref MessagePackReader reader)
    {
        List<WireDevice>? devices = null;
        List<WireSensor>? sensors = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "schema body key");
            switch (key)
            {
                case "devices":
                    devices = ReadArray(ref reader, ReadDevice);
                    break;
                case "sensors":
                    sensors = ReadArray(ref reader, ReadSensor);
                    break;
                default:
                    reader.Skip();
                    break;
            }
        }

        if (devices is null)
        {
            throw new ProtocolException("missing required field \"devices\"");
        }

        if (sensors is null)
        {
            throw new ProtocolException("missing required field \"sensors\"");
        }

        return new SchemaMessage(devices, sensors);
    }

    private static WireDevice ReadDevice(ref MessagePackReader reader)
    {
        string? id = null;
        string? kind = null;
        string? name = null;
        string? vendor = null;
        Dictionary<string, string>? properties = null;
        IdentityHint? hint = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "device map key");
            switch (key)
            {
                case "id":
                    id = ReadRequiredString(ref reader, "\"id\"");
                    break;
                case "kind":
                    kind = ReadRequiredString(ref reader, "\"kind\"");
                    break;
                case "name":
                    name = ReadRequiredString(ref reader, "\"name\"");
                    break;
                case "vendor":
                    // Option<String> in Rust: nil is a valid value here (unlike the
                    // required string fields above), so read directly instead of
                    // going through ReadRequiredString.
                    vendor = ReadNullableString(ref reader);
                    break;
                case "properties":
                    properties = ReadProperties(ref reader);
                    break;
                case "hint":
                    hint = ReadHint(ref reader);
                    break;
                default:
                    reader.Skip();
                    break;
            }
        }

        if (id is null)
        {
            throw new ProtocolException("missing required field \"id\"");
        }

        if (kind is null)
        {
            throw new ProtocolException("missing required field \"kind\"");
        }

        if (name is null)
        {
            throw new ProtocolException("missing required field \"name\"");
        }

        if (properties is null)
        {
            throw new ProtocolException("missing required field \"properties\"");
        }

        // vendor and hint are Option<T> in Rust: an omitted key decodes as
        // null/None (ruling R10), which is already their default here.
        return new WireDevice(id, kind, name, vendor, properties, hint);
    }

    private static Dictionary<string, string> ReadProperties(ref MessagePackReader reader)
    {
        var count = reader.ReadMapHeader();
        var properties = new Dictionary<string, string>(count, StringComparer.Ordinal);
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "properties key");
            var value = ReadRequiredString(ref reader, "properties value");
            properties[key] = value;
        }

        return properties;
    }

    private static IdentityHint? ReadHint(ref MessagePackReader reader)
    {
        if (reader.TryReadNil())
        {
            return null;
        }

        string? kind = null;
        ReadOnlySequence<byte>? valueSlice = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "hint map key");
            switch (key)
            {
                case "kind":
                    kind = ReadRequiredString(ref reader, "\"kind\"");
                    break;
                case "value":
                    var start = reader.Position;
                    reader.Skip();
                    valueSlice = reader.Sequence.Slice(start, reader.Position);
                    break;
                default:
                    reader.Skip();
                    break;
            }
        }

        if (kind is null)
        {
            throw new ProtocolException("missing required field \"kind\"");
        }

        if (valueSlice is null)
        {
            throw new ProtocolException("missing required field \"value\"");
        }

        var valueReader = new MessagePackReader(valueSlice.Value);
        return kind switch
        {
            "cpu" => ReadCpuHint(ref valueReader),
            "storage" => ReadStorageHint(ref valueReader),
            "memory" => ReadMemoryHint(ref valueReader),
            _ => throw new ProtocolException($"unknown identity hint kind \"{kind}\""),
        };
    }

    private static CpuHint ReadCpuHint(ref MessagePackReader reader)
    {
        uint? index = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "cpu hint key");
            if (key == "index")
            {
                index = reader.ReadUInt32();
            }
            else
            {
                reader.Skip();
            }
        }

        if (index is null)
        {
            throw new ProtocolException("missing required field \"index\"");
        }

        return new CpuHint(index.Value);
    }

    /// <summary>
    /// Reads a <c>Memory {}</c> hint's <c>value</c>: it must itself be a map
    /// (an empty one in practice, but unknown entries are tolerated and
    /// skipped like everywhere else) — matching Rust's <c>Memory {}</c>
    /// variant, whose <c>value</c> is a zero-length map, never <c>nil</c> or
    /// an array. <see cref="MessagePackReader.ReadMapHeader"/> throws if the
    /// next token isn't a map header, which <see cref="DecodePayload"/>'s
    /// outer try/catch turns into a <see cref="ProtocolException"/>.
    /// </summary>
    private static MemoryHint ReadMemoryHint(ref MessagePackReader reader)
    {
        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            ReadRequiredString(ref reader, "memory hint key");
            reader.Skip();
        }

        return new MemoryHint();
    }

    private static StorageHint ReadStorageHint(ref MessagePackReader reader)
    {
        uint? physicalDrive = null;
        string? model = null;
        string? serial = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "storage hint key");
            switch (key)
            {
                case "physical_drive":
                    physicalDrive = reader.ReadUInt32();
                    break;
                case "model":
                    model = ReadNullableString(ref reader);
                    break;
                case "serial":
                    serial = ReadNullableString(ref reader);
                    break;
                default:
                    reader.Skip();
                    break;
            }
        }

        if (physicalDrive is null)
        {
            throw new ProtocolException("missing required field \"physical_drive\"");
        }

        return new StorageHint(physicalDrive.Value, model, serial);
    }

    private static WireSensor ReadSensor(ref MessagePackReader reader)
    {
        string? deviceId = null;
        string? kind = null;
        string? name = null;
        string? unit = null;
        string? labelKey = null;
        string? labelArg = null;
        string? category = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "sensor map key");
            switch (key)
            {
                case "device_id":
                    deviceId = ReadRequiredString(ref reader, "\"device_id\"");
                    break;
                case "kind":
                    kind = ReadRequiredString(ref reader, "\"kind\"");
                    break;
                case "name":
                    name = ReadRequiredString(ref reader, "\"name\"");
                    break;
                case "unit":
                    unit = ReadRequiredString(ref reader, "\"unit\"");
                    break;
                case "label_key":
                    labelKey = ReadRequiredString(ref reader, "\"label_key\"");
                    break;
                case "label_arg":
                    labelArg = ReadNullableString(ref reader);
                    break;
                case "category":
                    category = ReadRequiredString(ref reader, "\"category\"");
                    break;
                default:
                    reader.Skip();
                    break;
            }
        }

        if (deviceId is null)
        {
            throw new ProtocolException("missing required field \"device_id\"");
        }

        if (kind is null)
        {
            throw new ProtocolException("missing required field \"kind\"");
        }

        if (name is null)
        {
            throw new ProtocolException("missing required field \"name\"");
        }

        if (unit is null)
        {
            throw new ProtocolException("missing required field \"unit\"");
        }

        if (labelKey is null)
        {
            throw new ProtocolException("missing required field \"label_key\"");
        }

        if (category is null)
        {
            throw new ProtocolException("missing required field \"category\"");
        }

        return new WireSensor(deviceId, kind, name, unit, labelKey, labelArg, category);
    }

    private static SnapshotMessage ReadSnapshot(ref MessagePackReader reader)
    {
        ulong? seq = null;
        ulong? timestampMs = null;
        List<double?>? values = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "snapshot body key");
            switch (key)
            {
                case "seq":
                    seq = reader.ReadUInt64();
                    break;
                case "timestamp_ms":
                    timestampMs = reader.ReadUInt64();
                    break;
                case "values":
                    values = ReadValues(ref reader);
                    break;
                default:
                    reader.Skip();
                    break;
            }
        }

        if (seq is null)
        {
            throw new ProtocolException("missing required field \"seq\"");
        }

        if (timestampMs is null)
        {
            throw new ProtocolException("missing required field \"timestamp_ms\"");
        }

        if (values is null)
        {
            throw new ProtocolException("missing required field \"values\"");
        }

        return new SnapshotMessage(seq.Value, timestampMs.Value, values);
    }

    private static List<double?> ReadValues(ref MessagePackReader reader)
    {
        var count = reader.ReadArrayHeader();
        var values = new List<double?>(count);
        for (var i = 0; i < count; i++)
        {
            if (reader.TryReadNil())
            {
                values.Add(null);
            }
            else
            {
                var value = reader.ReadDouble();
                values.Add(double.IsFinite(value) ? value : null);
            }
        }

        return values;
    }

    private static ErrorMessage ReadError(ref MessagePackReader reader)
    {
        string? code = null;
        string? message = null;

        var count = reader.ReadMapHeader();
        for (var i = 0; i < count; i++)
        {
            var key = ReadRequiredString(ref reader, "error body key");
            switch (key)
            {
                case "code":
                    code = ReadRequiredString(ref reader, "\"code\"");
                    break;
                case "message":
                    message = ReadRequiredString(ref reader, "\"message\"");
                    break;
                default:
                    reader.Skip();
                    break;
            }
        }

        if (code is null)
        {
            throw new ProtocolException("missing required field \"code\"");
        }

        if (message is null)
        {
            throw new ProtocolException("missing required field \"message\"");
        }

        return new ErrorMessage(code, message);
    }

    private static List<T> ReadArray<T>(ref MessagePackReader reader, ReadItem<T> readItem)
    {
        var count = reader.ReadArrayHeader();
        var list = new List<T>(count);
        for (var i = 0; i < count; i++)
        {
            list.Add(readItem(ref reader));
        }

        return list;
    }

    private static string ReadRequiredString(ref MessagePackReader reader, string fieldName)
    {
        var value = ReadNullableString(ref reader);
        if (value is null)
        {
            throw new ProtocolException($"{fieldName} must not be nil");
        }

        return value;
    }

    /// <summary>
    /// Reads a string value (or <c>nil</c>, as <c>null</c>), decoding its
    /// UTF-8 payload strictly (ruling R12): unlike
    /// <see cref="MessagePackReader.ReadString"/>, which uses .NET's default
    /// lossy UTF-8 decoding (invalid byte sequences silently become U+FFFD),
    /// this throws a <see cref="ProtocolException"/> on invalid UTF-8 —
    /// matching Rust's <c>rmp_serde</c>/<c>serde</c>, which reject invalid
    /// UTF-8 in any string field via <c>std::str::from_utf8</c>.
    /// </summary>
    private static string? ReadNullableString(ref MessagePackReader reader)
    {
        var sequence = reader.ReadStringSequence();
        return sequence is null ? null : StrictUtf8.Decode(sequence.Value);
    }
}

/// <summary>
/// Decodes raw bytes as UTF-8 strictly, throwing a
/// <see cref="ProtocolException"/> on invalid byte sequences instead of
/// .NET's default lossy behaviour (which replaces invalid sequences with
/// U+FFFD). Used everywhere a string is read from the wire — both by
/// <see cref="MessageCodec"/>'s field decoding and by
/// <see cref="MessageStructureValidator"/>'s map-key duplicate check — so
/// invalid UTF-8 is rejected the same way Rust's <c>rmp_serde</c>/
/// <c>serde</c> would (ruling R12).
/// </summary>
internal static class StrictUtf8
{
    private static readonly UTF8Encoding Encoding = new(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);

    public static string Decode(ReadOnlySequence<byte> bytes)
    {
        try
        {
            if (bytes.IsSingleSegment)
            {
                return Encoding.GetString(bytes.FirstSpan);
            }

            var length = checked((int)bytes.Length);
            var array = ArrayPool<byte>.Shared.Rent(length);
            try
            {
                bytes.CopyTo(array);
                return Encoding.GetString(array, 0, length);
            }
            finally
            {
                ArrayPool<byte>.Shared.Return(array);
            }
        }
        catch (DecoderFallbackException ex)
        {
            throw new ProtocolException("invalid UTF-8 in a MessagePack string", ex);
        }
    }
}

/// <summary>
/// Structurally validates a raw MessagePack payload before any field is
/// decoded, mirroring <c>crates/oma-ipc/src/frame.rs</c>'s
/// <c>validate_message</c>/<c>validate_value</c> scanner exactly (ruling
/// R9): the same nesting-depth and element-count limits, applied while
/// skipping unrecognized fields too, and the same duplicate-key rule
/// (compares each map key's <em>decoded</em> UTF-8 payload, not its raw
/// encoded bytes). Never allocates memory proportional to a declared
/// length: strings/bin/ext payloads are only skipped over via
/// <see cref="SequenceReader{T}.Advance"/>, and array/map elements are
/// visited by recursive, bounds-checked scanning of the existing sequence.
/// </summary>
internal static class MessageStructureValidator
{
    private const int MaxDepth = 64;
    private const uint MaxElements = 100_000;

    public static void Validate(ReadOnlySequence<byte> payload)
    {
        if (payload.IsEmpty)
        {
            throw new ProtocolException("empty payload");
        }

        var reader = new SequenceReader<byte>(payload);
        ValidateValue(ref reader, 0);
        if (!reader.End)
        {
            throw new ProtocolException("trailing bytes after the MessagePack message");
        }
    }

    private static void ValidateValue(ref SequenceReader<byte> reader, int depth)
    {
        if (depth > MaxDepth)
        {
            throw new ProtocolException("message nesting exceeds the maximum depth");
        }

        var marker = ReadU8(ref reader);
        switch (marker)
        {
            // positive fixint, negative fixint, nil, false, true
            case <= 0x7f or >= 0xe0 or 0xc0 or 0xc2 or 0xc3:
                return;
            case 0xc1:
                throw new ProtocolException("reserved MessagePack marker 0xc1");
            case 0xc4: // bin8
                Skip(ref reader, ReadU8(ref reader));
                return;
            case 0xc5: // bin16
                Skip(ref reader, ReadU16(ref reader));
                return;
            case 0xc6: // bin32
                Skip(ref reader, ReadU32(ref reader));
                return;
            case 0xc7: // ext8: length, then a 1-byte type tag, then data.
            {
                var n = ReadU8(ref reader);
                Skip(ref reader, 1);
                Skip(ref reader, n);
                return;
            }

            case 0xc8: // ext16
            {
                var n = ReadU16(ref reader);
                Skip(ref reader, 1);
                Skip(ref reader, n);
                return;
            }

            case 0xc9: // ext32
            {
                var n = ReadU32(ref reader);
                Skip(ref reader, 1);
                Skip(ref reader, n);
                return;
            }

            case 0xca: // f32
                Skip(ref reader, 4);
                return;
            case 0xcb: // f64
                Skip(ref reader, 8);
                return;
            case 0xcc: // u8
                Skip(ref reader, 1);
                return;
            case 0xcd: // u16
                Skip(ref reader, 2);
                return;
            case 0xce: // u32
                Skip(ref reader, 4);
                return;
            case 0xcf: // u64
                Skip(ref reader, 8);
                return;
            case 0xd0: // i8
                Skip(ref reader, 1);
                return;
            case 0xd1: // i16
                Skip(ref reader, 2);
                return;
            case 0xd2: // i32
                Skip(ref reader, 4);
                return;
            case 0xd3: // i64
                Skip(ref reader, 8);
                return;
            case 0xd4: // fixext1 (1 type byte + 1 data byte)
                Skip(ref reader, 2);
                return;
            case 0xd5: // fixext2
                Skip(ref reader, 3);
                return;
            case 0xd6: // fixext4
                Skip(ref reader, 5);
                return;
            case 0xd7: // fixext8
                Skip(ref reader, 9);
                return;
            case 0xd8: // fixext16
                Skip(ref reader, 17);
                return;
            case 0xd9: // str8
                Skip(ref reader, ReadU8(ref reader));
                return;
            case 0xda: // str16
                Skip(ref reader, ReadU16(ref reader));
                return;
            case 0xdb: // str32
                Skip(ref reader, ReadU32(ref reader));
                return;
            case 0xdc: // array16
                ValidateElements(ref reader, ReadU16(ref reader), depth);
                return;
            case 0xdd: // array32
                ValidateElements(ref reader, ReadU32(ref reader), depth);
                return;
            case 0xde: // map16
                ValidateMap(ref reader, ReadU16(ref reader), depth);
                return;
            case 0xdf: // map32
                ValidateMap(ref reader, ReadU32(ref reader), depth);
                return;
            case >= 0x80 and <= 0x8f: // fixmap
                ValidateMap(ref reader, (uint)(marker & 0x0f), depth);
                return;
            case >= 0x90 and <= 0x9f: // fixarray
                ValidateElements(ref reader, (uint)(marker & 0x0f), depth);
                return;
            case >= 0xa0 and <= 0xbf: // fixstr
                Skip(ref reader, (uint)(marker & 0x1f));
                return;
            default:
                throw new ProtocolException($"unexpected MessagePack marker 0x{marker:x2}");
        }
    }

    /// <summary>
    /// Validates <paramref name="count"/> array elements (ruling R9: an
    /// array's limit is its item count, checked directly against
    /// <see cref="MaxElements"/>).
    /// </summary>
    private static void ValidateElements(ref SequenceReader<byte> reader, uint count, int depth)
    {
        if (count > MaxElements)
        {
            throw new ProtocolException($"array declares {count} elements, exceeding the {MaxElements} limit");
        }

        for (uint i = 0; i < count; i++)
        {
            ValidateValue(ref reader, depth + 1);
        }
    }

    /// <summary>
    /// Validates <paramref name="count"/> map entries (ruling R9: a map's
    /// limit is its key/value-pair count, not the number of values
    /// scanned). Also rejects a map that repeats the same string key twice,
    /// comparing keys by their decoded UTF-8 content so that e.g. a fixstr
    /// and a str8 encoding of the same text are still caught as duplicates.
    /// </summary>
    private static void ValidateMap(ref SequenceReader<byte> reader, uint count, int depth)
    {
        if (count > MaxElements)
        {
            throw new ProtocolException($"map declares {count} entries, exceeding the {MaxElements} limit");
        }

        // No capacity is reserved from the declared count: at this point the
        // count is only a header value, not yet backed by confirmed entries
        // (a hostile message can chain many map headers through the "key"
        // position of an outer map without ever providing real entries, so
        // pre-sizing from `count` here would let a few hundred bytes of
        // input pin down tens of MB of `HashSet` backing arrays before the
        // depth limit even has a chance to reject the message). Grows
        // organically instead, exactly like the Rust scanner's
        // `HashSet::new()`.
        var seenKeys = new HashSet<string>();
        for (uint i = 0; i < count; i++)
        {
            var keyStart = reader.Position;
            ValidateValue(ref reader, depth + 1);
            var keyPayload = TryGetStringKeyPayload(reader.Sequence.Slice(keyStart, reader.Position));
            if (keyPayload is not null && !seenKeys.Add(keyPayload))
            {
                throw new ProtocolException("map has a duplicate key");
            }

            ValidateValue(ref reader, depth + 1);
        }
    }

    /// <summary>
    /// If <paramref name="keyBytes"/> (a fully-scanned MessagePack value) is
    /// a string (fixstr, str8, str16 or str32), returns its decoded UTF-8
    /// payload — with the marker and length header stripped off, so two
    /// keys encoding the same string with different marker families
    /// compare equal.
    /// </summary>
    private static string? TryGetStringKeyPayload(ReadOnlySequence<byte> keyBytes)
    {
        var keyReader = new SequenceReader<byte>(keyBytes);
        if (!keyReader.TryRead(out var marker))
        {
            return null;
        }

        int length;
        switch (marker)
        {
            case >= 0xa0 and <= 0xbf:
                length = marker & 0x1f;
                break;
            case 0xd9:
                length = ReadU8(ref keyReader);
                break;
            case 0xda:
                length = ReadU16(ref keyReader);
                break;
            case 0xdb:
                length = checked((int)ReadU32(ref keyReader));
                break;
            default:
                return null;
        }

        var payload = keyBytes.Slice(keyReader.Position, length);

        // Strict decoding (ruling R12): a key with invalid UTF-8 is
        // rejected, matching Rust's serde-driven map-key deserialization.
        return StrictUtf8.Decode(payload);
    }

    private static ProtocolException Eof()
    {
        return new ProtocolException("unexpected end of message");
    }

    private static byte ReadU8(ref SequenceReader<byte> reader)
    {
        if (!reader.TryRead(out var value))
        {
            throw Eof();
        }

        return value;
    }

    private static ushort ReadU16(ref SequenceReader<byte> reader)
    {
        if (!reader.TryReadBigEndian(out short value))
        {
            throw Eof();
        }

        return unchecked((ushort)value);
    }

    private static uint ReadU32(ref SequenceReader<byte> reader)
    {
        if (!reader.TryReadBigEndian(out int value))
        {
            throw Eof();
        }

        return unchecked((uint)value);
    }

    /// <summary>
    /// Skips <paramref name="count"/> bytes without allocating: this only
    /// ever moves <paramref name="reader"/>'s position over bytes already
    /// present in the backing <see cref="ReadOnlySequence{T}"/>.
    /// </summary>
    private static void Skip(ref SequenceReader<byte> reader, uint count)
    {
        if (count > reader.Remaining)
        {
            throw Eof();
        }

        reader.Advance(count);
    }
}
