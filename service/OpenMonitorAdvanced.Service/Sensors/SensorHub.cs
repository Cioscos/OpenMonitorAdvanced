using System.Collections.Concurrent;
using System.Diagnostics;
using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// Samples an <see cref="IHardwareTree"/> and delivers <see cref="FeedUpdate"/>s to subscribers,
/// each at its own interval.
/// </summary>
/// <remarks>
/// <para>
/// <b>Threads.</b> Two dedicated MTA threads, started on the first subscription.
/// <c>oma-sampler</c> opens the tree (≈ 4.5 s with LHM), owns the schema, updates every
/// non-storage root once per <c>min(subscriber intervals)</c>, publishes one snapshot per tick and
/// delivers it to the subscribers that are due; between sampling ticks it also wakes for a
/// subscriber's delivery deadline, reusing the latest snapshot instead of running LHM again.
/// Sampling and delivery deadlines are anchored to their schedule (<c>+= interval</c>, resynced
/// only when more than one interval behind), and a subscriber's phase starts from the sample it
/// first receives, so one subscriber costs one wake per interval.
/// <c>oma-storage</c> owns every disk access: every 30 s (only while someone is subscribed) it
/// applies the D6 gate, lists the drives, resolves each new disk's identity
/// (<see cref="IDiskPowerProbe.Describe"/>), updates each disk that is known to be spinning and
/// publishes the outcome as one immutable <see cref="StorageRound"/>; at the end of each
/// round's disk work it reads the disks' counters once more as the reference the next round
/// compares with (<see cref="ActivityWatch"/>, <c>TakeReference</c>), so it wakes only once per
/// round. A round is: the state of every drive,
/// the resolved disks, the storage part of the request it applied and the raw values keyed by LHM
/// sensor identifier, which the sampler only looks up (values older than two rounds, or from
/// before an idle period, count as absent). A
/// disk enters the schema only once the storage worker has resolved it, so the sampler never
/// waits on disk I/O. The two threads share no lock across hardware I/O: <c>_subLock</c> only
/// guards subscriber bookkeeping; a storage round is one reference swap at its end, the sampler
/// takes one round per tick and uses it for the service block, the schema and the values alike,
/// and schema, bindings and snapshot are swapped as one immutable <see cref="Published"/> reference.
/// </para>
/// <para>
/// <b>D6.</b> The tree is opened without storage. <see cref="IHardwareTree.EnableStorage"/> is
/// called only when a <see cref="GateEpisode"/> round says every drive that needs it (controller
/// ruling R17) is spinning, tried on every storage round until it succeeds (the drives that
/// block are flagged in the schema's <c>drives</c>). Afterwards nothing blocks: every round
/// lists the drives again (<see cref="IDiskPowerProbe.Enumerate"/>) and checks each one whose
/// <see cref="DriveFacts.RequiresPowerCheck"/> holds and whose SMART is on
/// (<see cref="DriveStates.IsSmartOff"/>), whether LHM exposes it or not; a disk is updated
/// only when it answers "active".
/// </para>
/// <para>
/// <b>Asking costs</b> (design M6b §4.4). CHECK POWER MODE resets Windows' idle timer of the
/// disk and powers up a disk Windows turned off, so a disk is asked (and then read) only when
/// Windows reports it on and its read/write counters grew since the end of the round before
/// (the reference is read after that round's own questions and SMART reads, so the window is
/// the whole interval between two rounds and never holds the service's traffic);
/// otherwise nothing is sent to it: it is <c>standby</c> when Windows turned it off and
/// <c>idle</c> when it is on without recent activity (which may hide a standby the disk chose
/// itself). The closed gate follows the same idea (<see cref="GateEpisode"/>). Two things
/// stand in for counters that grew. A disk Windows reports on is asked once, without
/// activity, the first time it is watched, so a spinning but quiet disk shows its values: in
/// the round that opens the gate (which goes on with the gate's answers), after storage is
/// switched on again, after the hub was idle, when its SMART is switched on and when it is
/// newly listed; that once is spent when the drives are listed, before the question, so a
/// round that fails cannot repeat it. And a disk Windows reported off and now reports on is
/// asked in that round.
/// </para>
/// <para>
/// <b>Held values</b> (protocol v3, <see cref="SnapshotMessage.Held"/>). A disk that rests
/// (confirmed standby, turned off by Windows, or idle) keeps the values of the round before,
/// flagged as held, round after round while it rests: only for the disk they were measured on
/// (same <see cref="DriveKey"/> in the earlier round, in the resolved disk and in this round's
/// enumeration), and only from a round that is itself still current: one that started within
/// the two-round limit above and that nothing replaced while this round ran (the idle drop).
/// So a late round loses the value until the disk is read again, instead of bringing an expired
/// one back. An unknown state, "no media", a failed update or SMART off keep nothing. In a
/// snapshot a storage value is held when its round kept it, or when that round's values went
/// out in an earlier snapshot already (30 s rounds against faster sampling); an absent value
/// and a value from another source never are.
/// </para>
/// <para>
/// <b>Lifetime.</b> The tree is never closed while the hub lives: without subscribers sampling
/// stops and the tree stays open (the service exits after two idle minutes, Task 7).
/// <see cref="Dispose"/> stops both threads and waits up to <see cref="WorkerJoinTimeout"/> for
/// each; only if both stopped does it dispose the tree (a worker stuck in a driver call may still
/// be inside LHM, and closing it under that worker is worse than leaving it to process exit).
/// A subscription after <see cref="Dispose"/> is a no-op.
/// </para>
/// <para>
/// <b>Requests (spec M5 §2.8).</b> Every subscriber holds a <see cref="FeedRequest"/>.
/// <see cref="Subscribe"/>, <see cref="IFeedSubscription.Update"/> and unsubscribing recompute the
/// sampling interval and the <see cref="EffectiveConfig"/> in one <c>_subLock</c> section, so a
/// client replacing its request never passes through "no subscribers". A changed aggregate is
/// published as an immutable <see cref="DesiredConfig"/> with a growing version; without
/// subscribers the last one stays. These calls come from pipe sessions and never touch the tree:
/// the sampler folds the desired configuration into the schema's <c>service</c> block, which
/// says <c>pending</c> while it differs from the applied one. A subscriber's deliveries wait until
/// the published block reflects the version its own request produced, so the schema forced after
/// a request never shows the state from before it; the sample taken at once for a new request
/// keeps the sampling schedule.
/// </para>
/// <para>
/// <b>Applying a request</b> (F2.3, P7, P10), each part on the thread that owns it:
/// <list type="bullet">
/// <item>the schema, at once on the sampler: the roots of switched-off modules and the disks whose
/// drive key has SMART off leave the plan before that tick's updates, so its snapshot already
/// uses the new schema, in the same revision;</item>
/// <item>the LHM groups other than storage, on the sampler (<see cref="ModuleApplier"/>): opened
/// with the requested ones only, later switched with <see cref="IHardwareTree.SetModules"/> once
/// the storage worker has parked at the boundary of its loop (<see cref="StoragePark"/>), without
/// the sampler ever waiting for it; after <see cref="ReconfigureTimeout"/> the request is
/// <c>failed</c> and its park asked again on every tick, never forced; setters that throw are
/// <c>failed</c> too and tried again only after <see cref="FailureRetryDelay"/>;</item>
/// <item>storage, "softly" on the storage worker: switched off, its values and resolved disks are
/// dropped, the last drive list stays with every entry <c>smartOff</c>, and no gate, enumeration,
/// description, power check or update runs, but the LHM group stays open; a disk with SMART off
/// (by request, or a USB disk nobody switched on) is still described (access 0) and then neither
/// power-checked nor updated. It still counts for the D6 gate (verdict c).</item>
/// </list>
/// The service block reports the groups actually open, the storage part the tick's storage round
/// was made with (so a request shows as applied together with its drive list, never before),
/// the request's status and the drive list. When a schema rebuild fails the current plan gets
/// the new block anyway (<c>failed</c> if the request's schema could not be built), so no client
/// waits on a request that cannot be shown.
/// </para>
/// </remarks>
public sealed partial class SensorHub : ISensorFeed, IDisposable
{
    internal static readonly TimeSpan StorageInterval = TimeSpan.FromSeconds(30);
    private static readonly TimeSpan ErrorLogInterval = TimeSpan.FromMinutes(1);
    internal static readonly TimeSpan FailureRetryDelay = TimeSpan.FromSeconds(30);

