using System.Buffers.Binary;
using System.Diagnostics;
using System.IO.Pipes;
using System.Security.AccessControl;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Pipe;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using OpenMonitorAdvanced.Service.Tests.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Pipe;

public sealed class PipeListenerTests
{
    /// <summary>Enough values that one snapshot (about 180 KB) overflows the pipe's 64 KB buffer.</summary>
    private const int LargeSensorCount = 20_000;

    private static CancellationToken Ct => TestContext.Current.CancellationToken;

    [Fact]
    public async Task ClientReceivesHelloThenSchemaAndSnapshot()
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        using var client = await TestClient.ConnectAsync(h.PipeName, Ct);

        var hello = await client.ReadAsync<HelloMessage>(Ct);
        Assert.Equal(4u, hello.ProtocolVersion);
        Assert.Equal("ok", hello.PawnIo);
        Assert.False(string.IsNullOrWhiteSpace(hello.ServiceVersion));

        await client.SendAsync(new SubscribeMessage(1000, [], [], []), Ct);
        await PipeAssert.EventuallyAsync(() => h.Feed.Active.Count == 1, "the subscription", Ct);
        Assert.Equal([1000u], h.Feed.Intervals);

        FeedUpdate first = MakeUpdate(3, seq: 1, withSchema: true);
        h.Feed.Push(first);
        PipeAssert.SameMessage(first.Schema!, await client.ReadAsync<SchemaMessage>(Ct));
        PipeAssert.SameMessage(first.Snapshot, await client.ReadAsync<SnapshotMessage>(Ct));

