using System.Threading.Channels;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Frames;
using OpenMonitorAdvanced.Service.Protocol;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Frames;

public sealed class FrameCaptureTests : IDisposable
{
    private const string Session = "OpenMonitorAdvanced-Frames";

    private const string Header =
        "Application,ProcessID,SwapChainAddress,PresentMode,FrameType,TimeInQPC,MsBetweenPresents,"
        + "MsBetweenDisplayChange,MsUntilDisplayed,MsBetweenAppStart,MsPCLatency,MsGPUBusy,PCLFrameId";

    private const string Row =
        "game.exe,4242,0x22A59F87270,Hardware: Independent Flip,Application,366427208391,14.173,13.64,18.98,14.186,60.55,13.69,14347";

    private static readonly FramesOptions Plain = new(TrackPcLatency: false, TrackGpu: true);
    private static readonly FramesOptions WithPcl = new(TrackPcLatency: true, TrackGpu: true);

    private static readonly FramesStatusMessage Off = new(FramesStates.Off, null, null);
    private static readonly FramesStatusMessage Starting = new(FramesStates.Starting, null, PresentMonPin.Version);
    private static readonly FramesStatusMessage Running = new(FramesStates.Running, null, PresentMonPin.Version);

    private readonly FakeTimeProvider _time = new();
    private readonly FakeFrameSource _source = new();
    private readonly FakeEtw _etw = new();
    private readonly Channel<FramesStatusMessage> _statuses = Channel.CreateUnbounded<FramesStatusMessage>();
    private readonly Channel<TimeSpan> _retries = Channel.CreateUnbounded<TimeSpan>();
    private string? _hash = PresentMonPin.Sha256;
    private FrameCapture? _capture;

    public void Dispose() => _capture?.Dispose();

    private FrameCapture NewCapture()
    {
        _capture = new FrameCapture(_source, _etw, () => _hash, _time, NullLogger<FrameCapture>.Instance);
        _capture.StatusChanged += s => _statuses.Writer.TryWrite(s);
        _capture.RetryScheduled += d => _retries.Writer.TryWrite(d);
        return _capture;
    }

    private async Task<FramesStatusMessage> NextStatusAsync() =>
        await _statuses.Reader.ReadAsync(TestContext.Current.CancellationToken).AsTask().WaitAsync(FramesWait.Limit, TestContext.Current.CancellationToken);

    private async Task<TimeSpan> NextRetryAsync() =>
        await _retries.Reader.ReadAsync(TestContext.Current.CancellationToken).AsTask().WaitAsync(FramesWait.Limit, TestContext.Current.CancellationToken);

    /// <summary>Configures, waits for the run, feeds the header and waits for <c>running</c>.</summary>
    private async Task<FakeRun> StartRunningAsync(FrameCapture capture, FramesOptions options, int runIndex = 0)
    {
        capture.Configure(options);
        Assert.Equal(Starting, await NextStatusAsync());
        var run = await _source.RunAsync(runIndex);
        run.WriteLine(Header);
        Assert.Equal(Running, await NextStatusAsync());
        return run;
    }

    [Fact]
    public void StopsALeftoverSessionAtConstruction()
    {
        var names = new List<string>();
        _etw.OnStop = names.Add;

        var capture = NewCapture();

        Assert.Equal([Session], names);
        Assert.Equal(Off, capture.Status);
        Assert.Equal(0, _source.Starts.Count);
    }

    [Fact]
    public void ArgumentsMatchTheSpikeDecision()
    {
        string[] common =
        [
            "--output_stdout", "--no_console_stats", "--qpc_time", "--track_frame_type", "--write_frame_id",
            "--session_name", Session, "--stop_existing_session", "--no_track_input",
        ];

        Assert.Equal([.. common, "--no_track_gpu"], FrameCapture.Arguments(new FramesOptions(false, false)));
        Assert.Equal([.. common, "--track_pc_latency", "--no_track_gpu"], FrameCapture.Arguments(new FramesOptions(true, false)));
        Assert.Equal(common, FrameCapture.Arguments(new FramesOptions(false, true)));
        Assert.Equal([.. common, "--track_pc_latency"], FrameCapture.Arguments(new FramesOptions(true, true)));
    }

    [Fact]
    public async Task MissingExecutableReportsMissing()
    {
        _hash = null;
        var capture = NewCapture();

        capture.Configure(Plain);

        Assert.Equal(new FramesStatusMessage(FramesStates.Missing, null, null), await NextStatusAsync());
        Assert.Equal(0, _source.Starts.Count);
    }

    [Fact]
    public async Task WrongHashReportsTamperedAndNeverStarts()
    {
        _hash = new string('0', 64);
        var capture = NewCapture();

        capture.Configure(Plain);

        Assert.Equal(new FramesStatusMessage(FramesStates.Tampered, null, null), await NextStatusAsync());
        _time.Advance(TimeSpan.FromMinutes(5));
        capture.Configure(null);
        Assert.Equal(Off, await NextStatusAsync());
        Assert.Equal(0, _source.Starts.Count);
    }