    private readonly IHardwareTree _tree;
    private readonly IDiskPowerProbe _disks;
    private readonly Func<bool> _pawnIoAvailable;
    private readonly TimeProvider _time;
    private readonly ILogger<SensorHub> _log;

    // Subscriber bookkeeping (any thread), guarded by _subLock. Never held across hardware I/O
    // or a subscriber callback.
    private readonly object _subLock = new();
    private readonly List<Subscriber> _subscribers = [];
    private long _minIntervalTicks;
    private long _sampleAnchor;
    private bool _sampled;
    private long _nextSampleDue = long.MinValue;
    private long _nextStorageDue;
    private int _nextSubscriberId;
    private Thread? _samplerThread;
    private Thread? _storageThread;
    private bool _disposed;

    // Sampler-owned state.
    private readonly List<(Subscriber Subscriber, bool ForcedSchema)> _dueScratch = [];
    private readonly ModuleApplier _applier;
    private DesiredConfig? _servedDesired;
    private EffectiveConfig _schemaFilter = EffectiveConfig.AllOn; // the served request's schema effect
    private EffectiveConfig _servedStoragePart = EffectiveConfig.AllOn.StoragePart;
    private ReconfigurationStatus _status = ReconfigurationStatus.Applied;
    private bool _rebuildFailing;
    private StorageRound _view = StorageRound.Empty; // the storage round the current tick works with, taken once at its start
    private (ServiceModules Groups, EffectiveConfig Storage, IReadOnlyList<WireDrive> Drives, ReconfigurationStatus Status)? _stateInputs;
    private ServiceStateBlock _serviceState = ServiceStateBlock.AllActive;
    private long _reflectedVersion; // the _desired version the published service block reflects
    private readonly HashSet<string> _failedRoots = new(StringComparer.Ordinal);
    private Plan? _plan;
    private readonly Dictionary<string, (DiskResolution Disk, string Id)> _storagePins = new(StringComparer.Ordinal);
    private bool _pawnIo;
    private ulong _seq;
    private long _publishedGeneration; // the storage round whose values the latest snapshot carried

