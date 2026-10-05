using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Frames;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Tests.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Frames;

public sealed class FramesHubTests : IDisposable
{
    private const string Header =
        "Application,ProcessID,SwapChainAddress,PresentMode,FrameType,TimeInQPC,MsBetweenPresents,"
        + "MsBetweenDisplayChange,MsUntilDisplayed,MsBetweenAppStart,MsPCLatency,MsGPUBusy,PCLFrameId";

    private const string CsvRow =
        "game.exe,4242,0x22A59F87270,Hardware: Independent Flip,Application,366427208391,14.173,13.64,18.98,14.186,60.55,13.69,14347";

    private static readonly FramesConfigureMessage On = new(Enabled: true, TrackPcLatency: false, TrackGpu: true);
    private static readonly TimeSpan Tick = TimeSpan.FromMilliseconds(100);

    private readonly FakeTimeProvider _time = new();
    private readonly FakeFrameSource _source = new();
    private readonly FakeEtw _etw = new();
    private readonly ListLogger<FramesHub> _log = new();
    private readonly FrameCapture _capture;
    private readonly FrameAggregator _aggregator;
    private readonly FrameRequests _requests;
    private readonly FramesHub _hub;
    private readonly CountSignal _parsed = new();

    public FramesHubTests()
    {
        _capture = new FrameCapture(_source, _etw, () => PresentMonPin.Sha256, _time, NullLogger<FrameCapture>.Instance);
        _aggregator = new FrameAggregator(ticksPerSecond: _time.TimestampFrequency);
        _requests = new FrameRequests(_time);
        _hub = new FramesHub(_capture, _aggregator, _requests, _time, _log);

        // Added after the hub's handler: once this fires, the aggregator has the row.
        _capture.RowParsed += (_, _) => _parsed.Increment();
    }

    public void Dispose() => _hub.Dispose();

    private PresentMonRow Row(uint pid, ulong qpc, double? displayedMs = 10.0) =>
        new(pid, "game.exe", "Hardware: Independent Flip", new WireFrame(qpc, 1, "app", displayedMs is not null, 10.0, displayedMs, null, null, null, null, null));

    private Recorder Subscribe(int session)
    {
        var recorder = new Recorder();
        recorder.Subscription = _hub.Subscribe(session, recorder.Deliver);
        return recorder;
    }

    [Fact]
    public async Task BatchesAreDeliveredEveryHundredMilliseconds()
    {
        var client = Subscribe(1);
        _hub.OnConfigure(1, On);
        _hub.OnTarget(1, new FramesTargetMessage(4242));

        // Through the real wiring: a parsed CSV row lands in the aggregator.
        var run = await _source.RunAsync(0);
        run.WriteLine(Header);
        run.WriteLine(CsvRow);
        await _parsed.WaitForAsync(1);

        _time.Advance(Tick - TimeSpan.FromMilliseconds(1));
        Assert.Empty(client.Of<FrameBatchMessage>());

        _time.Advance(TimeSpan.FromMilliseconds(1));
        var batch = Assert.Single(client.Of<FrameBatchMessage>());
        Assert.Equal(4242u, batch.Pid);
        Assert.Equal(366427208391UL, Assert.Single(batch.Frames).Qpc);
        Assert.Equal(0u, batch.Dropped);

        _aggregator.Add(Row(4242, 366427208392), 0);
        _time.Advance(Tick);
        Assert.Equal(2, client.Of<FrameBatchMessage>().Count);

        _time.Advance(Tick); // nothing new: no empty batch
        Assert.Equal(2, client.Of<FrameBatchMessage>().Count);
    }

    [Fact]
    public void EachSessionGetsOnlyItsTarget()
    {
        var a = Subscribe(1);
        var b = Subscribe(2);
        var c = Subscribe(3);
        foreach (var session in new[] { 1, 2, 3 })
        {
            _hub.OnConfigure(session, On);
        }

        _hub.OnTarget(1, new FramesTargetMessage(10));
        _hub.OnTarget(2, new FramesTargetMessage(20));
        _hub.OnTarget(3, new FramesTargetMessage(10));

        _aggregator.Add(Row(10, 100), 0);
        _aggregator.Add(Row(20, 200), 0);
        _aggregator.Add(Row(30, 300), 0);
        _time.Advance(Tick);

        Assert.Equal([100UL], Assert.Single(a.Of<FrameBatchMessage>()).Frames.Select(f => f.Qpc));
        Assert.Equal([200UL], Assert.Single(b.Of<FrameBatchMessage>()).Frames.Select(f => f.Qpc));
        Assert.Equal([100UL], Assert.Single(c.Of<FrameBatchMessage>()).Frames.Select(f => f.Qpc));
    }

