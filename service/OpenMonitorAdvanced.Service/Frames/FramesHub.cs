using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Frames;

/// <summary>
/// Serves frame data to the pipe sessions that asked for it (spec M7b §4.1): every
/// <see cref="BatchInterval"/> the <see cref="FrameBatchMessage"/> of each session's target, every
/// <see cref="SummaryInterval"/> the <see cref="PresentingProcessesMessage"/> to all of them, and
/// every <see cref="FramesStatusMessage"/> change as it happens.
/// <para>
/// <b>Requests.</b> The requests of every session are combined by <see cref="FrameRequests"/>; each
/// change reconfigures the capture (only when the combined options change, except for an explicit
/// <see cref="OnConfigure"/>, which also retries a failed capture) and the aggregator's targets. A
/// disconnected session's request expires after <see cref="FrameRequests.Grace"/>, on a timer.
/// </para>
/// <para>
/// <b>Delivery never blocks.</b> A session's <c>deliver</c> only queues and returns
/// <see langword="false"/> when its queue is full. A refused batch adds its frames and its
/// <c>Dropped</c> to the <c>Dropped</c> of that session's next batch; a refused summary is lost; a
/// refused status is sent again (the current one) on the next batch tick. Deliveries happen under
/// the hub's lock, so a session sees statuses in order and never one after its subscription ended.
/// </para>
/// <para>
/// <b>Timers</b> (through the injected <see cref="TimeProvider"/>) run only while a session is
/// subscribed. The hub owns the capture: <see cref="Dispose"/> stops the timers, unhooks the
/// capture's events and disposes it (which stops PresentMon and its ETW session).
/// </para>
/// </summary>
internal sealed class FramesHub : IDisposable
{
    internal static readonly TimeSpan BatchInterval = TimeSpan.FromMilliseconds(100);
    internal static readonly TimeSpan SummaryInterval = TimeSpan.FromSeconds(1);
    internal static readonly TimeSpan StallLogInterval = TimeSpan.FromMinutes(1);

    private readonly FrameCapture _capture;
    private readonly FrameAggregator _aggregator;
    private readonly FrameRequests _requests;
    private readonly TimeProvider _time;
    private readonly ILogger<FramesHub> _log;
    private readonly Action<PresentMonRow, long> _onRow;
    private readonly Action<FramesStatusMessage> _onStatus;

    private readonly Lock _gate = new();
    private readonly Dictionary<int, Subscriber> _subscribers = [];
    private ITimer? _batchTimer;
    private ITimer? _summaryTimer;
    private ITimer? _expiryTimer;
    private FramesOptions? _applied;
    private int _loggedStalls;
    private DateTimeOffset? _lastStallLog;
    private bool _disposed;

    public FramesHub(FrameCapture capture, FrameAggregator aggregator, FrameRequests requests, TimeProvider time, ILogger<FramesHub> log)
    {
        _capture = capture;
        _aggregator = aggregator;
        _requests = requests;
        _time = time;
        _log = log;
        _onRow = aggregator.Add;
        _onStatus = OnStatusChanged;
        capture.RowParsed += _onRow;
        capture.StatusChanged += _onStatus;
    }

