using System.Threading.Channels;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Frames;

/// <summary>What the app asked the frame engine to capture (spec M7b §4.1, SD1).</summary>
internal sealed record FramesOptions(bool TrackPcLatency, bool TrackGpu);

/// <summary>
/// Lifecycle of the PresentMon capture (spec M7b §4.1): executable check, start, header check,
/// a 100 ms ETW flush while running, retries with backoff after a crash, and shutdown.
/// <para>
/// One worker task owns the whole state machine and handles commands from a channel, in order:
/// <see cref="Configure"/> calls, the header outcome and the exit of each run (posted by that
/// run's reader task) and retry timer ticks. So <see cref="Configure"/> never blocks its caller
/// (disposing a run can take up to 5 s, and happens on the worker), and there are no races
/// between a reconfiguration and a crash: whatever comes second sees the result of the first.
/// Events from runs that are no longer current carry an old run id and are ignored. All timing
/// goes through the injected <see cref="TimeProvider"/>.
/// </para>
/// </summary>
internal sealed class FrameCapture : IDisposable
{
    internal const string SessionName = "OpenMonitorAdvanced-Frames";
    internal const int CrashesToFail = 5;
    internal static readonly TimeSpan FlushInterval = TimeSpan.FromMilliseconds(100);
    internal static readonly TimeSpan FirstRetry = TimeSpan.FromSeconds(1);
    internal static readonly TimeSpan MaxRetry = TimeSpan.FromSeconds(60);
    internal static readonly TimeSpan HealthyRun = TimeSpan.FromSeconds(60);
    internal static readonly TimeSpan CrashWindow = TimeSpan.FromMinutes(10);

    /// <summary><c>ERROR_WMI_INSTANCE_NOT_FOUND</c>: there was no session to stop.</summary>
    private const uint NothingToStop = 4201;

    private static readonly TimeSpan DisposeWait = TimeSpan.FromSeconds(15);
    private static readonly FramesStatusMessage OffStatus = new(FramesStates.Off, null, null);

    private readonly IFrameSource _source;
    private readonly IEtwSession _etw;
    private readonly Func<string?> _currentHash;
    private readonly TimeProvider _time;
    private readonly ILogger<FrameCapture> _log;
    private readonly Channel<Command> _commands = Channel.CreateUnbounded<Command>(
        new UnboundedChannelOptions { SingleReader = true });

    private readonly Task _worker;
    private volatile FramesStatusMessage _status = OffStatus;
    private int _disposed;

    /// <summary>Id of the run the flush timer belongs to (0 = none); read by the timer thread.</summary>
    private long _flushRun;
    private int _flushFailureLogged;

    // Owned by the worker.
    private FramesOptions? _options;
    private ActiveRun? _active;
    private long _lastRunId;
    private ITimer? _retryTimer;
    private long _retryGeneration;
    private TimeSpan _nextRetry = FirstRetry;
    private readonly Queue<DateTimeOffset> _crashes = new();

    public FrameCapture(IFrameSource source, IEtwSession etw, Func<string?> currentHash, TimeProvider time, ILogger<FrameCapture> log)
    {
        _source = source;
        _etw = etw;
        _currentHash = currentHash;
        _time = time;
        _log = log;

        // A session left over by a previous service that crashed outlives its PresentMon (it lives in the kernel).
        StopSession();
        _worker = Task.Run(WorkAsync);
    }

    /// <summary>
    /// Raised on the worker thread at every status change. A handler must not call
    /// <see cref="Dispose"/>: that waits for the worker, which is the thread running the handler.
    /// </summary>
    public event Action<FramesStatusMessage>? StatusChanged;

    /// <summary>
    /// Raised on the reading thread for each parsed row of the current run, with the arrival time
    /// from <see cref="TimeProvider.GetTimestamp"/>. Rows still buffered when a run is stopped are
    /// dropped (at most a row already being raised can finish). A handler that throws ends the
    /// reading: the run is killed and handled as a crash.
    /// </summary>
    public event Action<PresentMonRow, long>? RowParsed;

