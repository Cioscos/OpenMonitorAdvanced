using System.Buffers;
using System.Buffers.Binary;
using MessagePack;
using OpenMonitorAdvanced.Service.Protocol;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Protocol;

/// <summary>
/// Byte-parity and hostile-input tests for <see cref="MessageCodec"/> and
/// <see cref="FrameReader"/>, mirroring <c>crates/oma-ipc</c>'s own test
/// suite (<c>crates/oma-ipc/src/frame.rs</c>) and asserting against the
/// shared fixtures in <c>protocol/fixtures/*.msgpack</c> (see
/// <c>protocol/fixtures/README.md</c> for the logical content of each one).
/// </summary>
public sealed class CodecTests
{
    public static readonly TheoryData<string> FixtureNames =
    [
        "hello", "subscribe", "schema", "snapshot", "snapshot_empty", "error",
    ];

    [Theory]
    [MemberData(nameof(FixtureNames))]
    public void FixtureIsEncodedByteForByte(string name)
    {
        var expected = ReadFixture(name);
        var actual = MessageCodec.EncodePayload(Reference(name));
        Assert.Equal(expected, actual);
    }

    [Theory]
    [MemberData(nameof(FixtureNames))]
    public void FixtureDecodesToTheReference(string name)
    {
        var bytes = ReadFixture(name);
        var decoded = MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes));

        // Re-encoding the decoded message must reproduce the exact fixture bytes.
        var reEncoded = MessageCodec.EncodePayload(decoded);
        Assert.Equal(bytes, reEncoded);

        // Scalar fields (and, for collection-bearing messages, their elements'
        // scalar fields) must match the logical reference from the README.
        // Comparing whole records containing IReadOnlyList/IReadOnlyDictionary
        // properties directly would rely on those collections' own (reference)
        // equality, not a structural one, so collection-bearing cases are
        // compared field-by-field below instead.
        switch (name)
        {
            case "hello":
            case "subscribe":
            case "error":
                Assert.Equal(Reference(name), decoded);
                break;
            case "snapshot":
            case "snapshot_empty":
                var expectedSnapshot = (SnapshotMessage)Reference(name);
                var actualSnapshot = Assert.IsType<SnapshotMessage>(decoded);
                Assert.Equal(expectedSnapshot.Seq, actualSnapshot.Seq);
                Assert.Equal(expectedSnapshot.TimestampMs, actualSnapshot.TimestampMs);
                Assert.Equal(expectedSnapshot.Values, actualSnapshot.Values);
                break;
            case "schema":
                var expectedSchema = (SchemaMessage)Reference(name);
                var actualSchema = Assert.IsType<SchemaMessage>(decoded);
                Assert.Equal(expectedSchema.Devices.Count, actualSchema.Devices.Count);
                for (var i = 0; i < expectedSchema.Devices.Count; i++)
                {
                    AssertDeviceEqual(expectedSchema.Devices[i], actualSchema.Devices[i]);
                }

                Assert.Equal(expectedSchema.Sensors.Count, actualSchema.Sensors.Count);
                for (var i = 0; i < expectedSchema.Sensors.Count; i++)
                {
                    AssertSensorEqual(expectedSchema.Sensors[i], actualSchema.Sensors[i]);
                }

                break;
        }
    }

    [Fact]
    public void NonFiniteValuesAreWrittenAsNil()
    {
        var message = new SnapshotMessage(1, 0, new double?[] { double.NaN, double.PositiveInfinity });
        var payload = MessageCodec.EncodePayload(message);
        var decoded = Assert.IsType<SnapshotMessage>(MessageCodec.DecodePayload(new ReadOnlySequence<byte>(payload)));
        Assert.Equal(new double?[] { null, null }, decoded.Values);
    }

    [Fact]
    public async Task OversizedFrameThrowsBeforeReadingTheBody()
    {
        var frame = new byte[20];
        BinaryPrimitives.WriteUInt32LittleEndian(frame, ProtocolConstants.MaxFrameBytes + 1);
        var stream = new CountingReadStream(frame);

        await Assert.ThrowsAsync<ProtocolException>(async () => await FrameReader.ReadAsync(stream, CancellationToken.None));

        Assert.Equal(4, stream.TotalBytesRead);
    }

    [Fact]
    public void UnknownTypeThrows()
    {
        var bytes = BuildEnvelope("ping", (ref MessagePackWriter w) => w.WriteMapHeader(0));
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void ExtraFieldsAreSkipped()
    {
        var bytes = BuildEnvelope("subscribe", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(2);
            w.Write("interval_ms");
            w.Write(500u);
            w.Write("extra");
            w.Write(1);
        });

        var decoded = Assert.IsType<Subscribe>(MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.Equal(500u, decoded.IntervalMs);
    }

    [Fact]
    public async Task CleanEndOfStreamReturnsNull()
    {
        var stream = new MemoryStream([]);
        var result = await FrameReader.ReadAsync(stream, CancellationToken.None);
        Assert.Null(result);
    }

    [Fact]
    public void EmptyPayloadIsRejected()
    {
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(ReadOnlySequence<byte>.Empty));
    }

    [Fact]
    public void TrailingBytesAreRejected()
    {
        var payload = MessageCodec.EncodePayload(new Subscribe(500));
        var bytes = new byte[payload.Length + 1];
        payload.CopyTo(bytes, 0);
        bytes[^1] = 0xc0; // an extra nil byte tacked on after a valid message
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void MissingRequiredFieldIsRejected()
    {
        var bytes = BuildEnvelope("subscribe", (ref MessagePackWriter w) => w.WriteMapHeader(0));
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void DuplicateFieldInBodyIsRejected()
    {
        var bytes = BuildEnvelope("subscribe", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(2);
            w.Write("interval_ms");
            w.Write(500u);
            w.Write("interval_ms");
            w.Write(999u);
        });

        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void Array32DeclaringUInt32MaxElementsIsRejected()
    {
        byte[] bytes = [0xdd, 0xff, 0xff, 0xff, 0xff];
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void Map32OverLimitIsRejected()
    {
        var bytes = new byte[5];
        bytes[0] = 0xdf;
        BinaryPrimitives.WriteUInt32BigEndian(bytes.AsSpan(1), 100_001);
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void NestingDeeperThan64IsRejected()
    {
        var bytes = new byte[66];
        for (var i = 0; i < 65; i++)
        {
            bytes[i] = 0x91; // fixarray, 1 element
        }

        bytes[65] = 0xc0; // innermost nil
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public async Task PartialFrameAtEofIsAnError()
    {
        var frame = MessageCodec.EncodeFrame(new Subscribe(500));
        var truncated = frame[..^2];
        var stream = new MemoryStream(truncated);

        await Assert.ThrowsAsync<ProtocolException>(async () => await FrameReader.ReadAsync(stream, CancellationToken.None));
    }

    private static void AssertDeviceEqual(WireDevice expected, WireDevice actual)
    {
        Assert.Equal(expected.Id, actual.Id);
        Assert.Equal(expected.Kind, actual.Kind);
        Assert.Equal(expected.Name, actual.Name);
        Assert.Equal(expected.Vendor, actual.Vendor);
        AssertPropertiesEqual(expected.Properties, actual.Properties);
        AssertHintEqual(expected.Hint, actual.Hint);
    }

    private static void AssertPropertiesEqual(IReadOnlyDictionary<string, string> expected, IReadOnlyDictionary<string, string> actual)
    {
        Assert.Equal(expected.Count, actual.Count);
        foreach (var (key, value) in expected)
        {
            Assert.True(actual.TryGetValue(key, out var actualValue), $"missing property key \"{key}\"");
            Assert.Equal(value, actualValue);
        }
    }

    private static void AssertHintEqual(IdentityHint? expected, IdentityHint? actual)
    {
        switch (expected)
        {
            case null:
                Assert.Null(actual);
                break;
            case CpuHint cpu:
                var actualCpu = Assert.IsType<CpuHint>(actual);
                Assert.Equal(cpu.Index, actualCpu.Index);
                break;
            case StorageHint storage:
                var actualStorage = Assert.IsType<StorageHint>(actual);
                Assert.Equal(storage.PhysicalDrive, actualStorage.PhysicalDrive);
                Assert.Equal(storage.Model, actualStorage.Model);
                Assert.Equal(storage.Serial, actualStorage.Serial);
                break;
            case MemoryHint:
                Assert.IsType<MemoryHint>(actual);
                break;
        }
    }

    private static void AssertSensorEqual(WireSensor expected, WireSensor actual)
    {
        Assert.Equal(expected.DeviceId, actual.DeviceId);
        Assert.Equal(expected.Kind, actual.Kind);
        Assert.Equal(expected.Name, actual.Name);
        Assert.Equal(expected.Unit, actual.Unit);
        Assert.Equal(expected.LabelKey, actual.LabelKey);
        Assert.Equal(expected.LabelArg, actual.LabelArg);
        Assert.Equal(expected.Category, actual.Category);
    }

    /// <summary>The logical message each fixture holds, per <c>protocol/fixtures/README.md</c>.</summary>
    private static IMessage Reference(string name) => name switch
    {
        "hello" => new Hello(1, "0.1.0"),
        "subscribe" => new Subscribe(1000),
        "schema" => new SchemaMessage(
            [
                new WireDevice(
                    "lhm-cpu", "cpu", "AMD Ryzen 9 7950X3D", "AMD",
                    new Dictionary<string, string>(), new CpuHint(0)),
                new WireDevice(
                    "lhm-nvme0", "storage", "Samsung SSD 990 PRO 2TB", null,
                    new Dictionary<string, string> { ["firmware"] = "4B2QJXD7" },
                    new StorageHint(0, "Samsung SSD 990 PRO 2TB", "0025_38B1_4150_2A6C.")),
                new WireDevice(
                    "lhm-hdd1", "storage", "ST2000DM008", null,
                    new Dictionary<string, string>(),
                    new StorageHint(1, "ST2000DM008-2UB102", null)),
                new WireDevice(
                    "lhm-ram", "memory", "Memory", null,
                    new Dictionary<string, string> { ["dimm0.size"] = "32 GB", ["dimm0.speedMts"] = "6000" },
                    new MemoryHint()),
                new WireDevice(
                    "lhm-mb", "motherboard", "Nuvoton NCT6799D", null,
                    new Dictionary<string, string>(), null),
            ],
            [
                new WireSensor(
                    "lhm-cpu", "temperature", "package", "celsius",
                    "cpu.temperature.package", null, "temperature"),
                new WireSensor(
                    "lhm-mb", "fan", "lhm-fan-1", "rpm",
                    "lhm.raw", "Ventola n.1 — °C", "fan"),
                new WireSensor(
                    "lhm-nvme0", "percent", "wear", "percent",
                    "storage.percentUsed", null, "percent"),
            ]),
        "snapshot" => new SnapshotMessage(
            4_294_967_301UL, 1_790_000_000_000UL,
            new double?[] { 45.0, null, -12.5, 0.0 }),
        "snapshot_empty" => new SnapshotMessage(1, 0, Array.Empty<double?>()),
        "error" => new ErrorMessage("bad_request", "Messaggio non valido: è atteso Subscribe"),
        _ => throw new ArgumentOutOfRangeException(nameof(name), name, "unknown fixture name"),
    };

    private static byte[] ReadFixture(string name)
    {
        var path = Path.Combine(AppContext.BaseDirectory, "Fixtures", name + ".msgpack");
        return File.ReadAllBytes(path);
    }

    private delegate void WriteBody(ref MessagePackWriter w);

    private static byte[] BuildEnvelope(string type, WriteBody writeBody)
    {
        var buffer = new ArrayBufferWriter<byte>();
        var writer = new MessagePackWriter(buffer);
        writer.WriteMapHeader(2);
        writer.Write("type");
        writer.Write(type);
        writer.Write("body");
        writeBody(ref writer);
        writer.Flush();
        return buffer.WrittenSpan.ToArray();
    }

    /// <summary>
    /// A <see cref="Stream"/> wrapper counting the total bytes actually read
    /// from it, so <see cref="OversizedFrameThrowsBeforeReadingTheBody"/> can
    /// assert that <see cref="FrameReader.ReadAsync"/> only ever consumed
    /// the 4-byte header before throwing.
    /// </summary>
    private sealed class CountingReadStream(byte[] data) : Stream
    {
        private readonly MemoryStream _inner = new(data);

        public int TotalBytesRead { get; private set; }

        public override bool CanRead => true;

        public override bool CanSeek => false;

        public override bool CanWrite => false;

        public override long Length => throw new NotSupportedException();

        public override long Position
        {
            get => throw new NotSupportedException();
            set => throw new NotSupportedException();
        }

        public override async ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken = default)
        {
            var read = await _inner.ReadAsync(buffer, cancellationToken).ConfigureAwait(false);
            TotalBytesRead += read;
            return read;
        }

        public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException();

        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();

        public override void SetLength(long value) => throw new NotSupportedException();

        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();

        public override void Flush()
        {
        }
    }
}
