using System.Diagnostics;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using Microsoft.Extensions.Time.Testing;
using Microsoft.Win32.SafeHandles;
using OpenMonitorAdvanced.Service.Pipe;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using OpenMonitorAdvanced.Service.Tests.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Pipe;

/// <summary>
/// Scripted <see cref="ISensorFeed"/>: records every subscription (its requests, dispose count) and
/// lets a test push updates synchronously to the active ones, the way the hub's sampler thread does.
/// Disposable like the real hub, recording how many subscriptions were still active at that point.
/// </summary>
internal sealed class FakeFeed : ISensorFeed, IDisposable
{
    private readonly object _gate = new();
    private readonly List<Subscription> _all = [];
    private int _disposeCount;

    public int DisposeCount => Volatile.Read(ref _disposeCount);

    /// <summary>Active subscriptions when <see cref="Dispose"/> first ran (-1 = never disposed).</summary>
    public int ActiveWhenDisposed { get; private set; } = -1;

    public void Dispose()
    {
        if (Interlocked.Increment(ref _disposeCount) == 1)
        {
            ActiveWhenDisposed = Active.Count;
        }
    }

    /// <summary>When set, <see cref="Subscribe"/> throws the returned exception.</summary>
    public Func<uint, Exception>? FailSubscribe { get; set; }

    public IReadOnlyList<Subscription> All
    {
        get
        {
            lock (_gate)
            {
                return _all.ToArray();
            }
        }
    }

    public IReadOnlyList<Subscription> Active => All.Where(s => s.DisposeCount == 0).ToArray();

    /// <summary>The current interval of every subscription, in subscription order.</summary>
    public IReadOnlyList<uint> Intervals => All.Select(s => s.IntervalMs).ToArray();

    public IFeedSubscription Subscribe(FeedRequest request, Action<FeedUpdate> onUpdate)
    {
        if (FailSubscribe is { } fail)
        {
            throw fail(request.IntervalMs);
        }

        var subscription = new Subscription(request, onUpdate);
        lock (_gate)
        {
            _all.Add(subscription);
        }

        return subscription;
    }

    /// <summary>Delivers <paramref name="update"/> to every active subscription on the calling thread.</summary>
    public void Push(FeedUpdate update)
    {
        foreach (Subscription subscription in Active)
        {
            subscription.OnUpdate(update);
        }
    }

    /// <summary>One subscription with every request it received (the first from Subscribe, then each Update).</summary>
    internal sealed class Subscription(FeedRequest request, Action<FeedUpdate> onUpdate) : IFeedSubscription
    {
        private readonly object _gate = new();
        private readonly List<FeedRequest> _requests = [request];
        private int _disposeCount;

        public IReadOnlyList<FeedRequest> Requests
        {
            get
            {
                lock (_gate)
                {
                    return _requests.ToArray();
                }
            }
        }

        public uint IntervalMs => Requests[^1].IntervalMs;

        public Action<FeedUpdate> OnUpdate => onUpdate;

        public int DisposeCount => Volatile.Read(ref _disposeCount);

        public void Update(FeedRequest replacement)
        {
            lock (_gate)
            {
                _requests.Add(replacement);
            }
        }

        public void Dispose() => Interlocked.Increment(ref _disposeCount);
    }
}

/// <summary>A test-side pipe client speaking the framed protocol.</summary>
internal sealed class TestClient(PipeStream stream) : IDisposable
{
    private const uint GenericRead = 0x80000000;
    private const uint GenericWrite = 0x40000000;
    private const uint OpenExisting = 3;
    private const uint FileFlagOverlapped = 0x40000000;
    private const int ErrorFileNotFound = 2;
    private const int ErrorPipeBusy = 231;

    public PipeStream Stream => stream;