    [Fact]
    public async Task HashIsComparedIgnoringCase()
    {
        _hash = PresentMonPin.Sha256.ToLowerInvariant();
        var capture = NewCapture();

        await StartRunningAsync(capture, Plain);

        Assert.Equal(1, _source.Starts.Count);
    }

    [Fact]
    public async Task ValidHeaderMeansRunningAndFlushesEveryHundredMilliseconds()
    {
        var capture = NewCapture();

        var run = await StartRunningAsync(capture, Plain);

        Assert.Equal(FrameCapture.Arguments(Plain), run.Arguments);
        Assert.Equal(Running, capture.Status);
        _time.Advance(TimeSpan.FromSeconds(1));
        Assert.Equal(10, _etw.Flushes);
    }

    [Fact]
    public async Task MissingColumnFailsWithColumns()
    {
        var capture = NewCapture();
        capture.Configure(Plain);
        Assert.Equal(Starting, await NextStatusAsync());
        var run = await _source.RunAsync(0);

        run.WriteLine(Header.Replace("MsUntilDisplayed,", ""));

        Assert.Equal(new FramesStatusMessage(FramesStates.Failed, "columns", null), await NextStatusAsync());
        Assert.True(run.IsDisposed);
        Assert.Equal(2, _etw.Stops.Count);
        _time.Advance(TimeSpan.FromMinutes(5));
        Assert.Equal(1, _source.Starts.Count);
        Assert.Equal(0, _etw.Flushes);
    }

    [Fact]
    public async Task AccessDeniedBeforeHeaderMeansDenied()
    {
        var capture = NewCapture();
        capture.Configure(Plain);
        Assert.Equal(Starting, await NextStatusAsync());
        var run = await _source.RunAsync(0);

        run.Exit(6, "error: failed to start trace session: access denied.");

        Assert.Equal(new FramesStatusMessage(FramesStates.Denied, null, null), await NextStatusAsync());
        _time.Advance(TimeSpan.FromMinutes(5));
        Assert.Equal(1, _source.Starts.Count);
        Assert.Equal(0, _retries.Reader.Count);
    }

    [Fact]
    public async Task CrashRestartsWithBackoff()
    {
        var capture = NewCapture();
        capture.Configure(Plain);
        Assert.Equal(Starting, await NextStatusAsync());
        var run0 = await _source.RunAsync(0);

        // A crash before the header: the session is stopped and a retry follows after 1 s.
        run0.Exit(3);
        Assert.Equal(TimeSpan.FromSeconds(1), await NextRetryAsync());
        Assert.Equal(2, _etw.Stops.Count);
        Assert.True(run0.IsDisposed);
        Assert.Equal(Starting, capture.Status);
        _time.Advance(TimeSpan.FromMilliseconds(999));
        Assert.Equal(1, _source.Starts.Count);
        _time.Advance(TimeSpan.FromMilliseconds(1));
        var run1 = await _source.RunAsync(1);

        // A crash while running: back to starting (no flushes any more), next wait 2 s.
        run1.WriteLine(Header);
        Assert.Equal(Running, await NextStatusAsync());
        run1.Exit(3);
        Assert.Equal(Starting, await NextStatusAsync());
        Assert.Equal(TimeSpan.FromSeconds(2), await NextRetryAsync());
        Assert.Equal(3, _etw.Stops.Count);
        int flushes = _etw.Flushes;
        _time.Advance(TimeSpan.FromSeconds(2));
        Assert.Equal(flushes, _etw.Flushes);
        var run2 = await _source.RunAsync(2);

        // A healthy run of more than 60 s resets the wait to 1 s.
        run2.WriteLine(Header);
        Assert.Equal(Running, await NextStatusAsync());
        _time.Advance(TimeSpan.FromSeconds(61));
        run2.Exit(0);
        Assert.Equal(Starting, await NextStatusAsync());
        Assert.Equal(TimeSpan.FromSeconds(1), await NextRetryAsync());
    }

    [Fact]
    public async Task BackoffDoublesUpToSixtySeconds()
    {
        var capture = NewCapture();
        capture.Configure(Plain);
        Assert.Equal(Starting, await NextStatusAsync());

        // Crashes 11 minutes apart never reach five in ten minutes; runs that never got a header
        // do not reset the wait.
        int[] expected = [1, 2, 4, 8, 16, 32, 60, 60];
        for (int i = 0; i < expected.Length; i++)
        {
            var run = await _source.RunAsync(i);
            _time.Advance(TimeSpan.FromMinutes(11));
            run.Exit(3);
            var delay = await NextRetryAsync();
            Assert.Equal(TimeSpan.FromSeconds(expected[i]), delay);
            _time.Advance(delay);
        }

        await _source.RunAsync(expected.Length);
        Assert.Equal(Starting, capture.Status);
        Assert.Equal(0, _statuses.Reader.Count);
    }

