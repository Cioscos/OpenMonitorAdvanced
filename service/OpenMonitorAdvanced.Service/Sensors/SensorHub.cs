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
/// applies the D6 gate, resolves each new disk's identity (<see cref="IDiskPowerProbe.Describe"/>),
/// updates each disk that is known to be spinning and publishes an immutable, timestamped cache
/// of raw values keyed by LHM sensor identifier, which the sampler only looks up (values older
/// than two rounds, or from before an idle period, count as absent). A disk enters the schema
/// only once the storage worker has resolved it, so the sampler never waits on disk I/O. The two
/// threads share no lock across hardware I/O: <c>_subLock</c> only guards subscriber bookkeeping,
/// and schema, bindings and snapshot are swapped as one immutable <see cref="Published"/> reference.
/// </para>
/// <para>
/// <b>D6.</b> The tree is opened without storage. <see cref="IHardwareTree.EnableStorage"/> is
/// called only when <see cref="IDiskPowerProbe.AllRotationalDisksActive"/> says every drive that
/// needs it (controller ruling R17) is spinning, re-checked on every storage round until it
/// succeeds; afterwards each disk whose <see cref="DriveFacts.RequiresPowerCheck"/> holds is
/// updated only when <see cref="IDiskPowerProbe.IsSpunDown"/> is <see langword="false"/>,
/// otherwise its values are absent.
/// </para>
/// <para>
/// <b>Lifetime.</b> The tree is never closed while the hub lives: without subscribers sampling
/// stops and the tree stays open (the service exits after two idle minutes, Task 7).
/// <see cref="Dispose"/> stops both threads and waits up to <see cref="WorkerJoinTimeout"/> for
/// each; only if both stopped does it dispose the tree (a worker stuck in a driver call may still
/// be inside LHM, and closing it under that worker is worse than leaving it to process exit).
/// A subscription after <see cref="Dispose"/> is a no-op.
/// </para>
/// </remarks>
public sealed class SensorHub : ISensorFeed, IDisposable
{
    internal static readonly TimeSpan StorageInterval = TimeSpan.FromSeconds(30);
    private static readonly TimeSpan ErrorLogInterval = TimeSpan.FromMinutes(1);
    private static readonly TimeSpan FailureRetryDelay = TimeSpan.FromSeconds(30);

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
    private readonly List<Subscriber> _dueScratch = [];
    private readonly HashSet<string> _failedRoots = new(StringComparer.Ordinal);
    private Plan? _plan;
    private readonly Dictionary<string, (StorageInfo Info, string Id)> _storagePins = new(StringComparer.Ordinal);
    private bool _pawnIo;
    private ulong _seq;

    // Storage-owned state.
    private bool _storageEnabled;
    private bool _storageGateLogged;
    private readonly Dictionary<string, bool?> _diskStates = new(StringComparer.Ordinal);
    private readonly HashSet<string> _undescribedLogged = new(StringComparer.Ordinal);

    // Shared, published by reference swap.
    private volatile Published? _published;
    private volatile StorageCache _storageCache = StorageCache.Empty;
    private volatile IReadOnlyDictionary<string, DiskResolution> _resolvedDisks = new Dictionary<string, DiskResolution>();
    private volatile bool _opened;
    private int _structureDirty;
    private readonly ConcurrentDictionary<string, long> _errorLoggedAt = new(StringComparer.Ordinal);
    private readonly ConcurrentDictionary<string, byte> _notUniqueLogged = new(StringComparer.Ordinal);

    // Workers.
    private readonly CancellationTokenSource _stop = new();
    private readonly ManualResetEventSlim _samplerWake = new();
    private readonly ManualResetEventSlim _storageWake = new();

    public SensorHub(IHardwareTree tree, IDiskPowerProbe disks, Func<bool> pawnIoAvailable, TimeProvider time, ILogger<SensorHub> log)
    {
        _tree = tree;
        _disks = disks;
        _pawnIoAvailable = pawnIoAvailable;
        _time = time;
        _log = log;
        _tree.HardwareChanged += OnHardwareChanged;
    }

