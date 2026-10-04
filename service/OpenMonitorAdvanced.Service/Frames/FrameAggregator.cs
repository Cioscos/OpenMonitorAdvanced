using System.Diagnostics;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Frames;

/// <summary>
/// Per-process frame bookkeeping between the PresentMon reader and the hub (spec M7b section 4.1). The
/// reader thread calls <see cref="Add"/>; the hub calls <see cref="TakeBatch"/>,
/// <see cref="TakeSummary"/>, <see cref="SetTargets"/>, <see cref="Clear"/> and reads <see cref="Stalls"/>. One internal lock
/// guards all state. <c>ticksPerSecond</c> is both the QPC frequency of <c>TimeInQPC</c> and the unit of
/// the arrival timestamps (<c>TimeProvider.GetTimestamp()</c>); on Windows they are the same clock.
/// </summary>
internal sealed class FrameAggregator(
    int maxFramesPerBatch = ProtocolConstants.MaxFramesPerBatch,
    int maxProcesses = ProtocolConstants.MaxPresentingProcesses,
    long ticksPerSecond = 0)
{
    private readonly object _lock = new();
    private readonly long _ticksPerSecond = ticksPerSecond > 0 ? ticksPerSecond : Stopwatch.Frequency;
    private readonly Dictionary<uint, Window> _windows = [];
    private readonly Dictionary<uint, Pending> _pending = [];
    private HashSet<uint> _targets = [];
    private ulong _lastQpc;
    private long _lastArrival;
    private bool _hasArrival;
    private int _stalls;

    private readonly record struct Shown(ulong Qpc, double Ms, string Mode, ulong Swapchain);

    private sealed class Window
    {
        public string Name = "";
        public readonly Queue<Shown> Shown = new();
    }

    private sealed class Pending
    {
        public List<WireFrame> Frames = [];
        public uint Dropped;
    }

    /// <summary>Arrivals more than one second apart while rows were flowing (SD7).</summary>
    public int Stalls
    {
        get
        {
            lock (_lock)
            {
                return _stalls;
            }
        }
    }

    /// <summary>Only rows of these processes become frames; pending frames of the others are discarded.</summary>
    public void SetTargets(IReadOnlyCollection<uint> pids)
    {
        lock (_lock)
        {
            _targets = [.. pids];
            foreach (var pid in _pending.Keys.Where(p => !_targets.Contains(p)).ToList())
            {
                _pending.Remove(pid);
            }
        }
    }

    public void Add(PresentMonRow row, long arrivalTicks)
    {
        lock (_lock)
        {
            if (_hasArrival && arrivalTicks - _lastArrival > _ticksPerSecond)
            {
                _stalls++;
            }

            _lastArrival = arrivalTicks;
            _hasArrival = true;

            var frame = row.Frame;
            if (frame.Qpc > _lastQpc)
            {
                _lastQpc = frame.Qpc;
            }

            if (frame.Displayed)
            {
                if (!_windows.TryGetValue(row.Pid, out var window))
                {
                    window = new Window();
                    _windows[row.Pid] = window;
                }

                window.Name = row.Name;
                window.Shown.Enqueue(new Shown(frame.Qpc, frame.MsBetweenDisplayChange!.Value, row.PresentMode, frame.Swapchain));
                Prune(window);
            }

            if (_targets.Contains(row.Pid))
            {
                if (!_pending.TryGetValue(row.Pid, out var pending))
                {
                    pending = new Pending();
                    _pending[row.Pid] = pending;
                }

                if (pending.Frames.Count < maxFramesPerBatch)
                {
                    pending.Frames.Add(frame);
                }
                else
                {
                    pending.Dropped++;
                }
            }
        }
    }

    /// <summary>
    /// Forgets every process window and pending batch, for when the capture stops: with no more rows
    /// nothing would age them out. Targets and <see cref="Stalls"/> stay; the silence until the next
    /// row is not counted as a stall.
    /// </summary>
    public void Clear()
    {
        lock (_lock)
        {
            _windows.Clear();
            _pending.Clear();
            _hasArrival = false;
        }
    }

    /// <summary>The frames gathered for <paramref name="pid"/> since the last call, or null when there are none.</summary>
    public FrameBatchMessage? TakeBatch(uint pid)
    {
        lock (_lock)
        {
            if (!_pending.Remove(pid, out var pending) || pending.Frames.Count == 0)
            {
                return null;
            }

            return new FrameBatchMessage(pid, pending.Frames, pending.Dropped);
        }
    }

    /// <summary>Processes that displayed a frame within the last second of frame time, fastest first.</summary>
    public PresentingProcessesMessage TakeSummary(ulong atQpc)
    {
        lock (_lock)
        {
            var list = new List<PresentingProcess>();
            foreach (var (pid, window) in _windows.ToList())
            {
                Prune(window);
                if (window.Shown.Count == 0)
                {
                    _windows.Remove(pid);
                    continue;
                }

                var totalMs = 0.0;
                var modes = new Dictionary<string, int>(StringComparer.Ordinal);
                var chains = new HashSet<ulong>();
                foreach (var s in window.Shown)
                {
                    totalMs += s.Ms;
                    modes[s.Mode] = modes.GetValueOrDefault(s.Mode) + 1;
                    chains.Add(s.Swapchain);
                }

                var fps = totalMs > 0 ? 1000.0 * window.Shown.Count / totalMs : 0.0;
                var mode = modes.MaxBy(kv => kv.Value).Key;
                list.Add(new PresentingProcess(pid, window.Name, fps, mode, (uint)chains.Count));
            }

            list.Sort((a, b) => b.DisplayedFps.CompareTo(a.DisplayedFps));
            if (list.Count > maxProcesses)
            {
                list.RemoveRange(maxProcesses, list.Count - maxProcesses);
            }

            return new PresentingProcessesMessage(atQpc, list);
        }
    }

    private void Prune(Window window)
    {
        var floor = _lastQpc > (ulong)_ticksPerSecond ? _lastQpc - (ulong)_ticksPerSecond : 0;
        while (window.Shown.Count > 0 && window.Shown.Peek().Qpc <= floor)
        {
            window.Shown.Dequeue();
        }
    }
}