    /// <summary>Raised on the worker thread once a retry is armed, with its wait (test seam).</summary>
    internal event Action<TimeSpan>? RetryScheduled;

    public FramesStatusMessage Status => _status;

    /// <summary>
    /// <see langword="null"/> turns the capture off; the same options keep a starting or running
    /// capture as it is; different options restart it; any options retry after denied, tampered,
    /// missing or failed. Returns at once: the work happens on the worker.
    /// </summary>
    public void Configure(FramesOptions? options) => Post(new ConfigureCommand(options));

    /// <summary>The PresentMon arguments of spike decision SD1.</summary>
    internal static IReadOnlyList<string> Arguments(FramesOptions o)
    {
        List<string> args =
        [
            "--output_stdout", "--no_console_stats", "--qpc_time", "--track_frame_type", "--write_frame_id",
            "--session_name", SessionName, "--stop_existing_session", "--no_track_input",
        ];
        if (o.TrackPcLatency)
        {
            args.Add("--track_pc_latency");
        }

        if (!o.TrackGpu)
        {
            args.Add("--no_track_gpu");
        }

        return args;
    }

    /// <summary>Stops the capture (run, then ETW session) and waits for the worker to finish.</summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
        {
            return;
        }

        _commands.Writer.TryWrite(ShutdownCommand.Instance);
        _commands.Writer.TryComplete();
        if (!_worker.Wait(DisposeWait))
        {
            _log.LogWarning("Frame capture did not stop within {Seconds} s", DisposeWait.TotalSeconds);
        }
    }

    private void Post(Command command) => _commands.Writer.TryWrite(command);

    private async Task WorkAsync()
    {
        await foreach (Command command in _commands.Reader.ReadAllAsync().ConfigureAwait(false))
        {
            try
            {
                Handle(command);
            }
            catch (Exception e)
            {
                _log.LogError(e, "Frame capture: handling {Command} failed", command.GetType().Name);
            }

            if (command is ShutdownCommand)
            {
                return;
            }
        }
    }

    private void Handle(Command command)
    {
        switch (command)
        {
            case ConfigureCommand c:
                OnConfigure(c.Options);
                break;
            case HeaderCommand h:
                OnHeader(h.RunId, h.MissingColumn);
                break;
            case ExitedCommand e:
                OnExited(e.RunId, e.Exit, e.Rejected);
                break;
            case RetryCommand r:
                OnRetry(r.Generation);
                break;
            case ShutdownCommand:
                _options = null;
                StopAll();
                Publish(OffStatus);
                break;
        }
    }

    private void OnConfigure(FramesOptions? options)
    {
        if (options is null)
        {
            _options = null;
            StopAll();
            Publish(OffStatus);
            return;
        }

        bool active = _status.State is FramesStates.Starting or FramesStates.Running;
        if (active && options == _options)
        {
            return;
        }

        StopAll();
        _options = options;
        _nextRetry = FirstRetry;
        _crashes.Clear();
        TryStart();
    }

    private void TryStart()
    {
        string? hash;
        try
        {
            hash = _currentHash();
        }
        catch (Exception e)
        {
            _log.LogWarning(e, "PresentMon: cannot hash the executable");
            OnCrash(runningSince: null);
            return;
        }

        if (hash is null)
        {
            _log.LogWarning("PresentMon executable not found: frame capture unavailable");
            Publish(new FramesStatusMessage(FramesStates.Missing, null, null));
            return;
        }

        if (!string.Equals(hash, PresentMonPin.Sha256, StringComparison.OrdinalIgnoreCase))
        {
            _log.LogWarning("PresentMon executable has an unexpected SHA-256 {Hash}: not started", hash);
            Publish(new FramesStatusMessage(FramesStates.Tampered, null, null));
            return;
        }

        Publish(new FramesStatusMessage(FramesStates.Starting, null, PresentMonPin.Version));
        IPresentMonRun run;
        try
        {
            run = _source.Start(Arguments(_options!));
        }
        catch (Exception e)
        {
            _log.LogWarning(e, "PresentMon failed to start");
            OnCrash(runningSince: null);
            return;
        }

        long id = ++_lastRunId;
        var active = new ActiveRun(id, run);
        _active = active;
        _log.LogInformation("PresentMon {Version} started (run {Run})", PresentMonPin.Version, id);
        _ = Task.Run(() => ReadAsync(id, run, active.Stopped.Token));
    }

    /// <summary>
    /// Reader of one run: header, then rows; posts the header outcome and the exit to the worker.
    /// <paramref name="stopped"/> is cancelled when the worker drops the run, so lines still
    /// buffered from it are never raised.
    /// </summary>
    private async Task ReadAsync(long id, IPresentMonRun run, CancellationToken stopped)
    {
        var csv = new PresentMonCsv();
        try
        {
            bool header = false;
            try
            {
                await foreach (string line in run.StdoutLines.ReadAllAsync(stopped).ConfigureAwait(false))
                {
                    if (stopped.IsCancellationRequested)
                    {
                        break;
                    }

                    if (header)
                    {
                        if (csv.ParseRow(line) is { } row)
                        {
                            RowParsed?.Invoke(row, _time.GetTimestamp());
                        }

                        continue;
                    }

                    if (string.IsNullOrWhiteSpace(line))
                    {
                        continue;
                    }

                    header = csv.TryReadHeader(line, out string? missing);
                    Post(new HeaderCommand(id, missing));
                    if (!header)
                    {
                        return; // the worker kills the run; nothing more to read
                    }
                }
            }
            catch (OperationCanceledException) when (stopped.IsCancellationRequested)
            {
                // The worker dropped this run; its exit is stale and will be ignored.
            }

            PresentMonExit exit = await run.Exited.ConfigureAwait(false);
            Post(new ExitedCommand(id, exit, csv.Rejected));
        }
        catch (Exception e)
        {
            _log.LogWarning(e, "PresentMon run {Run}: reading failed", id);
            Post(new ExitedCommand(id, new PresentMonExit(-1, []), csv.Rejected));
        }
    }

    private void OnHeader(long id, string? missingColumn)
    {
        if (_active is not { } active || active.Id != id)
        {
            return;
        }

        if (missingColumn is not null)
        {
            _log.LogError("PresentMon output lacks the column {Column}: frame capture failed", missingColumn);
            StopAll();
            Publish(new FramesStatusMessage(FramesStates.Failed, "columns", null));
            return;
        }

        active.RunningSince = _time.GetUtcNow();
        Interlocked.Exchange(ref _flushFailureLogged, 0);
        Interlocked.Exchange(ref _flushRun, id);
        active.FlushTimer = _time.CreateTimer(_ => Flush(id), null, FlushInterval, FlushInterval);
        _log.LogInformation("PresentMon run {Run} is capturing", id);
        Publish(new FramesStatusMessage(FramesStates.Running, null, PresentMonPin.Version));
    }

    private void OnExited(long id, PresentMonExit exit, long rejected)
    {
        if (_active is not { } active || active.Id != id)
        {
            return;
        }

        DisposeRun();
        string? lastError = exit.StderrTail.Count > 0 ? exit.StderrTail[^1] : null;
        if (active.RunningSince is null
            && exit.StderrTail.Any(l => l.Contains("access denied", StringComparison.OrdinalIgnoreCase)))
        {
            _log.LogWarning("PresentMon cannot start the trace session (access denied): {Error}", lastError);
            StopSession();
            Publish(new FramesStatusMessage(FramesStates.Denied, null, null));
            return;
        }

        _log.LogWarning(
            "PresentMon run {Run} exited with code {Code} ({Rejected} rows rejected): {Error}",
            id, exit.ExitCode, rejected, lastError);
        OnCrash(active.RunningSince);
    }

    private void OnCrash(DateTimeOffset? runningSince)
    {
        DateTimeOffset now = _time.GetUtcNow();
        if (runningSince is { } since && now - since > HealthyRun)
        {
            _nextRetry = FirstRetry;
        }

        _crashes.Enqueue(now);
        while (now - _crashes.Peek() >= CrashWindow)
        {
            _crashes.Dequeue();
        }

        StopSession();
        if (_crashes.Count >= CrashesToFail)
        {
            _log.LogError("PresentMon crashed {Count} times in {Minutes} minutes: frame capture failed", _crashes.Count, CrashWindow.TotalMinutes);
            Publish(new FramesStatusMessage(FramesStates.Failed, "crashing", null));
            return;
        }

        TimeSpan delay = _nextRetry;
        _nextRetry = delay * 2 < MaxRetry ? delay * 2 : MaxRetry;
        Publish(new FramesStatusMessage(FramesStates.Starting, null, PresentMonPin.Version));
        long generation = ++_retryGeneration;
        _retryTimer = _time.CreateTimer(_ => Post(new RetryCommand(generation)), null, delay, Timeout.InfiniteTimeSpan);
        _log.LogInformation("PresentMon: retrying in {Seconds} s", delay.TotalSeconds);
        RetryScheduled?.Invoke(delay);
    }

    private void OnRetry(long generation)
    {
        if (generation != _retryGeneration || _options is null || _active is not null)
        {
            return;
        }

        CancelRetry();
        TryStart();
    }

    private void Flush(long id)
    {
        if (Interlocked.Read(ref _flushRun) != id)
        {
            return;
        }

        uint result = _etw.Flush(SessionName);
        if (result != 0 && Interlocked.Exchange(ref _flushFailureLogged, 1) == 0)
        {
            _log.LogDebug("Flushing the ETW session failed with {Status}", result);
        }
    }

    /// <summary>Cancels a pending retry and, with a run in progress, kills it and stops its session.</summary>
    private void StopAll()
    {
        CancelRetry();
        if (_active is not null)
        {
            DisposeRun();
            StopSession();
        }
    }

    private void CancelRetry()
    {
        _retryGeneration++;
        _retryTimer?.Dispose();
        _retryTimer = null;
    }

    private void DisposeRun()
    {
        if (_active is not { } active)
        {
            return;
        }

        _active = null;
        active.Stopped.Cancel();
        Interlocked.Exchange(ref _flushRun, 0);
        active.FlushTimer?.Dispose();
        active.Run.Dispose();

        // The source is not disposed: the reader may still check its token, and it holds no timer.
    }

    private void StopSession()
    {
        uint result = _etw.Stop(SessionName);
        if (result is not 0 and not NothingToStop)
        {
            _log.LogWarning("Stopping the ETW session {Session} failed with {Status}", SessionName, result);
        }
    }

    private void Publish(FramesStatusMessage status)
    {
        if (status == _status)
        {
            return;
        }

        _status = status;
        try
        {
            StatusChanged?.Invoke(status);
        }
        catch (Exception e)
        {
            _log.LogError(e, "Frame capture: a status handler failed");
        }
    }

    private sealed class ActiveRun(long id, IPresentMonRun run)
    {
        public long Id { get; } = id;

        public IPresentMonRun Run { get; } = run;

        public DateTimeOffset? RunningSince { get; set; }

        public ITimer? FlushTimer { get; set; }

        /// <summary>Cancelled when the worker drops the run: its reader stops raising rows.</summary>
        public CancellationTokenSource Stopped { get; } = new();
    }

    private abstract record Command;

    private sealed record ConfigureCommand(FramesOptions? Options) : Command;

    private sealed record HeaderCommand(long RunId, string? MissingColumn) : Command;

    private sealed record ExitedCommand(long RunId, PresentMonExit Exit, long Rejected) : Command;

    private sealed record RetryCommand(long Generation) : Command;

    private sealed record ShutdownCommand : Command
    {
        public static readonly ShutdownCommand Instance = new();
    }
}