    /// <summary>Test seam: <see langword="false"/> keeps the worker threads off, so tests drive the hub deterministically.</summary>
    internal bool StartWorkers { get; init; } = true;

    /// <summary>How long <see cref="Dispose"/> waits for each worker before giving up on it.</summary>
    internal TimeSpan WorkerJoinTimeout { get; init; } = TimeSpan.FromSeconds(10);

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
    public IDisposable Subscribe(uint intervalMs, Action<FeedUpdate> onUpdate)
    {
        ArgumentOutOfRangeException.ThrowIfZero(intervalMs);
        ArgumentNullException.ThrowIfNull(onUpdate);

        Subscriber subscriber;
        lock (_subLock)
        {
            if (_disposed)
            {
                // Shutting down: nothing will ever be delivered, and no thread may start now.
                return NoSubscription.Instance;
            }

            subscriber = new Subscriber(this, ++_nextSubscriberId, MsToTicks(intervalMs), onUpdate)
            {
                NextDue = _time.GetTimestamp(), // the first delivery is due at once (with the schema)
            };
            _subscribers.Add(subscriber);
            RecomputeSamplingLocked();
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

    /// <summary>One sampling tick: opens the tree the first time, updates every non-storage root, publishes a snapshot and delivers it to the due subscribers.</summary>
    internal void TickOnce()
    {
        if (IsDisposed)
        {
            return;
        }

        long tickStart = _time.GetTimestamp();
        ulong tickUnixMs = (ulong)_time.GetUtcNow().ToUnixTimeMilliseconds();
        Plan plan = _plan ?? OpenTree();

        _failedRoots.Clear();
        foreach (HardwareNode root in plan.UpdateRoots)
        {
            try
            {
                _tree.Update(root);
            }
            catch (Exception e)
            {
                _failedRoots.Add(root.Identifier);
                LogRateLimited(root.Identifier, e, "Updating {Root} failed; its sensors are absent from this snapshot", root.Identifier);
            }
        }

        // Rebuilt after the updates, so sensors LHM activates during an Update() are published
        // (with their value) in this very tick. A failed rebuild keeps the current schema and is
        // retried on the next tick.
        if (Interlocked.Exchange(ref _structureDirty, 0) == 1)
        {
            try
            {
                plan = RebuildPlan(plan);
            }
            catch (Exception e)
            {
                Interlocked.Exchange(ref _structureDirty, 1);
                LogRateLimited("schema-rebuild", e, "Rebuilding the schema failed; revision {Revision} stays in use and the rebuild is retried on the next tick", plan.Revision);
            }
        }

        var values = new double?[plan.LhmIds.Length];
        IReadOnlyDictionary<string, double?> storage = FreshStorageValues(tickStart);
        bool anyFailed = _failedRoots.Count > 0;
        for (int i = 0; i < values.Length; i++)
        {
            double? raw;
            if (plan.FromStorage[i])
            {
                raw = storage.TryGetValue(plan.LhmIds[i], out double? cached) ? cached : null;
            }
            else if (anyFailed && plan.Owner[i] is { } owner && _failedRoots.Contains(owner))
            {
                raw = null;
            }
            else
            {
                raw = _tree.Read(plan.LhmIds[i]);
            }

            values[i] = raw is double r && double.IsFinite(r * plan.Scales[i]) ? r * plan.Scales[i] : null;
        }

        long anchor;
        lock (_subLock)
        {
            // Anchored to the schedule when this tick is on time (or late by less than an
            // interval); resynced to the actual start when early (a direct call) or further behind.
            long scheduled = _nextSampleDue;
            bool onSchedule = scheduled != long.MinValue && scheduled <= tickStart && tickStart - scheduled < _minIntervalTicks;
            anchor = onSchedule ? scheduled : tickStart;
            _sampleAnchor = anchor;
            _sampled = true;
            _nextSampleDue = anchor + _minIntervalTicks;
        }

        _seq++;
        _published = new Published(plan.Revision, plan.Built.Schema, new SnapshotMessage(_seq, tickUnixMs, values), anchor);
        DeliverDue(_time.GetTimestamp());
    }

    /// <summary>One wake of the sampler loop: a sampling tick if one is due, otherwise only the due deliveries. Returns the delay until the next wake.</summary>
    internal TimeSpan RunDue()
    {
        if (IsDisposed)
        {
            return Timeout.InfiniteTimeSpan;
        }

        long now = _time.GetTimestamp();
        bool sample;
        lock (_subLock)
        {
            if (_subscribers.Count == 0)
            {
                return Timeout.InfiniteTimeSpan;
            }

            sample = _plan is null || now >= _nextSampleDue;
        }

        if (sample)
        {
            TickOnce();
        }
        else
        {
            DeliverDue(now);
        }

        lock (_subLock)
        {
            if (_subscribers.Count == 0)
            {
                return Timeout.InfiniteTimeSpan;
            }

            long due = _nextSampleDue;
            foreach (Subscriber s in _subscribers)
            {
                due = Math.Min(due, s.NextDue);
            }

            return TicksUntil(due);
        }
    }

    /// <summary>
    /// One storage round: the D6 gate, then for every disk its identity (first touch only), its
    /// power check and its update, then a new timestamped value cache.
    /// </summary>
    internal void StorageOnce()
    {
        if (!_opened || IsDisposed)
        {
            return;
        }

        if (!_storageEnabled && !TryEnableStorage())
        {
            return;
        }

        long roundStart = _time.GetTimestamp();
        IReadOnlyDictionary<string, DiskResolution> previous = _resolvedDisks;
        var resolved = new Dictionary<string, DiskResolution>(StringComparer.Ordinal);
        var cache = new Dictionary<string, double?>(StringComparer.Ordinal);
        IReadOnlyList<HardwareNode> roots = _tree.Roots;
        var identifierCounts = new Dictionary<string, int>(StringComparer.Ordinal);
        foreach (HardwareNode root in roots)
        {
            if (root.Type == HardwareType.Storage)
            {
                identifierCounts[root.Identifier] = identifierCounts.GetValueOrDefault(root.Identifier) + 1;
            }
        }

        foreach (HardwareNode root in roots)
        {
            if (root.Type != HardwareType.Storage)
            {
                continue;
            }

            if (_stop.IsCancellationRequested)
            {
                return;
            }

            if (identifierCounts[root.Identifier] > 1)
            {
                // Its identifier (and so its sensor identifiers, and every identifier-keyed map
                // here) is shared with another disk: never described, updated or published.
                LogNotUnique(root.Identifier);
                continue;
            }

            DiskResolution? resolution = Resolve(root, previous);
            if (resolution is null)
            {
                continue; // identity unknown: not published, not touched, retried next round
            }

            resolved[root.Identifier] = resolution;
            StorageInfo info = resolution.Info;
            if (resolution.Availability == DriveAvailability.NoMedia)
            {
                // LHM enumerated this disk, so "not ready / no media" cannot mean "no platter to
                // wake" (a USB bridge may answer so while its disk sleeps): it cannot be
                // confirmed active, so it is not updated.
                LogDiskStateChange(root.Identifier, info.DriveNumber, spunDown: null);
                continue;
            }

            if (info.Rotational)
            {
                bool? spunDown;
                try
                {
                    spunDown = _disks.IsSpunDown(info.DriveNumber);
                }
                catch (Exception e)
                {
                    LogRateLimited("power:" + root.Identifier, e, "Checking the power mode of {Root} failed", root.Identifier);
                    spunDown = null;
                }

                LogDiskStateChange(root.Identifier, info.DriveNumber, spunDown);
                if (spunDown != false)
                {
                    continue; // standby or unknown: no SMART read, values absent
                }
            }

            try
            {
                _tree.Update(root);
            }
            catch (Exception e)
            {
                LogRateLimited(root.Identifier, e, "Updating {Root} failed; its values are absent until the next storage round", root.Identifier);
                continue;
            }

            // Re-fetched: the update may have activated sensors (a new node for this root).
            CollectValues(FindRoot(root.Identifier) ?? root, cache);
        }

        if (!SameResolution(previous, resolved))
        {
            _resolvedDisks = resolved;
            Interlocked.Exchange(ref _structureDirty, 1);
        }

        // Never mutated after publication: the sampler only looks values up.
        _storageCache = new StorageCache(roundStart, cache);
    }

    /// <summary>One wake of the storage loop: a storage round when due (every 30 s, only with a subscriber, only once open). Returns the delay until the next round.</summary>
    internal TimeSpan RunStorageDue()
    {
        if (IsDisposed)
        {
            return Timeout.InfiniteTimeSpan;
        }

        long now = _time.GetTimestamp();
        lock (_subLock)
        {
            if (!_opened || _subscribers.Count == 0)
            {
                return Timeout.InfiniteTimeSpan;
            }

            if (now < _nextStorageDue)
            {
                return TicksUntil(_nextStorageDue);
            }
        }

        StorageOnce();

        lock (_subLock)
        {
            _nextStorageDue = now + SecondsToTicks(StorageInterval);
            return TicksUntil(_nextStorageDue);
        }
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

    private Plan OpenTree()
    {
        long started = _time.GetTimestamp();
        _tree.Open();
        _pawnIo = _pawnIoAvailable();
        _log.LogInformation("PawnIO available: {PawnIo}", _pawnIo);

        // Changes raised during Open() are covered by the Roots read right after.
        Interlocked.Exchange(ref _structureDirty, 0);
        Plan plan = BuildPlan(_tree.Roots, current: null);
        _plan = plan;
        _log.LogInformation(
            "Hardware tree opened in {Elapsed} ms: {Roots} roots, {Devices} devices, {Sensors} sensors (storage deferred until every rotational disk is active)",
            (long)_time.GetElapsedTime(started).TotalMilliseconds,
            plan.UpdateRoots.Length,
            plan.Built.Schema.Devices.Count,
            plan.Built.Schema.Sensors.Count);

        // Open() leaves tens of MB of garbage (s1-lhm.md §9.6): compact and decommit once.
        GC.Collect(2, GCCollectionMode.Aggressive, blocking: true, compacting: true);

        lock (_subLock)
        {
            _nextStorageDue = _time.GetTimestamp();
            _opened = true;
        }

        SetQuietly(_storageWake);
        return plan;
    }

    private Plan RebuildPlan(Plan current)
    {
        Plan plan = BuildPlan(_tree.Roots, current);
        if (plan.Revision != current.Revision)
        {
            _log.LogInformation(
                "Hardware changed: schema revision {Revision}, {Devices} devices, {Sensors} sensors",
                plan.Revision,
                plan.Built.Schema.Devices.Count,
                plan.Built.Schema.Sensors.Count);
        }

        _plan = plan;
        return plan;
    }

    /// <summary>
    /// The schema covers every non-storage root plus the disks the storage worker has resolved
    /// (with their resolved identity); unresolved disks stay out until then.
    /// </summary>
    private Plan BuildPlan(IReadOnlyList<HardwareNode> roots, Plan? current)
    {
        IReadOnlyDictionary<string, DiskResolution> resolved = _resolvedDisks;
        var schemaRoots = new List<HardwareNode>(roots.Count);
        foreach (HardwareNode root in roots)
        {
            if (root.Type != HardwareType.Storage)
            {
                schemaRoots.Add(root);
            }
            else if (resolved.TryGetValue(root.Identifier, out DiskResolution? resolution))
            {
                schemaRoots.Add(root with { Storage = resolution.Info });
            }
        }

        // A published id stays pinned while its disk keeps the same complete identity, so an
        // identical disk resolved in a later round never changes it (it gets its own id instead).
        var pins = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach ((string rootId, (StorageInfo info, string id)) in _storagePins)
        {
            if (resolved.TryGetValue(rootId, out DiskResolution? resolution) && resolution.Complete && resolution.Info == info)
            {
                pins[rootId] = id;
            }
        }

        BuiltSchema built = SchemaBuilder.Build(schemaRoots, _pawnIo, pins);
        foreach (string skipped in built.SkippedRoots)
        {
            LogNotUnique(skipped);
        }

        _storagePins.Clear();
        foreach ((string rootId, DiskResolution resolution) in resolved)
        {
            if (resolution.Complete && built.StorageDeviceIds.TryGetValue(rootId, out string? id))
            {
                _storagePins[rootId] = (resolution.Info, id);
            }
        }
        if (current is null)
        {
            return new Plan(roots, built, revision: 1);
        }

        return SchemaComparer.SameStructure(current.Built, built)
            ? new Plan(roots, current.Built, current.Revision)
            : new Plan(roots, built, current.Revision + 1);
    }

    /// <summary>
    /// The disk's identity with the descriptor model/serial and the rotational flag, from
    /// <see cref="IDiskPowerProbe.Describe"/>. Only a complete description (present, descriptor
    /// read, seek penalty known) is reused on later rounds while the drive number and serial
    /// stay the same; anything else is described again every round, so a transient answer never
    /// sticks. <see langword="null"/> when it cannot be described (logged once per disk).
    /// </summary>
    private DiskResolution? Resolve(HardwareNode root, IReadOnlyDictionary<string, DiskResolution> previous)
    {
        StorageInfo? fromTree = root.Storage;
        if (fromTree is null || fromTree.DriveNumber < 0)
        {
            return null;
        }

        if (previous.TryGetValue(root.Identifier, out DiskResolution? known)
            && known.Complete
            && known.Info.DriveNumber == fromTree.DriveNumber
            && known.Info.DriveSerial == fromTree.DriveSerial)
        {
            return known;
        }

        DriveFacts? facts;
        try
        {
            facts = _disks.Describe(fromTree.DriveNumber);
        }
        catch (Exception e)
        {
            LogRateLimited("describe:" + root.Identifier, e, "Describing {Root} failed", root.Identifier);
            facts = null;
        }

        if (facts is null)
        {
            if (_undescribedLogged.Add(root.Identifier))
            {
                _log.LogWarning(
                    "{Root} (PhysicalDrive{Drive}) cannot be described; it stays out of the schema and is retried every storage round",
                    root.Identifier,
                    fromTree.DriveNumber);
            }

            return null;
        }

        _undescribedLogged.Remove(root.Identifier);
        bool complete = facts.Availability == DriveAvailability.Present && facts.BusType is not null && facts.SeekPenalty is not null;
        StorageInfo info = fromTree with
        {
            DescriptorModel = facts.Model,
            DescriptorSerial = facts.Serial,
            Rotational = facts.Availability == DriveAvailability.NoMedia || facts.RequiresPowerCheck,
        };
        return new DiskResolution(info, facts.Availability, complete);
    }

    private static bool SameResolution(IReadOnlyDictionary<string, DiskResolution> a, IReadOnlyDictionary<string, DiskResolution> b) =>
        a.Count == b.Count && a.All(pair => b.TryGetValue(pair.Key, out DiskResolution? other) && other == pair.Value);

    private HardwareNode? FindRoot(string identifier)
    {
        foreach (HardwareNode root in _tree.Roots)
        {
            if (root.Identifier == identifier)
            {
                return root;
            }
        }

        return null;
    }

    private IReadOnlyDictionary<string, double?> FreshStorageValues(long now)
    {
        StorageCache cache = _storageCache;
        bool fresh = cache.Timestamp != long.MinValue && now - cache.Timestamp <= 2 * SecondsToTicks(StorageInterval);
        return fresh ? cache.Values : StorageCache.Empty.Values;
    }

    private bool TryEnableStorage()
    {
        bool allActive;
        try
        {
            allActive = _disks.AllRotationalDisksActive();
        }
        catch (Exception e)
        {
            LogRateLimited("storage-gate", e, "Checking the disks' power state failed; storage stays disabled");
            allActive = false;
        }

        if (!allActive)
        {
            if (!_storageGateLogged)
            {
                _storageGateLogged = true;
                _log.LogInformation("Storage stays disabled: a rotational disk is in standby or its state is unknown (re-checked every {Seconds} s)", StorageInterval.TotalSeconds);
            }

            return false;
        }

        try
        {
            _tree.EnableStorage();
        }
        catch (Exception e)
        {
            LogRateLimited("storage-enable", e, "Enabling storage failed");
            return false;
        }

        _storageEnabled = true;
        _log.LogInformation("Every rotational disk is active: storage enabled");
        return true;
    }

    private void CollectValues(HardwareNode node, Dictionary<string, double?> cache)
    {
        foreach (SensorNode sensor in node.Sensors)
        {
            cache[sensor.Identifier] = _tree.Read(sensor.Identifier);
        }

        foreach (HardwareNode child in node.Children)
        {
            CollectValues(child, cache);
        }
    }

    private void LogDiskStateChange(string root, int drive, bool? spunDown)
    {
        if (_diskStates.TryGetValue(root, out bool? previous) && previous == spunDown)
        {
            return;
        }

        _diskStates[root] = spunDown;
        string state = spunDown switch
        {
            true => "in standby: SMART skipped",
            false => "active",
            null => "of unknown power state: SMART skipped",
        };
        _log.LogInformation("{Root} (PhysicalDrive{Drive}) is {State}", root, drive, state);
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
                if (now < s.NextDue)
                {
                    continue;
                }

                // Anchored to the schedule; the first delivery sets the phase from the sample it
                // carries, so a subscriber and the sampling it drives share one wake per interval.
                long next = s.Scheduled ? s.NextDue + s.IntervalTicks : published.SampleAnchor + s.IntervalTicks;
                s.NextDue = next <= now ? now + s.IntervalTicks : next;
                s.Scheduled = true;
                _dueScratch.Add(s);
            }
        }

        foreach (Subscriber s in _dueScratch)
        {
            bool withSchema = s.DeliveredRevision != published.Revision;
            try
            {
                s.OnUpdate(new FeedUpdate(withSchema ? published.Schema : null, published.Snapshot));
                s.DeliveredRevision = published.Revision;
            }
            catch (Exception e)
            {
                LogRateLimited("subscriber:" + s.Id, e, "Subscriber {Subscriber} threw while receiving an update; it stays subscribed", s.Id);
            }
        }

        _dueScratch.Clear();
    }

    private void Unsubscribe(Subscriber subscriber)
    {
        bool idle;
        lock (_subLock)
        {
            if (!_subscribers.Remove(subscriber))
            {
                return;
            }

            RecomputeSamplingLocked();
            idle = _subscribers.Count == 0;
        }

        if (idle)
        {
            // Sampling and storage rounds stop: values read before the idle period must not be
            // published as current when a client comes back.
            _storageCache = StorageCache.Empty;
        }

        SetQuietly(_samplerWake); // recompute the next wake (or sleep until a new subscriber)
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
    private sealed record Published(int Revision, SchemaMessage Schema, SnapshotMessage Snapshot, long SampleAnchor);

    /// <summary>A disk's resolved identity; only a <paramref name="Complete"/> one is reused (and its device id pinned).</summary>
    private sealed record DiskResolution(StorageInfo Info, DriveAvailability Availability, bool Complete);

    /// <summary>Raw storage values of one round, keyed by LHM sensor identifier, with the round's monotonic start.</summary>
    private sealed record StorageCache(long Timestamp, IReadOnlyDictionary<string, double?> Values)
    {
        public static StorageCache Empty { get; } = new(long.MinValue, new Dictionary<string, double?>());
    }

    /// <summary>A schema revision with its per-binding sampling plan (sampler-owned, replaced as a whole).</summary>
    private sealed class Plan
    {
        public Plan(IReadOnlyList<HardwareNode> roots, BuiltSchema built, int revision)
        {
            Built = built;
            Revision = revision;
            UpdateRoots = roots.Where(r => r.Type != HardwareType.Storage).ToArray();

            var owners = new Dictionary<string, (string Root, bool Storage)>(StringComparer.Ordinal);
            foreach (HardwareNode root in roots)
            {
                AddOwners(root, root.Identifier, root.Type == HardwareType.Storage, owners);
            }

            int count = built.Bindings.Count;
            LhmIds = new string[count];
            Scales = new double[count];
            FromStorage = new bool[count];
            Owner = new string?[count];
            for (int i = 0; i < count; i++)
            {
                SensorBinding binding = built.Bindings[i];
                LhmIds[i] = binding.LhmIdentifier;
                Scales[i] = binding.Scale;
                if (owners.TryGetValue(binding.LhmIdentifier, out (string Root, bool Storage) owner))
                {
                    FromStorage[i] = owner.Storage;
                    Owner[i] = owner.Root;
                }
            }

            Debug.Assert(built.Schema.Sensors.Count == count, "bindings are index-aligned with the schema sensors");
        }

        public BuiltSchema Built { get; }

        public int Revision { get; }

        public HardwareNode[] UpdateRoots { get; }

        public string[] LhmIds { get; }

        public double[] Scales { get; }

        /// <summary>Read from the storage worker's cache, never from the tree.</summary>
        public bool[] FromStorage { get; }

        /// <summary>Identifier of the root whose failed update blanks this binding.</summary>
        public string?[] Owner { get; }

        private static void AddOwners(HardwareNode node, string root, bool storage, Dictionary<string, (string, bool)> owners)
        {
            foreach (SensorNode sensor in node.Sensors)
            {
                owners[sensor.Identifier] = (root, storage);
            }

            foreach (HardwareNode child in node.Children)
            {
                AddOwners(child, root, storage, owners);
            }
        }
    }

    private sealed class Subscriber(SensorHub hub, int id, long intervalTicks, Action<FeedUpdate> onUpdate) : IDisposable
    {
        public int Id { get; } = id;

        public long IntervalTicks { get; } = intervalTicks;

        public Action<FeedUpdate> OnUpdate { get; } = onUpdate;

        /// <summary>Monotonic (<see cref="TimeProvider.GetTimestamp"/>) deadline of the next delivery; guarded by <c>_subLock</c>.</summary>
        public long NextDue { get; set; }

        /// <summary>Whether <see cref="NextDue"/> is on the subscriber's schedule yet (after the first delivery); guarded by <c>_subLock</c>.</summary>
        public bool Scheduled { get; set; }

        /// <summary>Schema revision last delivered (0 = none yet); sampler-owned.</summary>
        public int DeliveredRevision { get; set; }

        public void Dispose() => hub.Unsubscribe(this);
    }

    private sealed class NoSubscription : IDisposable
    {
        public static NoSubscription Instance { get; } = new();

        public void Dispose()
        {
        }
    }
}

/// <summary>
/// Structural comparison of two built schemas: devices and sensors in order with every field,
/// device properties as an unordered (key-sorted) set, and the bindings. Record equality alone
/// would compare the <see cref="IReadOnlyList{T}"/>/<see cref="IReadOnlyDictionary{TKey,TValue}"/>
/// members by reference.
/// </summary>
internal static class SchemaComparer
{
    public static bool SameStructure(BuiltSchema a, BuiltSchema b)
    {
        if (a.Schema.Devices.Count != b.Schema.Devices.Count || !a.Schema.Sensors.SequenceEqual(b.Schema.Sensors) || !a.Bindings.SequenceEqual(b.Bindings))
        {
            return false;
        }

        for (int i = 0; i < a.Schema.Devices.Count; i++)
        {
            WireDevice x = a.Schema.Devices[i];
            WireDevice y = b.Schema.Devices[i];
            if (x.Id != y.Id || x.Kind != y.Kind || x.Name != y.Name || x.Vendor != y.Vendor || !Equals(x.Hint, y.Hint) || !SameProperties(x.Properties, y.Properties))
            {
                return false;
            }
        }

        return true;
    }

    private static bool SameProperties(IReadOnlyDictionary<string, string> x, IReadOnlyDictionary<string, string> y) =>
        x.Count == y.Count
        && x.OrderBy(p => p.Key, StringComparer.Ordinal).SequenceEqual(y.OrderBy(p => p.Key, StringComparer.Ordinal));
}