    [Fact]
    public void SummaryGoesToEverySubscriber()
    {
        var a = Subscribe(1);
        var b = Subscribe(2);
        _hub.OnConfigure(1, On);

        _aggregator.Add(Row(10, 1000), 0);
        _time.Advance(TimeSpan.FromSeconds(1) - Tick);
        Assert.Empty(a.Of<PresentingProcessesMessage>());

        _time.Advance(Tick);
        foreach (var client in new[] { a, b })
        {
            var summary = Assert.Single(client.Of<PresentingProcessesMessage>());
            Assert.Equal(10u, Assert.Single(summary.Processes).Pid);
            Assert.Equal((ulong)_time.GetTimestamp(), summary.AtQpc);
        }
    }

    [Fact]
    public void UndeliveredBatchAddsToDropped()
    {
        var client = Subscribe(1);
        _hub.OnConfigure(1, On);
        _hub.OnTarget(1, new FramesTargetMessage(10));

        client.Accept = false;
        for (ulong i = 0; i < 3; i++)
        {
            _aggregator.Add(Row(10, 100 + i), 0);
        }

        _time.Advance(Tick);
        Assert.Empty(client.Of<FrameBatchMessage>());

        client.Accept = true;
        _aggregator.Add(Row(10, 200), 0);
        _aggregator.Add(Row(10, 201), 0);
        _time.Advance(Tick);

        var batch = Assert.Single(client.Of<FrameBatchMessage>());
        Assert.Equal([200UL, 201UL], batch.Frames.Select(f => f.Qpc));
        Assert.Equal(3u, batch.Dropped);
    }

    [Fact]
    public void FramesGatheredWithNoSubscriberAreNotDeliveredLater()
    {
        // A client asked for PID 10 and left: within its grace the target stays, nobody reads.
        _hub.OnConfigure(1, On);
        _hub.OnTarget(1, new FramesTargetMessage(10));
        _hub.OnDisconnected(1);
        for (ulong i = 0; i < 600; i++)
        {
            _aggregator.Add(Row(10, 100 + i), 0);
        }

        _time.Advance(TimeSpan.FromSeconds(5));

        // It reconnects as a new session for the same game.
        var client = Subscribe(2);
        _hub.OnConfigure(2, On);
        _hub.OnTarget(2, new FramesTargetMessage(10));
        _aggregator.Add(Row(10, 5000), 0);
        _time.Advance(Tick);

        var batch = Assert.Single(client.Of<FrameBatchMessage>());
        Assert.Equal([5000UL], batch.Frames.Select(f => f.Qpc));
        Assert.Equal(0u, batch.Dropped);
    }

    [Fact]
    public void SessionsSubscribedDisabledStartNoTimers()
    {
        var a = Subscribe(1);
        var b = Subscribe(2);
        _hub.OnConfigure(1, new FramesConfigureMessage(Enabled: false, TrackPcLatency: false, TrackGpu: false));
        _hub.OnConfigure(2, new FramesConfigureMessage(Enabled: false, TrackPcLatency: false, TrackGpu: false));

        _time.Advance(TimeSpan.FromSeconds(5));

        foreach (var client in new[] { a, b })
        {
            Assert.Empty(client.Of<PresentingProcessesMessage>());
            Assert.Empty(client.Of<FrameBatchMessage>());
            Assert.Single(client.Of<FramesStatusMessage>()); // only the one on subscribing
        }
    }

    [Fact]
    public void EnablingASessionStartsTheTimersAndDisablingStopsThem()
    {
        var client = Subscribe(1);
        _hub.OnTarget(1, new FramesTargetMessage(10));
        _time.Advance(TimeSpan.FromSeconds(2));
        Assert.Empty(client.Of<PresentingProcessesMessage>());

        _hub.OnConfigure(1, On);
        _aggregator.Add(Row(10, 100), 0);
        _time.Advance(TimeSpan.FromSeconds(1));
        Assert.NotEmpty(client.Of<PresentingProcessesMessage>());
        int summaries = client.Of<PresentingProcessesMessage>().Count;
        Assert.Single(client.Of<FrameBatchMessage>());

        _hub.OnConfigure(1, new FramesConfigureMessage(Enabled: false, TrackPcLatency: false, TrackGpu: false));
        _aggregator.Add(Row(10, 200), 0);
        _time.Advance(TimeSpan.FromSeconds(5));
        Assert.Equal(summaries, client.Of<PresentingProcessesMessage>().Count);
        Assert.Single(client.Of<FrameBatchMessage>());
    }

    [Fact]
    public async Task StatusChangesAreBroadcast()
    {
        var a = Subscribe(1);
        var b = Subscribe(2);

        // The current status on subscribing, then every change.
        Assert.Equal(FramesStates.Off, Assert.Single(a.Of<FramesStatusMessage>()).State);
        Assert.Equal(FramesStates.Off, Assert.Single(b.Of<FramesStatusMessage>()).State);

        _hub.OnConfigure(1, On);
        await a.WaitForAsync(m => m is FramesStatusMessage { State: FramesStates.Starting });
        await b.WaitForAsync(m => m is FramesStatusMessage { State: FramesStates.Starting });

        var run = await _source.RunAsync(0);
        run.WriteLine(Header);
        await a.WaitForAsync(m => m is FramesStatusMessage { State: FramesStates.Running });
        await b.WaitForAsync(m => m is FramesStatusMessage { State: FramesStates.Running });
    }

