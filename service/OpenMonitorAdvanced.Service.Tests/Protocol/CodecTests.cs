using System.Buffers;
using System.Buffers.Binary;
using System.Text;
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
    private const string KeyA = "589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d83";
    private const string KeyB = "3ed905bde72420026a8d0268d0faa314158c7f6d24c853abd4cc97cf55904ea6";

    public static readonly TheoryData<string> FixtureNames =
    [
        "hello", "subscribe", "schema", "snapshot", "snapshot_empty", "error",
        "frames_configure", "frames_target", "frames_target_none", "frames_status", "presenting_processes", "frame_batch",
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
            case "error":
            case "frames_configure":
            case "frames_target":
            case "frames_target_none":
            case "frames_status":
                Assert.Equal(Reference(name), decoded);
                break;
            case "subscribe":
                var expectedSubscribe = (SubscribeMessage)Reference(name);
                var actualSubscribe = Assert.IsType<SubscribeMessage>(decoded);
                Assert.Equal(expectedSubscribe.IntervalMs, actualSubscribe.IntervalMs);
                Assert.Equal(expectedSubscribe.DisabledModules, actualSubscribe.DisabledModules);
                Assert.Equal(expectedSubscribe.SmartDisabledDrives, actualSubscribe.SmartDisabledDrives);
                Assert.Equal(expectedSubscribe.SmartEnabledDrives, actualSubscribe.SmartEnabledDrives);
                break;
            case "snapshot":
            case "snapshot_empty":
                var expectedSnapshot = (SnapshotMessage)Reference(name);
                var actualSnapshot = Assert.IsType<SnapshotMessage>(decoded);
                Assert.Equal(expectedSnapshot.Seq, actualSnapshot.Seq);
                Assert.Equal(expectedSnapshot.TimestampMs, actualSnapshot.TimestampMs);
                Assert.Equal(expectedSnapshot.Values, actualSnapshot.Values);
                Assert.Equal(expectedSnapshot.Held, actualSnapshot.Held);
                break;
            case "presenting_processes":
                var expectedList = (PresentingProcessesMessage)Reference(name);
                var actualList = Assert.IsType<PresentingProcessesMessage>(decoded);
                Assert.Equal(expectedList.AtQpc, actualList.AtQpc);
                Assert.Equal(expectedList.Processes, actualList.Processes);
                break;
            case "frame_batch":
                var expectedBatch = (FrameBatchMessage)Reference(name);
                var actualBatch = Assert.IsType<FrameBatchMessage>(decoded);
                Assert.Equal(expectedBatch.Pid, actualBatch.Pid);
                Assert.Equal(expectedBatch.Dropped, actualBatch.Dropped);
                Assert.Equal(expectedBatch.Frames, actualBatch.Frames);
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

                Assert.Equal(expectedSchema.Service.ActiveModules, actualSchema.Service.ActiveModules);
                Assert.Equal(expectedSchema.Service.SmartDisabledDrives, actualSchema.Service.SmartDisabledDrives);
                Assert.Equal(expectedSchema.Service.Reconfiguration, actualSchema.Service.Reconfiguration);
                Assert.Equal(expectedSchema.Service.Drives, actualSchema.Service.Drives);
                break;
        }
    }

    [Fact]
    public void NonFiniteValuesAreWrittenAsNil()
    {
        var message = new SnapshotMessage(1, 0, new double?[] { double.NaN, double.PositiveInfinity }, [false, false]);
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
    public void UnknownModuleIsABadRequest()
    {
        var bytes = SubscribeBody(["memory", "gpu"], []);
        var e = Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.Contains("gpu", e.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void AHugeUnknownTypeIsClipped()
    {
        string huge = new('x', 1 << 20);
        var bytes = BuildEnvelope(huge, (ref MessagePackWriter w) => w.WriteMapHeader(0));
        var e = Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.True(e.Message.Length <= 128, $"message is {e.Message.Length} characters");

        var frame = MessageCodec.EncodeFrame(new ErrorMessage("bad_request", e.Message));
        Assert.True(frame.Length < 1024);
    }

    [Fact]
    public void EveryKnownModuleIsAccepted()
    {
        var bytes = SubscribeBody(["cpu", "motherboard", "memory", "storage", "controller", "psu"], []);
        var decoded = Assert.IsType<SubscribeMessage>(MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.Equal(6, decoded.DisabledModules.Count);
    }

    [Fact]
    public void TooManyDriveKeysIsABadRequest()
    {
        var atLimit = Enumerable.Range(0, ProtocolConstants.MaxDriveKeys).Select(KeyFor).ToArray();
        var accepted = Assert.IsType<SubscribeMessage>(
            MessageCodec.DecodePayload(new ReadOnlySequence<byte>(SubscribeBody([], atLimit))));
        Assert.Equal(ProtocolConstants.MaxDriveKeys, accepted.SmartDisabledDrives.Count);

        var tooMany = Enumerable.Range(0, ProtocolConstants.MaxDriveKeys + 1).Select(KeyFor).ToArray();
        Assert.Throws<ProtocolException>(
            () => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(SubscribeBody([], tooMany))));
    }

    [Theory]
    [InlineData("")]
    [InlineData("abc")]
    [InlineData("589488FB5895D8B81B82760DC67568E8C99B40A81FAFE4240BD45DD1EE614D83")] // uppercase
    [InlineData("589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d8")] // 63 characters
    [InlineData("589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d833")] // 65 characters
    [InlineData("g89488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d83")] // not hex
    public void MalformedDriveKeyIsABadRequest(string key)
    {
        var bytes = SubscribeBody([], [key]);
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void SubscribeWithoutTheV2ListsIsRejected()
    {
        var bytes = BuildEnvelope("subscribe", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(1);
            w.Write("interval_ms");
            w.Write(500u);
        });
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void SubscribeWithoutTheV3ListIsRejected()
    {
        var bytes = BuildEnvelope("subscribe", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(3);
            w.Write("interval_ms");
            w.Write(500u);
            w.Write("disabled_modules");
            w.WriteArrayHeader(0);
            w.Write("smart_disabled_drives");
            w.WriteArrayHeader(0);
        });
        var e = Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.Contains("smart_enabled_drives", e.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void AKeyInBothListsIsABadRequest()
    {
        var bytes = SubscribeBody([], [KeyA, KeyB], [KeyB]);
        var e = Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.Equal("a drive key cannot be both enabled and disabled", e.Message);
    }

    [Fact]
    public void TooManyEnabledDriveKeysIsABadRequest()
    {
        var atLimit = Enumerable.Range(0, ProtocolConstants.MaxDriveKeys).Select(KeyFor).ToArray();
        var accepted = Assert.IsType<SubscribeMessage>(
            MessageCodec.DecodePayload(new ReadOnlySequence<byte>(SubscribeBody([], [], atLimit))));
        Assert.Equal(ProtocolConstants.MaxDriveKeys, accepted.SmartEnabledDrives.Count);

        var tooMany = Enumerable.Range(0, ProtocolConstants.MaxDriveKeys + 1).Select(KeyFor).ToArray();
        Assert.Throws<ProtocolException>(
            () => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(SubscribeBody([], [], tooMany))));
    }

    [Theory]
    [InlineData("")]
    [InlineData("abc")]
    [InlineData("589488FB5895D8B81B82760DC67568E8C99B40A81FAFE4240BD45DD1EE614D83")] // uppercase
    [InlineData("589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d8")] // 63 characters
    [InlineData("589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d833")] // 65 characters
    [InlineData("g89488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d83")] // not hex
    public void MalformedEnabledDriveKeyIsABadRequest(string key)
    {
        var bytes = SubscribeBody([], [], [key]);
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void ASnapshotWhoseHeldLengthDiffersIsRejected()
    {
        foreach (var (values, held) in new (double?[], bool[])[]
        {
            ([1.0, 2.0], [false]),
            ([1.0], [false, false]),
            ([], [false]),
            ([1.0], []),
        })
        {
            var bytes = SnapshotBody(values, held);
            Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
            Assert.Throws<ProtocolException>(() => MessageCodec.EncodePayload(new SnapshotMessage(1, 0, values, held)));
        }
    }

    [Fact]
    public void HeldWithoutAValueIsRejected()
    {
        var bytes = SnapshotBody([1.0, null], [false, true]);
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));

        // A held flag on a present value, and false on a nil, are fine.
        var ok = Assert.IsType<SnapshotMessage>(
            MessageCodec.DecodePayload(new ReadOnlySequence<byte>(SnapshotBody([1.0, null], [true, false]))));
        Assert.Equal([true, false], ok.Held);
    }

    [Fact]
    public void ANonFiniteValueLosesItsHeldFlag()
    {
        var bytes = BuildEnvelope("snapshot", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(4);
            w.Write("seq");
            w.Write(1u);
            w.Write("timestamp_ms");
            w.Write(0u);
            w.Write("values");
            w.WriteArrayHeader(2);
            w.WriteRaw(RawFloat64(double.NaN));
            w.WriteRaw(RawFloat64(2.0));
            w.Write("held");
            w.WriteArrayHeader(2);
            w.Write(true);
            w.Write(true);
        });

        var decoded = Assert.IsType<SnapshotMessage>(MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.Equal(new double?[] { null, 2.0 }, decoded.Values);
        Assert.Equal([false, true], decoded.Held);
    }

    [Fact]
    public void ASnapshotWithoutHeldIsRejected()
    {
        var bytes = BuildEnvelope("snapshot", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(3);
            w.Write("seq");
            w.Write(1u);
            w.Write("timestamp_ms");
            w.Write(0u);
            w.Write("values");
            w.WriteArrayHeader(0);
        });
        var e = Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.Contains("held", e.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void AServiceBlockWithoutDrivesIsRejected()
    {
        var bytes = SchemaBody(writeDrives: false);
        var e = Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.Contains("drives", e.Message, StringComparison.Ordinal);

        // The same block with the list decodes, so the rejection is about the missing key alone.
        var ok = Assert.IsType<SchemaMessage>(
            MessageCodec.DecodePayload(new ReadOnlySequence<byte>(SchemaBody(writeDrives: true))));
        Assert.Empty(ok.Service.Drives);
    }

    [Theory]
    [InlineData(2u)]
    [InlineData(3u)]
    public void AHelloOfEitherVersionDecodes(uint version)
    {
        // The app must read a v2 service's Hello to report it as incompatible, and vice versa.
        var payload = MessageCodec.EncodePayload(new HelloMessage(version, "0.1.0", "ok"));
        var decoded = Assert.IsType<HelloMessage>(MessageCodec.DecodePayload(new ReadOnlySequence<byte>(payload)));
        Assert.Equal(version, decoded.ProtocolVersion);
    }

    [Fact]
    public void ProtocolVersionIsFour()
    {
        Assert.Equal(4u, ProtocolConstants.Version);
        Assert.Equal(512, ProtocolConstants.MaxFramesPerBatch);
        Assert.Equal(32, ProtocolConstants.MaxPresentingProcesses);
    }

    private static WireFrame AFrame(double msBetweenPresents = 16.6, double? optional = 1.0) =>
        new(1, 2, "app", true, msBetweenPresents, optional, optional, optional, optional, optional, null);

    [Fact]
    public void ABatchOfFiveHundredTwelveFramesRoundTrips()
    {
        var frames = Enumerable.Range(0, 512).Select(_ => AFrame()).ToList();
        var bytes = MessageCodec.EncodePayload(new FrameBatchMessage(1, frames, 0));
        var decoded = Assert.IsType<FrameBatchMessage>(MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.Equal(512, decoded.Frames.Count);
    }

    [Fact]
    public void ABatchOverFiveHundredTwelveFramesIsRejected()
    {
        var frames = Enumerable.Range(0, 513).Select(_ => AFrame()).ToList();
        var bytes = MessageCodec.EncodePayload(new FrameBatchMessage(1, frames, 0));
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void ThirtyThreeProcessesAreRejected()
    {
        var list = Enumerable.Range(0, 33).Select(i => new PresentingProcess((uint)i, "a.exe", 60.0, "m", 1)).ToList();
        var ok = MessageCodec.EncodePayload(new PresentingProcessesMessage(1, list.Take(32).ToList()));
        Assert.Equal(32, Assert.IsType<PresentingProcessesMessage>(MessageCodec.DecodePayload(new ReadOnlySequence<byte>(ok))).Processes.Count);
        var bad = MessageCodec.EncodePayload(new PresentingProcessesMessage(1, list));
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bad)));
    }

    [Fact]
    public void ANonFiniteRequiredFrameTimeIsRejected()
    {
        var bytes = MessageCodec.EncodePayload(new FrameBatchMessage(1, [AFrame(double.NaN)], 0));
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void ANonFiniteDisplayedFpsIsRejected()
    {
        var bytes = MessageCodec.EncodePayload(
            new PresentingProcessesMessage(1, [new PresentingProcess(1, "a.exe", double.PositiveInfinity, "m", 1)]));
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void ANonFiniteOptionalFrameTimeBecomesNil()
    {
        var bytes = MessageCodec.EncodePayload(new FrameBatchMessage(1, [AFrame(16.6, double.NaN)], 0));
        var decoded = Assert.IsType<FrameBatchMessage>(MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        var frame = Assert.Single(decoded.Frames);
        Assert.Null(frame.MsGpuBusy);
        Assert.Null(frame.MsPcLatency);
    }

    [Fact]
    public void AFramesTargetWithoutAPidIsRejected()
    {
        var bytes = BuildEnvelope("frames_target", (ref MessagePackWriter w) => w.WriteMapHeader(0));
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    private static byte[] SnapshotBody(double?[] values, bool[] held) =>
        BuildEnvelope("snapshot", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(4);
            w.Write("seq");
            w.Write(1u);
            w.Write("timestamp_ms");
            w.Write(0u);
            w.Write("values");
            w.WriteArrayHeader(values.Length);
            foreach (var v in values)
            {
                if (v is { } d)
                {
                    w.Write(d);
                }
                else
                {
                    w.WriteNil();
                }
            }

            w.Write("held");
            w.WriteArrayHeader(held.Length);
            foreach (var h in held)
            {
                w.Write(h);
            }
        });

    private static byte[] SchemaBody(bool writeDrives) =>
        BuildEnvelope("schema", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(3);
            w.Write("devices");
            w.WriteArrayHeader(0);
            w.Write("sensors");
            w.WriteArrayHeader(0);
            w.Write("service");
            w.WriteMapHeader(writeDrives ? 4 : 3);
            w.Write("active_modules");
            w.WriteArrayHeader(0);
            w.Write("smart_disabled_drives");
            w.WriteArrayHeader(0);
            w.Write("reconfiguration");
            w.Write("applied");
            if (writeDrives)
            {
                w.Write("drives");
                w.WriteArrayHeader(0);
            }
        });

    private static string KeyFor(int i) => i.ToString("x64", System.Globalization.CultureInfo.InvariantCulture);

    private static byte[] SubscribeBody(string[] modules, string[] drives, string[]? enabled = null) =>
        BuildEnvelope("subscribe", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(4);
            w.Write("interval_ms");
            w.Write(1000u);
            w.Write("disabled_modules");
            w.WriteArrayHeader(modules.Length);
            foreach (var m in modules)
            {
                w.Write(m);
            }

            w.Write("smart_disabled_drives");
            w.WriteArrayHeader(drives.Length);
            foreach (var d in drives)
            {
                w.Write(d);
            }

            w.Write("smart_enabled_drives");
            w.WriteArrayHeader((enabled ?? []).Length);
            foreach (var d in enabled ?? [])
            {
                w.Write(d);
            }
        });

    [Fact]
    public void ExtraFieldsAreSkipped()
    {
        var bytes = BuildEnvelope("subscribe", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(5);
            w.Write("interval_ms");
            w.Write(500u);
            w.Write("disabled_modules");
            w.WriteArrayHeader(0);
            w.Write("smart_disabled_drives");
            w.WriteArrayHeader(0);
            w.Write("smart_enabled_drives");
            w.WriteArrayHeader(0);
            w.Write("extra");
            w.Write(1);
        });

        var decoded = Assert.IsType<SubscribeMessage>(MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
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
        var payload = MessageCodec.EncodePayload(new SubscribeMessage(500, [], [], []));
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
        var frame = MessageCodec.EncodeFrame(new SubscribeMessage(500, [], [], []));
        var truncated = frame[..^2];
        var stream = new MemoryStream(truncated);

        await Assert.ThrowsAsync<ProtocolException>(async () => await FrameReader.ReadAsync(stream, CancellationToken.None));
    }

    // ---- Fix round 1 (review of ca7c4a7) ------------------------------

    [Fact]
    public void NestedMapHeadersDoNotPreallocateFromDeclaredCounts()
    {
        // 65 nested map32 headers, each declaring 100 000 entries, chained
        // through the "key" position of the outer map: the scanner never
        // gets past reading the 65th header (depth 65 > 64), so only 325
        // bytes are ever read. Before the fix, ValidateMap allocated a
        // HashSet<string> sized for the *declared* count (100 000) before
        // any entry was confirmed to exist, so up to 65 such HashSets were
        // simultaneously alive on the call stack — tens of MB, for a 325
        // byte input.
        var bytes = new byte[65 * 5];
        for (var i = 0; i < 65; i++)
        {
            bytes[i * 5] = 0xdf; // map32
            BinaryPrimitives.WriteUInt32BigEndian(bytes.AsSpan(i * 5 + 1), 100_000);
        }

        var before = GC.GetAllocatedBytesForCurrentThread();
        var ex = Record.Exception(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        var allocated = GC.GetAllocatedBytesForCurrentThread() - before;

        Assert.IsType<ProtocolException>(ex);
        Assert.True(
            allocated < 1_000_000,
            $"expected well under 1 MB allocated while rejecting this 325-byte payload, allocated {allocated} bytes");
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void DuplicatePropertyKeyWithDifferentStringMarkersIsRejected(bool useStr16ForSecondEncoding)
    {
        var bytes = BuildSchemaWithDuplicatePropertyKey(useStr16ForSecondEncoding);
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void UnknownFieldWithOversizedArrayIsRejected()
    {
        var bytes = BuildEnvelope("subscribe", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(2);
            w.Write("interval_ms");
            w.Write(500u);
            w.Write("extra");
            w.WriteArrayHeader(100_001);
        });

        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void UnknownFieldWithExcessiveNestingIsRejected()
    {
        var bytes = BuildEnvelope("subscribe", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(2);
            w.Write("interval_ms");
            w.Write(500u);
            w.Write("extra");
            for (var i = 0; i < 65; i++)
            {
                w.WriteArrayHeader(1);
            }

            w.WriteNil();
        });

        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Theory]
    [InlineData(1)]
    [InlineData(2)]
    [InlineData(3)]
    public async Task TruncatedHeaderAtEofIsAnError(int headerBytesAvailable)
    {
        var stream = new MemoryStream(new byte[headerBytesAvailable]);
        await Assert.ThrowsAsync<ProtocolException>(async () => await FrameReader.ReadAsync(stream, CancellationToken.None));
    }

    [Fact]
    public void ValidatorAcceptsAnArrayHeaderDeclaringExactlyMaxElements()
    {
        var bytes = new byte[5 + 100_000];
        bytes[0] = 0xdd; // array32
        BinaryPrimitives.WriteUInt32BigEndian(bytes.AsSpan(1), 100_000);
        // 100 000 nil elements: bytes[5..] are already 0x00, which is not
        // nil (0xc0); fill them explicitly.
        Array.Fill(bytes, (byte)0xc0, 5, 100_000);

        var ex = Record.Exception(() => MessageStructureValidator.Validate(new ReadOnlySequence<byte>(bytes)));
        Assert.Null(ex);
    }

    [Fact]
    public void ValidatorAcceptsAMapHeaderDeclaringExactlyMaxElements()
    {
        // No entries follow: the count check itself must accept exactly the
        // limit, so the failure that does occur must come from running out
        // of bytes while reading the first entry, not from the element
        // count limit (mirrors crates/oma-ipc's
        // map_with_exactly_100000_entries_header_passes_the_count_check).
        var bytes = new byte[5];
        bytes[0] = 0xdf; // map32
        BinaryPrimitives.WriteUInt32BigEndian(bytes.AsSpan(1), 100_000);

        var ex = Assert.Throws<ProtocolException>(() => MessageStructureValidator.Validate(new ReadOnlySequence<byte>(bytes)));
        Assert.DoesNotContain("exceeding", ex.Message);
    }

    [Fact]
    public void ValidatorAcceptsNestingOfExactly64()
    {
        var bytes = new byte[65];
        for (var i = 0; i < 64; i++)
        {
            bytes[i] = 0x91; // fixarray, 1 element
        }

        bytes[64] = 0xc0; // innermost nil

        var ex = Record.Exception(() => MessageStructureValidator.Validate(new ReadOnlySequence<byte>(bytes)));
        Assert.Null(ex);
    }

    [Fact]
    public void RawWireNaNAndPositiveInfinityDecodeAsNull()
    {
        var bytes = BuildEnvelope("snapshot", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(4);
            w.Write("seq");
            w.Write(1u);
            w.Write("timestamp_ms");
            w.Write(0u);
            w.Write("values");
            w.WriteArrayHeader(2);
            w.WriteRaw(RawFloat64(double.NaN));
            w.WriteRaw(RawFloat64(double.PositiveInfinity));
            w.Write("held");
            w.WriteArrayHeader(2);
            w.Write(false);
            w.Write(false);
        });

        var decoded = Assert.IsType<SnapshotMessage>(MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
        Assert.Equal(new double?[] { null, null }, decoded.Values);
    }

    [Fact]
    public void InvalidUtf8InAStringFieldIsRejected()
    {
        var bytes = BuildEnvelope("error", (ref MessagePackWriter w) =>
        {
            w.WriteMapHeader(2);
            w.Write("code");
            w.Write("bad_request");
            w.Write("message");
            // str8, 1 byte of content: 0xff is never a valid UTF-8 leading byte.
            w.WriteRaw(new byte[] { 0xd9, 0x01, 0xff });
        });

        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(bytes)));
    }

    [Fact]
    public void InvalidUtf8InAMapKeyIsRejected()
    {
        var buffer = new ArrayBufferWriter<byte>();
        var writer = new MessagePackWriter(buffer);
        writer.WriteMapHeader(2);
        writer.WriteRaw(new byte[] { 0xd9, 0x01, 0xff }); // invalid UTF-8 "type" key
        writer.Write("subscribe");
        writer.Write("body");
        writer.WriteMapHeader(1);
        writer.Write("interval_ms");
        writer.Write(500u);
        writer.Flush();

        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(buffer.WrittenSpan.ToArray())));
    }

    [Fact]
    public void MemoryHintValueMustBeAMap()
    {
        var nilValue = BuildDeviceWithMemoryHint((ref MessagePackWriter w) => w.WriteNil());
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(nilValue)));

        var arrayValue = BuildDeviceWithMemoryHint((ref MessagePackWriter w) => w.WriteArrayHeader(0));
        Assert.Throws<ProtocolException>(() => MessageCodec.DecodePayload(new ReadOnlySequence<byte>(arrayValue)));
    }

    [Fact]
    public void PropertiesAreSortedByUtf8ByteOrderNotUtf16CodeUnits()
    {
        // "｡" (U+FF61) is EF BD A1 in UTF-8; "😀" (U+1F600) is F0 9F 98 80.
        // Byte-lexicographic order puts "｡" first (0xEF < 0xF0). UTF-16
        // ordinal order (comparing code units — the emoji is a surrogate
        // pair starting with D83D) would put the emoji first instead, since
        // 0xD83D < 0xFF61.
        var device = new WireDevice(
            "d", "cpu", "n", null,
            new Dictionary<string, string> { ["😀"] = "emoji", ["｡"] = "halfwidth" },
            null);
        var schema = new SchemaMessage([device], [], ServiceStateBlock.AllActive);
        var payload = MessageCodec.EncodePayload(schema);

        var halfwidthIndex = IndexOfSubsequence(payload, Encoding.UTF8.GetBytes("｡"));
        var emojiIndex = IndexOfSubsequence(payload, Encoding.UTF8.GetBytes("😀"));

        Assert.True(halfwidthIndex >= 0, "expected to find the halfwidth-period key's UTF-8 bytes in the payload");
        Assert.True(emojiIndex >= 0, "expected to find the emoji key's UTF-8 bytes in the payload");
        Assert.True(
            halfwidthIndex < emojiIndex,
            "expected \"｡\" (UTF-8 EF BD A1) to sort before \"😀\" (UTF-8 F0 9F 98 80)");
    }

    private static byte[] RawFloat64(double value)
    {
        var bytes = new byte[9];
        bytes[0] = 0xcb;
        BinaryPrimitives.WriteUInt64BigEndian(bytes.AsSpan(1), BitConverter.DoubleToUInt64Bits(value));
        return bytes;
    }

    private static int IndexOfSubsequence(byte[] haystack, byte[] needle)
    {
        for (var i = 0; i + needle.Length <= haystack.Length; i++)
        {
            if (haystack.AsSpan(i, needle.Length).SequenceEqual(needle))
            {
                return i;
            }
        }

        return -1;
    }

    private static void WriteStr8(ref MessagePackWriter w, string s)
    {
        var utf8 = Encoding.UTF8.GetBytes(s);
        w.WriteRaw(new byte[] { 0xd9, (byte)utf8.Length });
        w.WriteRaw(utf8);
    }

    private static void WriteStr16(ref MessagePackWriter w, string s)
    {
        var utf8 = Encoding.UTF8.GetBytes(s);
        var header = new byte[3];
        header[0] = 0xda;
        BinaryPrimitives.WriteUInt16BigEndian(header.AsSpan(1), (ushort)utf8.Length);
        w.WriteRaw(header);
        w.WriteRaw(utf8);
    }

    private static byte[] BuildSchemaWithDuplicatePropertyKey(bool useStr16ForSecondEncoding)
    {
        var buffer = new ArrayBufferWriter<byte>();
        var w = new MessagePackWriter(buffer);
        w.WriteMapHeader(2);
        w.Write("type");
        w.Write("schema");
        w.Write("body");
        w.WriteMapHeader(2);
        w.Write("devices");
        w.WriteArrayHeader(1);
        w.WriteMapHeader(6);
        w.Write("id");
        w.Write("d");
        w.Write("kind");
        w.Write("cpu");
        w.Write("name");
        w.Write("n");
        w.Write("vendor");
        w.WriteNil();
        w.Write("properties");
        w.WriteMapHeader(2); // duplicate key, 2 entries
        w.Write("firmware"); // fixstr
        w.Write("A");
        if (useStr16ForSecondEncoding)
        {
            WriteStr16(ref w, "firmware");
        }
        else
        {
            WriteStr8(ref w, "firmware");
        }

        w.Write("B");
        w.Write("hint");
        w.WriteNil();
        w.Write("sensors");
        w.WriteArrayHeader(0);
        w.Flush();
        return buffer.WrittenSpan.ToArray();
    }

    private static byte[] BuildDeviceWithMemoryHint(WriteBody writeHintValue)
    {
        var buffer = new ArrayBufferWriter<byte>();
        var w = new MessagePackWriter(buffer);
        w.WriteMapHeader(2);
        w.Write("type");
        w.Write("schema");
        w.Write("body");
        w.WriteMapHeader(2);
        w.Write("devices");
        w.WriteArrayHeader(1);
        w.WriteMapHeader(6);
        w.Write("id");
        w.Write("d");
        w.Write("kind");
        w.Write("memory");
        w.Write("name");
        w.Write("n");
        w.Write("vendor");
        w.WriteNil();
        w.Write("properties");
        w.WriteMapHeader(0);
        w.Write("hint");
        w.WriteMapHeader(2);
        w.Write("kind");
        w.Write("memory");
        w.Write("value");
        writeHintValue(ref w);
        w.Write("sensors");
        w.WriteArrayHeader(0);
        w.Flush();
        return buffer.WrittenSpan.ToArray();
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
        "hello" => new HelloMessage(4, "0.1.0", "rebootPending"),
        "subscribe" => new SubscribeMessage(1000, ["memory", "psu"], [KeyA], [KeyB]),
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
            ],
            new ServiceStateBlock(
                ["cpu", "motherboard", "storage", "controller"],
                [KeyA],
                "pending",
                [
                    new WireDrive(0, KeyA, "Samsung SSD 990 PRO 2TB", "smartOff", false),
                    new WireDrive(1, null, "ST2000DM008-2UB102", "standby", true),
                ])),
        "snapshot" => new SnapshotMessage(
            4_294_967_301UL, 1_790_000_000_000UL,
            new double?[] { 45.0, null, -12.5, 0.0 },
            [false, false, true, false]),
        "snapshot_empty" => new SnapshotMessage(1, 0, Array.Empty<double?>(), []),
        "error" => new ErrorMessage("bad_request", "Messaggio non valido: è atteso Subscribe"),
        "frames_configure" => new FramesConfigureMessage(true, true, false),
        "frames_target" => new FramesTargetMessage(25848),
        "frames_target_none" => new FramesTargetMessage(null),
        "frames_status" => new FramesStatusMessage(FramesStates.Running, null, "2.6.0"),
        "presenting_processes" => new PresentingProcessesMessage(
            380_058_775_270UL,
            [
                new PresentingProcess(25848, "CONTROLResonant.exe", 61.5, "Hardware Composed: Independent Flip", 1),
                new PresentingProcess(1852, "dwm.exe", 20.0, "Hardware: Legacy Flip", 1),
            ]),
        "frame_batch" => new FrameBatchMessage(
            25848,
            [
                new WireFrame(
                    369_166_005_856UL, 0x022A_3569_E270UL, "app", true,
                    17.1266, 7.1667, 11.7554, 17.1706, 35.5189, 16.1228, 43715UL),
                new WireFrame(
                    369_166_008_179UL, 0x022A_3569_E270UL, "app", true,
                    0.2323, 10.4468, 21.9699, 0.1817, 45.9657, 0.2476, null),
            ],
            3),
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