    // Storage-owned state.
    private long _storageSyncedVersion;
    private EffectiveConfig _storageApplied = EffectiveConfig.AllOn.StoragePart; // reaches the sampler inside a StorageRound
    private bool _storageEnabled;
    private bool _storageGateLogged;
    private readonly Dictionary<string, bool?> _diskStates = new(StringComparer.Ordinal);
    private readonly HashSet<string> _undescribedLogged = new(StringComparer.Ordinal);
    private readonly ActivityWatch _watch;
    private GateEpisode? _gate; // while the D6 gate is closed
    private bool _lateRound; // the storage round before this one found its own predecessor expired
    private long _rearmedAt; // when a late round last re-armed the first-watch questions

    // Shared, published by reference swap.
    private volatile DesiredConfig? _desired; // written under _subLock
    private volatile Published? _published;
    /// <summary>
    /// Critical Warning bits that make the flag 1: available spare, reliability, read-only,
    /// volatile backup and PMR (NVMe base spec 2.0). Bit 1 (temperature) is transient and stays
    /// with the temperature rules; bits 6-7 are reserved.
    /// </summary>
    private const int CriticalWarningMask = 0x3D;

    private volatile StorageRound _round = StorageRound.Empty; // replaced as a whole (PublishRound)
    private volatile bool _opened;
    private readonly StoragePark _park = new();
    private int _structureDirty;
    private int _wentIdle; // set when the last client leaves, taken by the storage worker's next round
    private readonly ConcurrentDictionary<string, long> _errorLoggedAt = new(StringComparer.Ordinal);
    private readonly ConcurrentDictionary<string, byte> _notUniqueLogged = new(StringComparer.Ordinal);

    // Workers.
    private readonly CancellationTokenSource _stop = new();
    private readonly ManualResetEventSlim _samplerWake = new();
    private readonly ManualResetEventSlim _storageWake = new();

    public SensorHub(IHardwareTree tree, IDiskPowerProbe disks, IDiskActivityProbe activity, Func<bool> pawnIoAvailable, TimeProvider time, ILogger<SensorHub> log)
    {
        _tree = tree;
        _disks = disks;
        _watch = new ActivityWatch(activity, time);
        _pawnIoAvailable = pawnIoAvailable;
        _time = time;
        _log = log;
        _applier = new ModuleApplier(tree, _park, time, log, () => SetQuietly(_storageWake));
        _tree.HardwareChanged += OnHardwareChanged;
    }