    [Fact]
    public async Task LeavingRunningClearsTheProcessListAndPendingFrames()
    {
        var client = Subscribe(1);
        _hub.OnConfigure(1, On);
        _hub.OnTarget(1, new FramesTargetMessage(4242));
        var run = await _source.RunAsync(0);
        run.WriteLine(Header);
        await client.WaitForAsync(m => m is FramesStatusMessage { State: FramesStates.Running });
        run.WriteLine(CsvRow);
        await _parsed.WaitForAsync(1);
        _time.Advance(TimeSpan.FromSeconds(1));
        Assert.Equal(4242u, Assert.Single(client.Of<PresentingProcessesMessage>()[^1].Processes).Pid);
        int batches = client.Of<FrameBatchMessage>().Count;

        // A frame still pending when PresentMon crashes: no more rows will come to age it out.
        _aggregator.Add(Row(4242, 366427208400), 0);
        var watcher = Subscribe(2); // receives Running at once, then the change
        run.Exit(3);
        await watcher.WaitForAsync(m => m is FramesStatusMessage { State: FramesStates.Starting });

        _time.Advance(TimeSpan.FromSeconds(1));
        Assert.Empty(client.Of<PresentingProcessesMessage>()[^1].Processes);
        Assert.Equal(batches, client.Of<FrameBatchMessage>().Count);
    }

    [Fact]
    public async Task UndeliveredStatusIsSentOnTheNextTick()
    {
        var client = Subscribe(1);
        client.Accept = false;
        _hub.OnConfigure(1, On);
        await _source.RunAsync(0); // Starting was published (and refused) before the run started

        client.Accept = true;
        _time.Advance(Tick);

        Assert.Equal(FramesStates.Starting, client.Of<FramesStatusMessage>()[^1].State);
    }

    [Fact]
    public async Task CaptureStopsWhenTheGraceAfterDisconnectEnds()
    {
        var client = Subscribe(1);
        _hub.OnConfigure(1, On);
        await client.WaitForAsync(m => m is FramesStatusMessage { State: FramesStates.Starting });
        var other = Subscribe(2);
        client.Subscription!.Dispose();
        _hub.OnDisconnected(1);

        _time.Advance(TimeSpan.FromSeconds(29));
        Assert.NotNull(_requests.Effective);

        _time.Advance(TimeSpan.FromSeconds(1));
        await other.WaitForAsync(m => m is FramesStatusMessage { State: FramesStates.Off });
        Assert.True((await _source.RunAsync(0)).IsDisposed);
    }

    [Fact]
    public void StallsAreLoggedAtMostOncePerMinute()
    {
        Subscribe(1);
        _hub.OnConfigure(1, On);
        long second = _time.TimestampFrequency;

        _aggregator.Add(Row(10, 100), 0);
        _aggregator.Add(Row(10, 101), 2 * second);
        _time.Advance(TimeSpan.FromSeconds(1));
        Assert.Single(StallLines());

        _aggregator.Add(Row(10, 102), 4 * second);
        _time.Advance(TimeSpan.FromSeconds(30));
        Assert.Single(StallLines());

        _time.Advance(TimeSpan.FromSeconds(30));
        Assert.Equal(2, StallLines().Count);
        Assert.All(StallLines(), e => Assert.Equal(LogLevel.Debug, e.Level));

        _time.Advance(TimeSpan.FromMinutes(2)); // no new stall: nothing to log
        Assert.Equal(2, StallLines().Count);
    }

    private List<LogEntry> StallLines() =>
        [.. _log.Entries.Where(e => e.Message.Contains("over 1 s", StringComparison.Ordinal))];

    /// <summary>A session's delivery callback: keeps what it accepts, refuses everything while <see cref="Accept"/> is false.</summary>
    private sealed class Recorder
    {
        private readonly Lock _gate = new();
        private readonly List<IMessage> _messages = [];
        private readonly List<(Func<IMessage, bool> Match, TaskCompletionSource Done)> _waiters = [];

        public volatile bool Accept = true;

        public IDisposable? Subscription { get; set; }

        public List<T> Of<T>()
            where T : IMessage
        {
            lock (_gate)
            {
                return [.. _messages.OfType<T>()];
            }
        }

        public bool Deliver(IMessage message)
        {
            if (!Accept)
            {
                return false;
            }

            List<TaskCompletionSource> ready = [];
            lock (_gate)
            {
                _messages.Add(message);
                _waiters.RemoveAll(w =>
                {
                    if (!w.Match(message))
                    {
                        return false;
                    }

                    ready.Add(w.Done);
                    return true;
                });
            }

            foreach (var done in ready)
            {
                done.TrySetResult();
            }

            return true;
        }

        public Task WaitForAsync(Func<IMessage, bool> match)
        {
            lock (_gate)
            {
                if (_messages.Any(match))
                {
                    return Task.CompletedTask;
                }

                var done = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
                _waiters.Add((match, done));
                return done.Task.WaitAsync(FramesWait.Limit);
            }
        }
    }
}