        FeedUpdate second = MakeUpdate(3, seq: 2, withSchema: false);
        h.Feed.Push(second);
        PipeAssert.SameMessage(second.Snapshot, await client.ReadAsync<SnapshotMessage>(Ct));
    }

    [Theory]
    [InlineData(PawnIoStatus.Ok, "ok")]
    [InlineData(PawnIoStatus.Missing, "missing")]
    [InlineData(PawnIoStatus.Unavailable, "unavailable")]
    [InlineData(PawnIoStatus.Unknown, "unknown")]
    [InlineData(PawnIoStatus.RebootPending, "rebootPending")]
    public async Task HelloCarriesThePawnIoStatus(PawnIoStatus status, string wire)
    {
        await using var h = await ListenerHarness.StartAsync(Ct, pawnIo: status);
        using var client = await TestClient.ConnectAsync(h.PipeName, Ct);

        var hello = await client.ReadAsync<HelloMessage>(Ct);

        Assert.Equal(wire, hello.PawnIo);
    }

    [Theory]
    [InlineData("unknown module")]
    [InlineData("too many keys")]
    [InlineData("too many enabled keys")]
    [InlineData("key in both lists")]
    [InlineData("malformed key")]
    public async Task ABadSubscribeGetsAnErrorAndThePipeCloses(string problem)
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        using var client = await TestClient.ConnectAsync(h.PipeName, Ct);
        await client.ReadAsync<HelloMessage>(Ct);

        const string DriveA = "589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d83";
        SubscribeMessage subscribe = problem switch
        {
            "unknown module" => new SubscribeMessage(1000, ["gpu"], [], []),
            "too many keys" => new SubscribeMessage(
                1000,
                [],
                Enumerable.Range(0, ProtocolConstants.MaxDriveKeys + 1).Select(i => i.ToString("x64", System.Globalization.CultureInfo.InvariantCulture)).ToArray(),
                []),
            "key in both lists" => new SubscribeMessage(1000, [], [DriveA], [DriveA]),
            "too many enabled keys" => new SubscribeMessage(
                1000,
                [],
                [],
                Enumerable.Range(0, ProtocolConstants.MaxDriveKeys + 1).Select(i => i.ToString("x64", System.Globalization.CultureInfo.InvariantCulture)).ToArray()),
            _ => new SubscribeMessage(1000, [], ["NOT-A-KEY"], []),
        };
        await client.SendAsync(subscribe, Ct);

        var error = await client.ReadAsync<ErrorMessage>(Ct);
        Assert.Equal("bad_request", error.Code);
        Assert.Null(await client.ReadAsync(Ct));
        Assert.Empty(h.Feed.All);
    }

    [Theory]
    [InlineData(0u, 250u)]
    [InlineData(10u, 250u)]
    [InlineData(250u, 250u)]
    [InlineData(1000u, 1000u)]
    [InlineData(5000u, 5000u)]
    [InlineData(60000u, 5000u)]
    [InlineData(uint.MaxValue, 5000u)]
    public async Task IntervalIsClamped(uint requested, uint expected)
    {
        await using var h = await ListenerHarness.StartAsync(Ct);

        using var client = await h.SubscribedClientAsync(requested, Ct);

        Assert.Equal([expected], h.Feed.Intervals);
    }

    [Fact]
    public async Task ResubscribeUpdatesTheRequestWithoutUnsubscribing()
    {
        const string Drive = "589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d83";
        await using var h = await ListenerHarness.StartAsync(Ct);
        using var client = await h.SubscribedClientAsync(1000, Ct);
        FakeFeed.Subscription subscription = Assert.Single(h.Feed.All);

        await client.SendAsync(new SubscribeMessage(60000, ["storage", "psu"], [Drive], []), Ct);
        await PipeAssert.EventuallyAsync(() => subscription.Requests.Count == 2, "the replaced request", Ct);

        // Replaced in place: no second subscription, and the first one is never disposed.
        Assert.Same(subscription, Assert.Single(h.Feed.All));
        Assert.Equal(0, subscription.DisposeCount);
        FeedRequest replaced = subscription.Requests[1];
        Assert.Equal(ProtocolConstants.MaxIntervalMs, replaced.IntervalMs);
        Assert.Equal(ServiceModules.Storage | ServiceModules.Psu, replaced.Disabled);
        Assert.Equal([Drive], replaced.SmartDisabledDrives);
        Assert.Equal([1000u, 5000u], subscription.Requests.Select(r => r.IntervalMs));

        // The same subscription keeps streaming to the client.
        FeedUpdate current = MakeUpdate(2, seq: 222, withSchema: true);
        h.Feed.Push(current);
        PipeAssert.SameMessage(current.Schema!, await client.ReadAsync<SchemaMessage>(Ct));
        PipeAssert.SameMessage(current.Snapshot, await client.ReadAsync<SnapshotMessage>(Ct));

        client.Dispose();
        await PipeAssert.EventuallyAsync(() => subscription.DisposeCount == 1, "the session to release its subscription", Ct);
    }

    [Fact]
    public async Task TheFirstSubscribeCarriesTheSourceRequest()
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        using var client = await TestClient.ConnectAsync(h.PipeName, Ct);
        await client.ReadAsync<HelloMessage>(Ct);

        await client.SendAsync(new SubscribeMessage(1000, ["cpu"], [], []), Ct);
        await PipeAssert.EventuallyAsync(() => h.Feed.All.Count == 1, "the subscription", Ct);

        FeedRequest request = Assert.Single(h.Feed.All[0].Requests);
        Assert.Equal(ServiceModules.Cpu, request.Disabled);
        Assert.Empty(request.SmartDisabledDrives);
    }

    [Theory]
    [InlineData("snapshot")]
    [InlineData("hello")]
    [InlineData("error")]
    [InlineData("schema")]
    [InlineData("garbage")]
    [InlineData("oversized")]
    public async Task InvalidMessageGetsAnErrorAndThePipeCloses(string kind)
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        using var client = await TestClient.ConnectAsync(h.PipeName, Ct);
        await client.ReadAsync<HelloMessage>(Ct);

        byte[] frame = kind switch
        {
            "snapshot" => MessageCodec.EncodeFrame(new SnapshotMessage(1, 2, [1.0], [false])),
            "hello" => MessageCodec.EncodeFrame(new HelloMessage(3, "client", "ok")),
            "error" => MessageCodec.EncodeFrame(new ErrorMessage("bad_request", "client")),
            "schema" => MessageCodec.EncodeFrame(MakeSchema(1)),
            "garbage" => [3, 0, 0, 0, 0xc1, 0xc1, 0xc1],
            "oversized" => LengthPrefix(ProtocolConstants.MaxFrameBytes + 1),
            _ => throw new ArgumentOutOfRangeException(nameof(kind)),
        };
        await client.SendRawAsync(frame, Ct);

        var error = await client.ReadAsync<ErrorMessage>(Ct);
        Assert.Equal("bad_request", error.Code);
        Assert.Null(await client.ReadAsync(Ct));
        Assert.Empty(h.Feed.All);
    }

    [Fact]
    public async Task MissingSubscribeTimesOut()
    {
        await using var h = await ListenerHarness.StartAsync(Ct, subscribeTimeout: TimeSpan.FromMilliseconds(200));
        using var client = await TestClient.ConnectAsync(h.PipeName, Ct);
        await client.ReadAsync<HelloMessage>(Ct);

        var elapsed = Stopwatch.StartNew();
        var error = await client.ReadAsync<ErrorMessage>(Ct);

        Assert.Equal("bad_request", error.Code);
        Assert.True(elapsed.Elapsed < TimeSpan.FromSeconds(3), $"the timeout took {elapsed.Elapsed}");
        Assert.Null(await client.ReadAsync(Ct));
        Assert.Empty(h.Feed.All);
    }

    [Fact]
    public async Task FramesConfigureAfterSubscribeIsAccepted()
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        using var client = await h.SubscribedClientAsync(1000, Ct);

        await client.SendAsync(new FramesConfigureMessage(Enabled: true, TrackPcLatency: false, TrackGpu: true), Ct);

        // The current status on subscribing, then the capture starting.
        Assert.Equal(FramesStates.Off, (await client.ReadAsync<FramesStatusMessage>(Ct)).State);
        Assert.Equal(FramesStates.Starting, (await client.ReadAsync<FramesStatusMessage>(Ct)).State);
        Assert.NotNull(h.FrameRequests.Effective);

        await client.SendAsync(new FramesTargetMessage(4242), Ct);
        await PipeAssert.EventuallyAsync(() => h.FrameRequests.Targets.Contains(4242u), "the target", Ct);

        // Sensor updates still flow on the same session.
        h.Feed.Push(new FeedUpdate(null, new SnapshotMessage(1, 2, [1.0], [false])));
        Assert.Equal(1UL, (await client.ReadAsync<SnapshotMessage>(Ct)).Seq);

        client.Dispose();
        await PipeAssert.EventuallyAsync(() => h.FrameRequests.NextExpiry is not null, "the disconnect", Ct);
        Assert.NotNull(h.FrameRequests.Effective); // within the 30 s grace
    }

    [Theory]
    [InlineData("configure")]
    [InlineData("target")]
    public async Task FramesConfigureBeforeSubscribeGetsAnError(string kind)
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        using var client = await TestClient.ConnectAsync(h.PipeName, Ct);
        await client.ReadAsync<HelloMessage>(Ct);

        IMessage message = kind == "configure"
            ? new FramesConfigureMessage(Enabled: true, TrackPcLatency: false, TrackGpu: true)
            : new FramesTargetMessage(4242);
        await client.SendAsync(message, Ct);

        Assert.Equal("bad_request", (await client.ReadAsync<ErrorMessage>(Ct)).Code);
        Assert.Null(await client.ReadAsync(Ct));
        Assert.Null(h.FrameRequests.Effective);
        Assert.Empty(h.FrameRequests.Targets);
        Assert.Equal(0, h.FrameSource.Starts.Count);
    }

    [Fact]
    public async Task NinthClientIsRefusedAndOthersKeepStreaming()
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        var clients = new List<TestClient>();
        try
        {
            for (int i = 0; i < 8; i++)
            {
                clients.Add(await h.SubscribedClientAsync(1000, Ct));
            }

            using (var ninth = new NamedPipeClientStream(".", h.PipeName, PipeDirection.InOut, PipeOptions.Asynchronous))
            {
                await Assert.ThrowsAsync<TimeoutException>(() => ninth.ConnectAsync(500, Ct));
            }

            FeedUpdate update = MakeUpdate(2, seq: 1, withSchema: true);
            h.Feed.Push(update);
            foreach (TestClient client in clients)
            {
                PipeAssert.SameMessage(update.Schema!, await client.ReadAsync<SchemaMessage>(Ct));
                PipeAssert.SameMessage(update.Snapshot, await client.ReadAsync<SnapshotMessage>(Ct));
            }

            // A freed slot goes to the next client.
            clients[0].Dispose();
            clients.RemoveAt(0);
            using var next = await TestClient.ConnectAsync(h.PipeName, Ct);
            await next.ReadAsync<HelloMessage>(Ct);
        }
        finally
        {
            clients.ForEach(c => c.Dispose());
        }
    }

    [Fact]
    public async Task ASlotClosedByTheServerIsReusedWhileItsClientStillHoldsTheHandle()
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        var clients = new List<TestClient>();
        try
        {
            for (int i = 0; i < 8; i++)
            {
                clients.Add(await h.SubscribedClientAsync(1000, Ct));
            }

            // The server ends one session; that client reads the EOF but keeps its handle open.
            await clients[0].SendAsync(new HelloMessage(3, "client", "ok"), Ct);
            Assert.Equal("bad_request", (await clients[0].ReadAsync<ErrorMessage>(Ct)).Code);
            Assert.Null(await clients[0].ReadAsync(Ct));

            using var next = await TestClient.ConnectAsync(h.PipeName, Ct, timeoutMs: 5000);
            await next.ReadAsync<HelloMessage>(Ct);
        }
        finally
        {
            clients.ForEach(c => c.Dispose());
        }
    }

    [Fact]
    public async Task EighthClientReceivesHelloWithoutWaitingForAnotherToLeave()
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        var clients = new List<TestClient>();
        try
        {
            for (int i = 0; i < 8; i++)
            {
                TestClient client = await TestClient.ConnectAsync(h.PipeName, Ct, timeoutMs: 2000);
                clients.Add(client);
                await client.ReadAsync<HelloMessage>(Ct, TimeSpan.FromSeconds(2));
            }

            Assert.Equal(8, h.Idle.ClientCount);
        }
        finally
        {
            clients.ForEach(c => c.Dispose());
        }
    }

    [Fact]
    public async Task SlowClientDoesNotBlockOtherSubscribers()
    {
        // A long write timeout: the slow client must be dropped by its full queue, not by the timeout.
        await using var h = await ListenerHarness.StartAsync(Ct, writeTimeout: TimeSpan.FromMinutes(1));
        using var slow = await h.SubscribedClientAsync(1000, Ct); // never reads again
        using var fast = await h.SubscribedClientAsync(1000, Ct);
        FakeFeed.Subscription slowSubscription = h.Feed.All[0];

        for (ulong seq = 1; seq <= 6; seq++)
        {
            var push = Stopwatch.StartNew();
            h.Feed.Push(MakeUpdate(LargeSensorCount, seq, withSchema: seq == 1));
            Assert.True(push.Elapsed < TimeSpan.FromSeconds(1), $"the feed callback blocked for {push.Elapsed}");

            if (seq == 1)
            {
                await fast.ReadAsync<SchemaMessage>(Ct);
            }

            var snapshot = await fast.ReadAsync<SnapshotMessage>(Ct, TimeSpan.FromSeconds(2));
            Assert.Equal(seq, snapshot.Seq);
        }

        await PipeAssert.EventuallyAsync(() => slowSubscription.DisposeCount == 1, "the slow client's session to close", Ct);
        Assert.Single(h.Feed.Active);
    }

    [Fact]
    public async Task AClientThatStopsReadingIsClosedAfterTheWriteTimeout()
    {
        await using var h = await ListenerHarness.StartAsync(Ct, writeTimeout: TimeSpan.FromMilliseconds(300));
        using var stuck = await h.SubscribedClientAsync(1000, Ct);

        // One push only: the queue never fills, so only the write timeout can close the session.
        h.Feed.Push(MakeUpdate(LargeSensorCount, seq: 1, withSchema: true));

        await PipeAssert.EventuallyAsync(() => h.Feed.All[0].DisposeCount == 1, "the stuck session to close", Ct);
        await PipeAssert.EventuallyAsync(() => h.Idle.ClientCount == 0, "the idle count to drop", Ct);
    }

    [Fact]
    public async Task SchemaAndSnapshotCannotInterleaveWithAnotherUpdate()
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        using var client = await h.SubscribedClientAsync(1000, Ct);

        for (int round = 0; round < 50; round++)
        {
            int[] sizes = [1 + (2 * round), 2 + (2 * round)];
            using var barrier = new Barrier(sizes.Length);
            Task[] pushes = sizes
                .Select(n => Task.Run(
                    () =>
                    {
                        barrier.SignalAndWait(Ct);
                        h.Feed.Push(MakeUpdate(n, (ulong)n, withSchema: true));
                    },
                    Ct))
                .ToArray();
            await Task.WhenAll(pushes);

            var seen = new List<int>();
            foreach (int _ in sizes)
            {
                var schema = await client.ReadAsync<SchemaMessage>(Ct);
                var snapshot = await client.ReadAsync<SnapshotMessage>(Ct);
                Assert.Equal(schema.Sensors.Count, snapshot.Values.Count);
                Assert.Equal((ulong)schema.Sensors.Count, snapshot.Seq);
                seen.Add(schema.Sensors.Count);
            }

            Assert.Equal(sizes, seen.Order());
        }
    }

    [Theory]
    [InlineData("subscribe-fails")]
    [InlineData("client-leaves")]
    [InlineData("bad-request")]
    [InlineData("queue-overflow")]
    public async Task SessionFailureReleasesSubscriptionAndIdleCount(string failure)
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        if (failure == "subscribe-fails")
        {
            h.Feed.FailSubscribe = _ => new InvalidOperationException("feed unavailable");
        }

        using (var client = await TestClient.ConnectAsync(h.PipeName, Ct))
        {
            await client.ReadAsync<HelloMessage>(Ct);
            await PipeAssert.EventuallyAsync(() => h.Idle.ClientCount == 1, "the client to be counted", Ct);
            await client.SendAsync(new SubscribeMessage(1000, [], [], []), Ct);

            switch (failure)
            {
                case "subscribe-fails":
                    await PipeAssert.EventuallyAsync(() => h.Idle.ClientCount == 0, "the failed session to end", Ct);
                    break;
                case "client-leaves":
                    await PipeAssert.EventuallyAsync(() => h.Feed.All.Count == 1, "the subscription", Ct);
                    break;
                case "bad-request":
                    await PipeAssert.EventuallyAsync(() => h.Feed.All.Count == 1, "the subscription", Ct);
                    await client.SendAsync(new HelloMessage(3, "client", "ok"), Ct);
                    Assert.Equal("bad_request", (await client.ReadAsync<ErrorMessage>(Ct)).Code);
                    break;
                case "queue-overflow":
                    await PipeAssert.EventuallyAsync(() => h.Feed.All.Count == 1, "the subscription", Ct);
                    for (ulong seq = 1; seq <= 4; seq++)
                    {
                        h.Feed.Push(MakeUpdate(LargeSensorCount, seq, withSchema: seq == 1));
                    }

                    await PipeAssert.EventuallyAsync(() => h.Feed.All[0].DisposeCount > 0, "the overflowing session to end", Ct);
                    break;
            }
        }

        await PipeAssert.EventuallyAsync(() => h.Idle.ClientCount == 0, "the idle count to drop", Ct);
        await Task.Delay(100, Ct); // leave room for a (wrong) second release
        Assert.Equal(0, h.Idle.ClientCount);
        Assert.All(h.Feed.All, s => Assert.Equal(1, s.DisposeCount));
        Assert.Empty(h.Feed.Active);

        // The idle countdown runs again, and the listener still serves new clients.
        h.Time.Advance(TimeSpan.FromMinutes(2));
        Assert.Equal(1, h.Lifetime.StopCount);
        using var next = await TestClient.ConnectAsync(h.PipeName, Ct);
        await next.ReadAsync<HelloMessage>(Ct);
    }

    [Fact]
    public async Task ASecondClientConnectsWhileTheFirstIsBeingServed()
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        using (var first = await h.SubscribedClientAsync(1000, Ct))
        {
            await ConnectAndLeaveRepeatedlyAsync(h.PipeName, 50);

            FeedUpdate update = MakeUpdate(1, seq: 1, withSchema: true);
            h.Feed.Push(update);
            PipeAssert.SameMessage(update.Schema!, await first.ReadAsync<SchemaMessage>(Ct));
            PipeAssert.SameMessage(update.Snapshot, await first.ReadAsync<SnapshotMessage>(Ct));
        }

        // No other client keeps an instance alive now: each quick client that leaves must not
        // take the name with it (spike S3's original bug).
        await PipeAssert.EventuallyAsync(() => h.Idle.ClientCount == 0, "the first client to leave", Ct);
        await ConnectAndLeaveRepeatedlyAsync(h.PipeName, 50);
    }

    private static async Task ConnectAndLeaveRepeatedlyAsync(string pipeName, int times)
    {
        for (int i = 0; i < times; i++)
        {
            // Raw CreateFileW: fails the test on ERROR_FILE_NOT_FOUND instead of retrying.
            using var client = TestClient.RawConnect(pipeName);
            await client.ReadAsync<HelloMessage>(Ct);
        }
    }

    [Fact]
    public async Task TakenNameStopsTheListener()
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        using (var owner = await TestClient.ConnectAsync(h.PipeName, Ct))
        {
            await owner.ReadAsync<HelloMessage>(Ct); // the first listener owns the name
        }

        var log = new ListLogger<PipeListener>();
        var idle = new IdleShutdown(new FakeLifetime(), new FakeTimeProvider(), TimeSpan.FromMinutes(2));
        using var second = new PipeListener(new FakeFeed(), h.Options, idle, new PawnIoState(() => PawnIoStatus.Ok), h.Frames, log);
        await second.StartAsync(Ct);
        try
        {
            Task execute = second.ExecuteTask!;
            await Task.WhenAny(execute, Task.Delay(TimeSpan.FromSeconds(5), Ct));

            Assert.True(execute.IsFaulted, $"the second listener is {execute.Status}");
            Assert.Contains(log.Entries, e => e.Level >= LogLevel.Error && e.Message.Contains(h.PipeName, StringComparison.Ordinal));
        }
        finally
        {
            await second.StopAsync(CancellationToken.None);
        }

        using var client = await TestClient.ConnectAsync(h.PipeName, Ct);
        await client.ReadAsync<HelloMessage>(Ct);
    }

    [Fact]
    public async Task StoppingTheListenerClosesEverySession()
    {
        var h = await ListenerHarness.StartAsync(Ct);
        using var subscribed = await h.SubscribedClientAsync(1000, Ct);
        using var greeted = await TestClient.ConnectAsync(h.PipeName, Ct);
        await greeted.ReadAsync<HelloMessage>(Ct);

        await h.DisposeAsync();

        Assert.Null(await subscribed.ReadAsync(Ct));
        Assert.Null(await greeted.ReadAsync(Ct));
        Assert.All(h.Feed.All, s => Assert.Equal(1, s.DisposeCount));
        Assert.Equal(0, h.Idle.ClientCount);
    }

    [Fact]
    public async Task ProductionDescriptorIsAppliedToThePipe()
    {
        // Unprivileged, the first instance can be created with the production DACL (spike S3);
        // only the second one fails, which the listener logs and retries.
        await using var h = await ListenerHarness.StartAsync(Ct, sddl: new PipeListenerOptions().SecurityDescriptorSddl);
        using var client = await TestClient.ConnectAsync(h.PipeName, Ct);
        await client.ReadAsync<HelloMessage>(Ct);

        string dacl = client.Stream.GetAccessControl().GetSecurityDescriptorSddlForm(AccessControlSections.Access);

        // Protected DACL with exactly these three allow ACEs (their order carries no meaning).
        Assert.StartsWith("D:P(", dacl, StringComparison.Ordinal);
        string[] aces = dacl["D:P".Length..].Split(')', StringSplitOptions.RemoveEmptyEntries);
        Assert.Equal(["(A;;0x12019b;;;IU", "(A;;FA;;;BA", "(A;;FA;;;SY"], aces.Order(StringComparer.Ordinal));
    }

    [Fact]
    public async Task RemoteClientsAreRejected()
    {
        await using var h = await ListenerHarness.StartAsync(Ct);
        using (var local = await TestClient.ConnectAsync(h.PipeName, Ct))
        {
            await local.ReadAsync<HelloMessage>(Ct);
        }

        // \\localhost\pipe\... goes through the SMB redirector: a remote client for the pipe.
        using var remote = new NamedPipeClientStream("localhost", h.PipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        await Assert.ThrowsAsync<UnauthorizedAccessException>(() => remote.ConnectAsync(5000, Ct));
    }

    private static byte[] LengthPrefix(int length)
    {
        var header = new byte[4];
        BinaryPrimitives.WriteUInt32LittleEndian(header, (uint)length);
        return header;
    }

    private static SchemaMessage MakeSchema(int sensors) => new(
        [new WireDevice("cpu/test", "cpu", "Test CPU", null, new Dictionary<string, string>(), new CpuHint(0))],
        Enumerable.Range(0, sensors)
            .Select(i => new WireSensor("cpu/test", "load", $"s{i}", "percent", "lhm.raw", $"sensor {i}", "load"))
            .ToList(),
        ServiceStateBlock.AllActive);

    private static FeedUpdate MakeUpdate(int sensors, ulong seq, bool withSchema) => new(
        withSchema ? MakeSchema(sensors) : null,
        new SnapshotMessage(seq, 1_000 + seq, Enumerable.Range(0, sensors).Select(i => (double?)(i + (seq * 0.5))).ToList(), Enumerable.Repeat(false, sensors).ToList()));
}