    /// <summary>Test seam: <see langword="false"/> keeps the worker threads off, so tests drive the hub deterministically.</summary>
    internal bool StartWorkers { get; init; } = true;

    /// <summary>How long <see cref="Dispose"/> waits for each worker before giving up on it.</summary>
    internal TimeSpan WorkerJoinTimeout { get; init; } = TimeSpan.FromSeconds(10);

    /// <summary>How long a request may stay <c>pending</c> before it is reported <c>failed</c> (it is still retried).</summary>
    internal TimeSpan ReconfigureTimeout
    {
        get => _applier.Timeout;
        init => _applier.Timeout = value;
    }

    internal bool AnyWorkerAlive
    {
        get
        {
            lock (_subLock)
            {
                return (_samplerThread?.IsAlive ?? false) || (_storageThread?.IsAlive ?? false);
            }
        }
    }

    /// <summary>Current schema revision: 1 after opening, +1 on every structural change; 0 before opening.</summary>
    internal int Revision => _published?.Revision ?? 0;

    /// <inheritdoc />
    /// <summary>The configuration the subscribers' requests add up to; <see langword="null"/> before the first subscription.</summary>
    internal DesiredConfig? Desired => _desired;

    /// <summary>Whether the storage worker acknowledged a park the sampler has not released yet.</summary>
    internal bool IsStorageParked => _park.IsParked;

    /// <inheritdoc />
    public IFeedSubscription Subscribe(FeedRequest request, Action<FeedUpdate> onUpdate)
    {
        ArgumentNullException.ThrowIfNull(request);
        ArgumentOutOfRangeException.ThrowIfZero(request.IntervalMs);
        ArgumentNullException.ThrowIfNull(onUpdate);

        Subscriber subscriber;
        lock (_subLock)
        {
            if (_disposed)
            {
                // Shutting down: nothing will ever be delivered, and no thread may start now.
                return NoSubscription.Instance;
            }

            subscriber = new Subscriber(this, ++_nextSubscriberId, onUpdate)
            {
                Request = request,
                IntervalTicks = MsToTicks(request.IntervalMs),
                NextDue = _time.GetTimestamp(), // the first delivery is due at once (with the schema)
            };
            _subscribers.Add(subscriber);
            RecomputeLocked();
            subscriber.RequestVersion = _desired?.Version ?? 0;
            if (_subscribers.Count == 1 && _opened)
            {
                // Idle -> active: a storage round right away, so a quick reconnect does not wait
                // up to 30 s for disk values (the cache was dropped when the last client left).
                _nextStorageDue = _time.GetTimestamp();
            }

            // Started under the lock that Dispose takes first, so no thread can start after it.
            if (StartWorkers && _samplerThread is null)
            {
                _samplerThread = new Thread(SamplerLoop) { Name = "oma-sampler", IsBackground = true };
                _storageThread = new Thread(StorageLoop) { Name = "oma-storage", IsBackground = true };
                _samplerThread.Start();
                _storageThread.Start();
            }
        }

        SetQuietly(_samplerWake);
        SetQuietly(_storageWake);
        return subscriber;
    }

    /// <summary>
    /// Stops both workers and waits up to <see cref="WorkerJoinTimeout"/> for each. If both
    /// stopped, disposes the tree (LHM <c>Close()</c> only); otherwise leaves it open,
    /// since a stuck worker may still be inside it.
    /// </summary>
    public void Dispose()
    {
        Thread? sampler;
        Thread? storage;
        lock (_subLock)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            sampler = _samplerThread;
            storage = _storageThread;
        }

        _stop.Cancel();
        SetQuietly(_samplerWake);
        SetQuietly(_storageWake);

        bool allStopped = true;
        foreach (Thread? worker in new[] { sampler, storage })
        {
            // Dispose() from a subscriber callback runs on oma-sampler itself; that thread only
            // finishes delivering and then leaves its loop, without touching the tree again.
            if (worker is null || worker == Thread.CurrentThread || worker.Join(WorkerJoinTimeout))
            {
                continue;
            }

            allStopped = false;
            _log.LogWarning(
                "The {Worker} thread did not stop within {Seconds} s; the hardware tree is left open (released at process exit)",
                worker.Name,
                WorkerJoinTimeout.TotalSeconds);
        }