    public static async Task<TestClient> ConnectAsync(string pipeName, CancellationToken ct, int timeoutMs = 5000)
    {
        var client = new NamedPipeClientStream(".", pipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        try
        {
            await client.ConnectAsync(timeoutMs, ct);
        }
        catch
        {
            client.Dispose();
            throw;
        }

        return new TestClient(client);
    }

    /// <summary>
    /// Opens the pipe with a raw <c>CreateFileW</c>, retrying only on <c>ERROR_PIPE_BUSY</c> and
    /// failing the test on <c>ERROR_FILE_NOT_FOUND</c> (the pipe name vanished between two
    /// instances), which <see cref="NamedPipeClientStream"/> would silently retry.
    /// </summary>
    public static TestClient RawConnect(string pipeName)
    {
        string path = $@"\\.\pipe\{pipeName}";
        var elapsed = Stopwatch.StartNew();
        while (true)
        {
            SafePipeHandle handle = CreateFileW(path, GenericRead | GenericWrite, 0, IntPtr.Zero, OpenExisting, FileFlagOverlapped, IntPtr.Zero);
            if (!handle.IsInvalid)
            {
                return new TestClient(new NamedPipeClientStream(PipeDirection.InOut, isAsync: true, isConnected: true, handle));
            }

            int error = Marshal.GetLastPInvokeError();
            handle.Dispose();
            Assert.NotEqual(ErrorFileNotFound, error);
            Assert.Equal(ErrorPipeBusy, error);
            Assert.True(elapsed.Elapsed < TimeSpan.FromSeconds(5), "the pipe stayed busy for 5 s");
            _ = WaitNamedPipeW(path, 1000);
        }
    }

    public async Task<IMessage?> ReadAsync(CancellationToken ct, TimeSpan? timeout = null)
    {
        using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
        cts.CancelAfter(timeout ?? TimeSpan.FromSeconds(5));
        return await FrameReader.ReadAsync(stream, cts.Token);
    }

    public async Task<T> ReadAsync<T>(CancellationToken ct, TimeSpan? timeout = null)
        where T : IMessage => Assert.IsType<T>(await ReadAsync(ct, timeout));

    public Task SendAsync(IMessage message, CancellationToken ct) => SendRawAsync(MessageCodec.EncodeFrame(message), ct);

    public async Task SendRawAsync(byte[] bytes, CancellationToken ct) => await stream.WriteAsync(bytes, ct);

    public void Dispose() => stream.Dispose();

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafePipeHandle CreateFileW(
        string lpFileName,
        uint dwDesiredAccess,
        uint dwShareMode,
        IntPtr lpSecurityAttributes,
        uint dwCreationDisposition,
        uint dwFlagsAndAttributes,
        IntPtr hTemplateFile);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool WaitNamedPipeW(string lpNamedPipeName, uint nTimeOut);
}

/// <summary>
/// A running <see cref="PipeListener"/> on a unique test pipe name with the default DACL (an
/// unprivileged server with the production DACL cannot create its own second instance, spike S3),
/// a <see cref="FakeFeed"/> and an <see cref="IdleShutdown"/> on a <see cref="FakeTimeProvider"/>.
/// </summary>
internal sealed class ListenerHarness : IAsyncDisposable
{
    private ListenerHarness(PipeListenerOptions options, PawnIoStatus pawnIo)
    {
        Options = options;
        Idle = new IdleShutdown(Lifetime, Time, TimeSpan.FromMinutes(2));
        Listener = new PipeListener(Feed, options, Idle, new PawnIoState(() => pawnIo), Log);
    }

    public PipeListenerOptions Options { get; }

    public string PipeName => Options.PipeName;

    public FakeFeed Feed { get; } = new();

    public FakeTimeProvider Time { get; } = new();

    public FakeLifetime Lifetime { get; } = new();

    public ListLogger<PipeListener> Log { get; } = new();

    public IdleShutdown Idle { get; }

    public PipeListener Listener { get; }

    public static string NewPipeName() => $"OpenMonitorAdvanced.Sensors.test-{Guid.NewGuid():N}";

    public static async Task<ListenerHarness> StartAsync(
        CancellationToken ct,
        TimeSpan? subscribeTimeout = null,
        TimeSpan? writeTimeout = null,
        string? sddl = null,
        PawnIoStatus pawnIo = PawnIoStatus.Ok)
    {
        var defaults = new PipeListenerOptions();
        var harness = new ListenerHarness(new PipeListenerOptions
        {
            PipeName = NewPipeName(),
            SecurityDescriptorSddl = sddl,
            SubscribeTimeout = subscribeTimeout ?? defaults.SubscribeTimeout,
            WriteTimeout = writeTimeout ?? defaults.WriteTimeout,
        }, pawnIo);
        await harness.Listener.StartAsync(ct);
        return harness;
    }

    /// <summary>Connects, reads <see cref="HelloMessage"/>, subscribes and waits until the feed saw the subscription.</summary>
    public async Task<TestClient> SubscribedClientAsync(uint intervalMs, CancellationToken ct)
    {
        int before = Feed.All.Count;
        TestClient client = await TestClient.ConnectAsync(PipeName, ct);
        try
        {
            await client.ReadAsync<HelloMessage>(ct);
            await client.SendAsync(new SubscribeMessage(intervalMs, [], []), ct);
            await PipeAssert.EventuallyAsync(() => Feed.All.Count > before, "the subscription", ct);
            return client;
        }
        catch
        {
            client.Dispose();
            throw;
        }
    }

    public async ValueTask DisposeAsync()
    {
        await Listener.StopAsync(CancellationToken.None);
        Listener.Dispose();
    }
}

internal static class PipeAssert
{
    public static async Task EventuallyAsync(Func<bool> condition, string what, CancellationToken ct, TimeSpan? timeout = null)
    {
        var elapsed = Stopwatch.StartNew();
        while (!condition())
        {
            if (elapsed.Elapsed > (timeout ?? TimeSpan.FromSeconds(5)))
            {
                Assert.Fail($"Timed out waiting for {what}");
            }

            await Task.Delay(10, ct);
        }
    }

    /// <summary>Compares two messages by their wire encoding (records hold lists, compared by reference).</summary>
    public static void SameMessage(IMessage expected, IMessage actual) =>
        Assert.Equal(MessageCodec.EncodePayload(expected), MessageCodec.EncodePayload(actual));
}
