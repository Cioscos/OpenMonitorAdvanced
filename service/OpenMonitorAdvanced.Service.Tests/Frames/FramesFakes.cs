using System.Threading.Channels;
using OpenMonitorAdvanced.Service.Frames;

namespace OpenMonitorAdvanced.Service.Tests.Frames;

/// <summary>Generous wait for signals from the capture worker; tests never sleep.</summary>
internal static class FramesWait
{
    public static readonly TimeSpan Limit = TimeSpan.FromSeconds(10);
}

/// <summary>A counter that tests can await until it reaches a value.</summary>
internal sealed class CountSignal
{
    private readonly Lock _gate = new();
    private readonly List<(int Target, TaskCompletionSource Done)> _waiters = [];
    private int _count;

    public int Count
    {
        get
        {
            lock (_gate)
            {
                return _count;
            }
        }
    }

    public void Increment()
    {
        List<TaskCompletionSource> ready = [];
        lock (_gate)
        {
            _count++;
            _waiters.RemoveAll(w =>
            {
                if (w.Target > _count)
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
    }

    public Task WaitForAsync(int target)
    {
        lock (_gate)
        {
            if (_count >= target)
            {
                return Task.CompletedTask;
            }

            var done = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            _waiters.Add((target, done));
            return done.Task.WaitAsync(FramesWait.Limit);
        }
    }
}

/// <summary>A capture run driven by the test: stdout lines through a channel, the exit through a task source.</summary>
internal sealed class FakeRun : IPresentMonRun
{
    private readonly Channel<string> _lines = Channel.CreateUnbounded<string>();
    private readonly TaskCompletionSource<PresentMonExit> _exit = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private int _disposed;

    public FakeRun(IReadOnlyList<string> arguments, bool keepStdoutOpenOnDispose = false)
    {
        Arguments = arguments;
        KeepStdoutOpenOnDispose = keepStdoutOpenOnDispose;
    }

    public IReadOnlyList<string> Arguments { get; }

    /// <summary>Like stdout lines still buffered after the kill: <see cref="Dispose"/> ends the process but not stdout.</summary>
    public bool KeepStdoutOpenOnDispose { get; }

    /// <summary>Counts reads of <see cref="Exited"/>: the reader only awaits it once it stopped reading stdout.</summary>
    public CountSignal ExitedAwaited { get; } = new();

    public ChannelReader<string> StdoutLines => _lines.Reader;

    public Task<PresentMonExit> Exited
    {
        get
        {
            ExitedAwaited.Increment();
            return _exit.Task;
        }
    }

    public bool IsDisposed => Volatile.Read(ref _disposed) != 0;

    public void WriteLine(string line) => _lines.Writer.TryWrite(line);

    /// <summary>Ends stdout and then the process, as the real pump does.</summary>
    public void Exit(int code, params string[] stderr)
    {
        _lines.Writer.TryComplete();
        _exit.TrySetResult(new PresentMonExit(code, stderr));
    }

    /// <summary>Like the job closing: the child dies with code 1.</summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) == 0)
        {
            if (KeepStdoutOpenOnDispose)
            {
                _exit.TrySetResult(new PresentMonExit(1, []));
            }
            else
            {
                Exit(1);
            }
        }
    }
}

/// <summary>Records every start; each one returns a new <see cref="FakeRun"/>.</summary>
internal sealed class FakeFrameSource : IFrameSource
{
    private readonly Lock _gate = new();
    private readonly List<FakeRun> _runs = [];

    public CountSignal Starts { get; } = new();

    /// <summary>New runs keep stdout open when disposed (see <see cref="FakeRun.KeepStdoutOpenOnDispose"/>).</summary>
    public bool KeepStdoutOpenOnDispose { get; set; }

    /// <summary>When set, <see cref="Start"/> throws it instead of starting (not counted in <see cref="Starts"/>).</summary>
    public Exception? ThrowOnStart { get; set; }

    public IReadOnlyList<FakeRun> Runs
    {
        get
        {
            lock (_gate)
            {
                return [.. _runs];
            }
        }
    }

    public IPresentMonRun Start(IReadOnlyList<string> arguments)
    {
        if (ThrowOnStart is { } ex)
        {
            throw ex;
        }

        var run = new FakeRun(arguments, KeepStdoutOpenOnDispose);
        lock (_gate)
        {
            _runs.Add(run);
        }

        Starts.Increment();
        return run;
    }

    /// <summary>Waits for the run with this zero-based index to start.</summary>
    public async Task<FakeRun> RunAsync(int index)
    {
        await Starts.WaitForAsync(index + 1);
        return Runs[index];
    }
}

/// <summary>Counts <c>Stop</c> and <c>Flush</c>; stopping a session returns 4201 (nothing to stop).</summary>
internal sealed class FakeEtw : IEtwSession
{
    private int _flushes;

    public CountSignal Stops { get; } = new();

    public int Flushes => Volatile.Read(ref _flushes);

    /// <summary>Called inside <see cref="Stop"/> before it is counted.</summary>
    public Action<string>? OnStop { get; set; }

    public uint Stop(string name)
    {
        OnStop?.Invoke(name);
        Stops.Increment();
        return 4201;
    }

    public uint Flush(string name)
    {
        Interlocked.Increment(ref _flushes);
        return 0;
    }
}