        _tree.HardwareChanged -= OnHardwareChanged;
        if (!allStopped)
        {
            return; // the wake events and the token stay alive for the stuck worker
        }

        try
        {
            _tree.Dispose();
        }
        catch (Exception e)
        {
            _log.LogError(e, "Closing the hardware tree failed");
        }
    }

    private bool IsDisposed
    {
        get
        {
            lock (_subLock)
            {
                return _disposed;
            }
        }
    }

    private void DeliverDue(long now)
    {
        Published? published = _published;
        if (published is null)
        {
            return;
        }

        lock (_subLock)
        {
            foreach (Subscriber s in _subscribers)
            {
                if (now < s.NextDue || s.RequestVersion > published.ReflectedVersion)
                {
                    continue; // not due, or its request is not in this schema yet: stays due
                }

                // Anchored to the schedule; the first delivery sets the phase from the sample it
                // carries, so a subscriber and the sampling it drives share one wake per interval.
                long next = s.Scheduled ? s.NextDue + s.IntervalTicks : published.SampleAnchor + s.IntervalTicks;
                s.NextDue = next <= now ? now + s.IntervalTicks : next;
                s.Scheduled = true;
                _dueScratch.Add((s, s.ForceSchema));
                s.ForceSchema = false;
            }
        }

        foreach ((Subscriber s, bool forced) in _dueScratch)
        {
            bool withSchema = forced || s.DeliveredRevision != published.Revision;
            try
            {
                s.OnUpdate(new FeedUpdate(withSchema ? published.Schema : null, published.Snapshot));
                s.DeliveredRevision = published.Revision;
            }
            catch (Exception e)
            {
                if (forced)
                {
                    s.DeliveredRevision = 0; // the forced schema did not arrive: send it next time
                }

                LogRateLimited("subscriber:" + s.Id, e, "Subscriber {Subscriber} threw while receiving an update; it stays subscribed", s.Id);
            }
        }

        _dueScratch.Clear();
    }

    /// <summary>
    /// Replaces <paramref name="subscriber"/>'s request in place (<see cref="IFeedSubscription.Update"/>).
    /// Like a new subscription, the next delivery is due at once, carries the schema and sets the
    /// phase of the new interval; unlike one, the subscriber count never drops, so the storage
    /// cache and the storage schedule survive and the configuration does not oscillate.
    /// </summary>
    private void Update(Subscriber subscriber, FeedRequest request)
    {
        ArgumentNullException.ThrowIfNull(request);
        ArgumentOutOfRangeException.ThrowIfZero(request.IntervalMs);

        bool configChanged;
        lock (_subLock)
        {
            if (_disposed || !_subscribers.Contains(subscriber))
            {
                return; // unsubscribed (or shutting down): nothing to replace
            }

            subscriber.Request = request;
            subscriber.IntervalTicks = MsToTicks(request.IntervalMs);
            subscriber.NextDue = _time.GetTimestamp();
            subscriber.Scheduled = false;
            subscriber.ForceSchema = true;
            configChanged = RecomputeLocked();
            subscriber.RequestVersion = _desired?.Version ?? 0;
        }

        SetQuietly(_samplerWake);
        if (configChanged)
        {
            SetQuietly(_storageWake);
        }
    }

    private void Unsubscribe(Subscriber subscriber)
    {
        bool idle;
        bool configChanged;
        lock (_subLock)
        {
            if (!_subscribers.Remove(subscriber))
            {
                return;
            }

            configChanged = RecomputeLocked();
            idle = _subscribers.Count == 0;
        }

        if (idle)
        {
            // Sampling and storage rounds stop: values read before the idle period must not be
            // published as current when a client comes back. The drives and the disks stay.
            Interlocked.Exchange(ref _wentIdle, 1);
            PublishRound(long.MinValue, drives: null, resolved: null, StorageRound.Empty.Values, StorageRound.Empty.Held, applied: null);
        }

        SetQuietly(_samplerWake); // recompute the next wake (or sleep until a new subscriber)
        if (configChanged)
        {
            SetQuietly(_storageWake);
        }
    }

    /// <summary>
    /// Recomputes the sampling interval and the effective configuration together; publishes a new
    /// <see cref="DesiredConfig"/> when the configuration changed (never without subscribers).
    /// Returns whether it did. Caller holds <c>_subLock</c>.
    /// </summary>
    private bool RecomputeLocked()
    {
        RecomputeSamplingLocked();

        var requests = new FeedRequest[_subscribers.Count];
        for (int i = 0; i < requests.Length; i++)
        {
            requests[i] = _subscribers[i].Request;
        }

        EffectiveConfig? config = EffectiveConfig.Compute(requests);
        DesiredConfig? current = _desired;
        if (config is null || (current is not null && current.Config.Equals(config)))
        {
            return false;
        }

        _desired = new DesiredConfig((current?.Version ?? 0) + 1, config, _time.GetTimestamp());
        return true;
    }

    /// <summary>
    /// Sampler, at the start of every <see cref="RunDue"/>: serves a new <c>_desired</c> (its schema
    /// filter takes effect in this tick), lets the <see cref="ModuleApplier"/> step the LHM groups
    /// and refreshes the service block. A changed block is a structural change: the tick rebuilds
    /// the plan before its updates, with a new revision.
    /// </summary>
    private void ApplyDesired()
    {
        DesiredConfig? desired = _desired;
        if (!ReferenceEquals(desired, _servedDesired))
        {
            _servedDesired = desired;
            _schemaFilter = desired?.Config ?? EffectiveConfig.AllOn;
            _servedStoragePart = _schemaFilter.StoragePart;
        }

        _status = desired is null
            ? ReconfigurationStatus.Applied
            : _applier.Step(desired, _view.Applied.Equals(_servedStoragePart));
        UpdateServiceState();
    }

    /// <summary>
    /// Sampler: the service block from what is actually applied: the groups open in the tree,
    /// the storage part and the drive list of the tick's storage round (one reference, so the two
    /// never disagree) and the request's status (<c>failed</c> too while the request's schema
    /// cannot be built).
    /// </summary>
    private void UpdateServiceState()
    {
        ReconfigurationStatus status = _rebuildFailing && _plan is { } plan && !plan.Filter.Equals(_schemaFilter)
            ? ReconfigurationStatus.Failed
            : _status;
        EffectiveConfig storage = _view.Applied;
        (ServiceModules, EffectiveConfig, IReadOnlyList<WireDrive>, ReconfigurationStatus) inputs = (_applier.Groups, storage, _view.Drives, status);
        if (_stateInputs is { } last && last.Groups == inputs.Item1 && last.Storage.Equals(inputs.Item2) && ReferenceEquals(last.Drives, inputs.Item3) && last.Status == inputs.Item4)
        {
            return;
        }

        _stateInputs = inputs;
        var state = new ServiceStateBlock(
            ServiceModuleNames.ToWire(_applier.Groups | (storage.Enabled & ServiceModules.Storage)),
            [.. storage.SmartDisabledDrives.Order(StringComparer.Ordinal)],
            status switch
            {
                ReconfigurationStatus.Applied => "applied",
                ReconfigurationStatus.Pending => "pending",
                _ => "failed",
            },
            inputs.Item3);
        if (!SchemaComparer.SameServiceState(state, _serviceState))
        {
            _serviceState = state;
        }
    }

    private void RecomputeSamplingLocked()
    {
        long min = long.MaxValue;
        foreach (Subscriber s in _subscribers)
        {
            min = Math.Min(min, s.IntervalTicks);
        }

        _minIntervalTicks = _subscribers.Count == 0 ? 0 : min;
        _nextSampleDue = _sampled ? _sampleAnchor + _minIntervalTicks : long.MinValue;
    }

    private void OnHardwareChanged() => Interlocked.Exchange(ref _structureDirty, 1);

    /// <summary>Once per identifier for the hub's lifetime (either worker may report it).</summary>
    private void LogNotUnique(string identifier)
    {
        if (_notUniqueLogged.TryAdd(identifier, 0))
        {
            _log.LogWarning("{Root} is not published: its LHM identifier or device id is not unique, so its sensors cannot be told apart", identifier);
        }
    }

    private void LogRateLimited(string key, Exception e, string message, params object?[] args)
    {
        long now = _time.GetTimestamp();
        if (_errorLoggedAt.TryGetValue(key, out long last) && now - last < SecondsToTicks(ErrorLogInterval))
        {
            return;
        }

        _errorLoggedAt[key] = now;
#pragma warning disable CA2254 // the templates are constants passed through from the call sites above
        _log.LogWarning(e, message, args);
#pragma warning restore CA2254
    }

    private long MsToTicks(uint ms) => (long)(ms * (double)_time.TimestampFrequency / 1000d);

    private long SecondsToTicks(TimeSpan span) => (long)(span.TotalSeconds * _time.TimestampFrequency);

    private TimeSpan TicksUntil(long due)
    {
        long now = _time.GetTimestamp();
        return due <= now ? TimeSpan.Zero : TimeSpan.FromSeconds((due - now) / (double)_time.TimestampFrequency);
    }

    private static void SetQuietly(ManualResetEventSlim wake)
    {
        try
        {
            wake.Set();
        }
        catch (ObjectDisposedException)
        {
            // Nothing left to wake.
        }
    }

    private void SamplerLoop() => RunLoop(_samplerWake, RunDue, "sampler");

    private void StorageLoop() => RunLoop(_storageWake, RunStorageDue, "storage");

    private void RunLoop(ManualResetEventSlim wake, Func<TimeSpan> step, string name)
    {
        using ITimer timer = _time.CreateTimer(static w => SetQuietly((ManualResetEventSlim)w!), wake, Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
        CancellationToken stop = _stop.Token;
        while (!stop.IsCancellationRequested)
        {
            wake.Reset();
            TimeSpan delay;
            try
            {
                delay = step();
            }
            catch (Exception e)
            {
                LogRateLimited("loop:" + name, e, "The {Worker} loop failed; retrying", name);
                delay = FailureRetryDelay;
            }

            if (delay != Timeout.InfiniteTimeSpan)
            {
                if (delay == TimeSpan.Zero)
                {
                    continue;
                }

                timer.Change(delay, Timeout.InfiniteTimeSpan);
            }

            try
            {
                wake.Wait(stop);
            }
            catch (OperationCanceledException)
            {
                break; // Dispose()
            }
        }
    }

    /// <summary>
    /// Schema, bindings and snapshot published together, so a snapshot always matches the schema
    /// sent with it; <paramref name="SampleAnchor"/> is the scheduled (monotonic) time of the sample.
    /// </summary>
    /// <remarks><paramref name="ReflectedVersion"/> is the <c>_desired</c> version the schema's service block reflects.</remarks>
    private sealed record Published(int Revision, SchemaMessage Schema, SnapshotMessage Snapshot, long SampleAnchor, long ReflectedVersion);

    /// <summary>
    /// The configuration the subscribers ask for, replaced as a whole: <paramref name="Version"/>
    /// grows by one per change, <paramref name="RequestedAt"/> is the monotonic time of the change.
    /// </summary>
    internal sealed record DesiredConfig(long Version, EffectiveConfig Config, long RequestedAt);

    private sealed class Subscriber(SensorHub hub, int id, Action<FeedUpdate> onUpdate) : IFeedSubscription
    {
        public int Id { get; } = id;

        /// <summary>This subscriber's current request; guarded by <c>_subLock</c>.</summary>
        public required FeedRequest Request { get; set; }

        /// <summary>Guarded by <c>_subLock</c>.</summary>
        public long IntervalTicks { get; set; }

        /// <summary>The next delivery carries the schema whatever the revision (set by <see cref="Update"/>); guarded by <c>_subLock</c>.</summary>
        public bool ForceSchema { get; set; }

        /// <summary>The <c>_desired</c> version after this subscriber's latest request: nothing is delivered from an older state; guarded by <c>_subLock</c>.</summary>
        public long RequestVersion { get; set; }

        public Action<FeedUpdate> OnUpdate { get; } = onUpdate;

        /// <summary>Monotonic (<see cref="TimeProvider.GetTimestamp"/>) deadline of the next delivery; guarded by <c>_subLock</c>.</summary>
        public long NextDue { get; set; }

        /// <summary>Whether <see cref="NextDue"/> is on the subscriber's schedule yet (after the first delivery); guarded by <c>_subLock</c>.</summary>
        public bool Scheduled { get; set; }

        /// <summary>Schema revision last delivered (0 = none yet); sampler-owned.</summary>
        public int DeliveredRevision { get; set; }

        public void Update(FeedRequest request) => hub.Update(this, request);

        public void Dispose() => hub.Unsubscribe(this);
    }

    private sealed class NoSubscription : IFeedSubscription
    {
        public static NoSubscription Instance { get; } = new();

        public void Update(FeedRequest request)
        {
        }

        public void Dispose()
        {
        }
    }
}