    /// <summary>
    /// Subscribes a session, which at once receives the current status. <paramref name="deliver"/>
    /// is called under the hub's lock from timer and capture threads: it must only queue, and
    /// return <see langword="false"/> when the session's queue is full.
    /// </summary>
    public IDisposable Subscribe(int session, Func<IMessage, bool> deliver)
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return new Subscription(this, null);
            }

            var subscriber = new Subscriber(session, deliver);
            _subscribers[session] = subscriber;
            subscriber.StatusOwed = !deliver(_capture.Status);
            if (_batchTimer is null)
            {
                _batchTimer = _time.CreateTimer(_ => OnBatchTick(), null, BatchInterval, BatchInterval);
                _summaryTimer = _time.CreateTimer(_ => OnSummaryTick(), null, SummaryInterval, SummaryInterval);
            }

            return new Subscription(this, subscriber);
        }
    }

    public void OnConfigure(int session, FramesConfigureMessage message)
    {
        lock (_gate)
        {
            _requests.Configure(session, message);
            Apply(explicitConfigure: true);
        }
    }

    public void OnTarget(int session, FramesTargetMessage message)
    {
        if (message.Pid is { } pid && !FrameRequests.IsValidPid(pid))
        {
            _log.LogWarning("Pipe client {Client} asked for frames of PID {Pid}, which never presents; ignored", session, pid);
        }

        lock (_gate)
        {
            _requests.Target(session, message.Pid);
            Apply(explicitConfigure: false);
        }
    }

    public void OnDisconnected(int session)
    {
        lock (_gate)
        {
            _requests.Disconnected(session);
            Apply(explicitConfigure: false);
        }
    }

    public void Dispose()
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            _subscribers.Clear();
            StopTickTimers();
            _expiryTimer?.Dispose();
            _expiryTimer = null;
        }

        // Outside the lock: a status handler running on the capture's worker may be waiting for
        // it, and the capture's Dispose waits for that worker.
        _capture.RowParsed -= _onRow;
        _capture.StatusChanged -= _onStatus;
        _capture.Dispose();
    }

    /// <summary>Pushes the combined requests to the capture and the aggregator; under the lock.</summary>
    private void Apply(bool explicitConfigure)
    {
        if (_disposed)
        {
            return;
        }

        FramesOptions? options = _requests.Effective;
        if (explicitConfigure || options != _applied)
        {
            _applied = options;
            _capture.Configure(options);
        }

        _aggregator.SetTargets(_requests.Targets);

        _expiryTimer?.Dispose();
        _expiryTimer = _requests.NextExpiry is { } due
            ? _time.CreateTimer(_ => OnExpiry(), null, due, Timeout.InfiniteTimeSpan)
            : null;
    }

    private void OnExpiry()
    {
        lock (_gate)
        {
            Apply(explicitConfigure: false);
        }
    }

    private void OnStatusChanged(FramesStatusMessage status)
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            foreach (Subscriber subscriber in _subscribers.Values)
            {
                subscriber.StatusOwed = !subscriber.Deliver(status);
            }
        }
    }

    private void OnBatchTick()
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            foreach (Subscriber subscriber in _subscribers.Values)
            {
                if (subscriber.StatusOwed)
                {
                    subscriber.StatusOwed = !subscriber.Deliver(_capture.Status);
                }
            }

            // Every target is taken, also those no subscriber follows now (a session within its
            // grace): their frames are discarded rather than piling up for a later reader.
            foreach (uint pid in _requests.Targets)
            {
                FrameBatchMessage? batch = _aggregator.TakeBatch(pid);
                foreach (Subscriber subscriber in _subscribers.Values)
                {
                    if (_requests.TargetOf(subscriber.Session) == pid)
                    {
                        Send(subscriber, pid, batch);
                    }
                }
            }
        }
    }

    /// <summary>Delivers <paramref name="batch"/> (may be null: nothing new) with what the session lost before.</summary>
    private static void Send(Subscriber subscriber, uint pid, FrameBatchMessage? batch)
    {
        if (subscriber.CarriedPid != pid)
        {
            subscriber.CarriedPid = pid;
            subscriber.CarriedDropped = 0;
        }

        if (batch is null)
        {
            return;
        }

        FrameBatchMessage message = subscriber.CarriedDropped == 0
            ? batch
            : batch with { Dropped = SaturatingAdd(batch.Dropped, subscriber.CarriedDropped) };
        subscriber.CarriedDropped = subscriber.Deliver(message)
            ? 0
            : SaturatingAdd(message.Dropped, (uint)message.Frames.Count);
    }

    private void OnSummaryTick()
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            PresentingProcessesMessage summary = _aggregator.TakeSummary((ulong)_time.GetTimestamp());
            foreach (Subscriber subscriber in _subscribers.Values)
            {
                _ = subscriber.Deliver(summary); // a refused summary is simply lost
            }

            LogStalls();
        }
    }

    /// <summary>Gaps over 1 s in the arrival of rows (SD7), at Debug and at most once per <see cref="StallLogInterval"/>.</summary>
    private void LogStalls()
    {
        int stalls = _aggregator.Stalls;
        DateTimeOffset now = _time.GetUtcNow();
        if (stalls == _loggedStalls || (_lastStallLog is { } last && now - last < StallLogInterval))
        {
            return;
        }

        _log.LogDebug(
            "Frame data arrived after {Count} gaps over 1 s since the last report ({Total} since the start)",
            stalls - _loggedStalls,
            stalls);
        _loggedStalls = stalls;
        _lastStallLog = now;
    }

    private void Unsubscribe(Subscriber subscriber)
    {
        lock (_gate)
        {
            if (_subscribers.TryGetValue(subscriber.Session, out Subscriber? current) && current == subscriber)
            {
                _subscribers.Remove(subscriber.Session);
            }

            if (_subscribers.Count == 0)
            {
                StopTickTimers();
            }
        }
    }

    private void StopTickTimers()
    {
        _batchTimer?.Dispose();
        _summaryTimer?.Dispose();
        _batchTimer = null;
        _summaryTimer = null;
    }

    private static uint SaturatingAdd(uint a, uint b) => a > uint.MaxValue - b ? uint.MaxValue : a + b;

    /// <summary>One subscribed session; its mutable fields are guarded by the hub's lock.</summary>
    private sealed class Subscriber(int session, Func<IMessage, bool> deliver)
    {
        public int Session { get; } = session;

        public bool StatusOwed { get; set; }

        public uint? CarriedPid { get; set; }

        /// <summary>Frames lost in refused batches of <see cref="CarriedPid"/>, added to the next batch's <c>Dropped</c>.</summary>
        public uint CarriedDropped { get; set; }

        /// <summary>Never throws: it runs on timer threads, where an exception would end the process.</summary>
        public bool Deliver(IMessage message)
        {
            try
            {
                return deliver(message);
            }
            catch (Exception)
            {
                return false;
            }
        }
    }

    private sealed class Subscription(FramesHub hub, Subscriber? subscriber) : IDisposable
    {
        private int _disposed;

        public void Dispose()
        {
            if (subscriber is not null && Interlocked.Exchange(ref _disposed, 1) == 0)
            {
                hub.Unsubscribe(subscriber);
            }
        }
    }
}