    [Fact]
    public async Task FiveCrashesInTenMinutesFail()
    {
        var capture = NewCapture();
        capture.Configure(Plain);
        Assert.Equal(Starting, await NextStatusAsync());

        for (int i = 0; i < 4; i++)
        {
            (await _source.RunAsync(i)).Exit(3);
            _time.Advance(await NextRetryAsync());
        }

        (await _source.RunAsync(4)).Exit(3);

        Assert.Equal(new FramesStatusMessage(FramesStates.Failed, "crashing", null), await NextStatusAsync());
        Assert.Equal(6, _etw.Stops.Count);
        _time.Advance(TimeSpan.FromMinutes(30));
        Assert.Equal(5, _source.Starts.Count);

        // The next Configure, even with the same options, tries again from a clean slate.
        capture.Configure(Plain);
        Assert.Equal(Starting, await NextStatusAsync());
        (await _source.RunAsync(5)).Exit(3);
        Assert.Equal(TimeSpan.FromSeconds(1), await NextRetryAsync());
    }

    [Fact]
    public async Task StartFailureCountsAsACrash()
    {
        _source.ThrowOnStart = new System.ComponentModel.Win32Exception(5);
        var capture = NewCapture();

        capture.Configure(Plain);

        Assert.Equal(Starting, await NextStatusAsync());
        Assert.Equal(TimeSpan.FromSeconds(1), await NextRetryAsync());
        _source.ThrowOnStart = null;
        _time.Advance(TimeSpan.FromSeconds(1));
        await _source.RunAsync(0);
    }

    [Fact]
    public async Task SameOptionsDoNotRestart()
    {
        var capture = NewCapture();
        var run = await StartRunningAsync(capture, Plain);

        capture.Configure(new FramesOptions(TrackPcLatency: false, TrackGpu: true));
        capture.Configure(null);

        // Commands run in order: the only change after the repeat is the stop.
        Assert.Equal(Off, await NextStatusAsync());
        Assert.Equal(1, _source.Starts.Count);
        Assert.True(run.IsDisposed);
    }

    [Fact]
    public async Task DifferentOptionsRestart()
    {
        var capture = NewCapture();
        var run0 = await StartRunningAsync(capture, Plain);

        capture.Configure(WithPcl);

        Assert.Equal(Starting, await NextStatusAsync());
        var run1 = await _source.RunAsync(1);
        Assert.True(run0.IsDisposed);
        Assert.Equal(FrameCapture.Arguments(WithPcl), run1.Arguments);
        Assert.Equal(2, _etw.Stops.Count);
        run1.WriteLine(Header);
        Assert.Equal(Running, await NextStatusAsync());
    }

    [Fact]
    public async Task ConfigureNullStopsTheJobAndTheSession()
    {
        var capture = NewCapture();
        var run = await StartRunningAsync(capture, Plain);
        bool? disposedAtStop = null;
        _etw.OnStop = _ => disposedAtStop = run.IsDisposed;

        capture.Configure(null);

        Assert.Equal(Off, await NextStatusAsync());
        Assert.True(disposedAtStop);
        Assert.Equal(2, _etw.Stops.Count);
        int flushes = _etw.Flushes;
        _time.Advance(TimeSpan.FromSeconds(1));
        Assert.Equal(flushes, _etw.Flushes);
    }

    [Fact]
    public async Task DisposeWhileStartingKillsTheRunAndStopsTheSession()
    {
        var capture = NewCapture();
        capture.Configure(Plain);
        Assert.Equal(Starting, await NextStatusAsync());
        var run = await _source.RunAsync(0);

        capture.Dispose();

        Assert.True(run.IsDisposed);
        Assert.Equal(2, _etw.Stops.Count);
        Assert.Equal(Off, capture.Status);
        capture.Configure(Plain);
        Assert.Equal(1, _source.Starts.Count);
    }

    [Fact]
    public async Task DisposeDuringARetryWaitNeverStartsAgain()
    {
        var capture = NewCapture();
        capture.Configure(Plain);
        Assert.Equal(Starting, await NextStatusAsync());
        (await _source.RunAsync(0)).Exit(3);
        await NextRetryAsync();

        capture.Dispose();
        _time.Advance(TimeSpan.FromMinutes(1));

        Assert.Equal(1, _source.Starts.Count);
        Assert.Equal(Off, capture.Status);
    }

    [Fact]
    public async Task RowsAreParsedAndRaised()
    {
        var capture = NewCapture();
        var rows = Channel.CreateUnbounded<(PresentMonRow Row, long Arrival)>();
        capture.RowParsed += (row, arrival) => rows.Writer.TryWrite((row, arrival));
        var run = await StartRunningAsync(capture, Plain);
        _time.Advance(TimeSpan.FromSeconds(3));
        long now = _time.GetTimestamp();

        run.WriteLine("not,a,row");
        run.WriteLine(Row);

        var (row, arrival) = await rows.Reader.ReadAsync(TestContext.Current.CancellationToken).AsTask().WaitAsync(FramesWait.Limit, TestContext.Current.CancellationToken);
        Assert.Equal(4242u, row.Pid);
        Assert.Equal("game.exe", row.Name);
        Assert.Equal(366427208391UL, row.Frame.Qpc);
        Assert.Equal(now, arrival);
        Assert.Equal(0, rows.Reader.Count);
    }
}
