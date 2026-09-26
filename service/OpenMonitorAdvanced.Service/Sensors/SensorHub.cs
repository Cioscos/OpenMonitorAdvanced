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
/// <c>oma-storage</c> owns every storage read: every 30 s (only while someone is subscribed) it
/// applies the D6 gate, updates each disk that is known to be spinning and publishes an immutable
/// cache of raw values keyed by LHM sensor identifier, which the sampler only looks up. The two
/// threads share no lock across hardware I/O: <c>_subLock</c> only guards subscriber bookkeeping,
/// and schema, bindings and snapshot are swapped as one immutable <see cref="Published"/> reference.
/// </para>
/// <para>
/// <b>D6.</b> The tree is opened without storage. <see cref="IHardwareTree.EnableStorage"/> is
/// called only when <see cref="IDiskPowerProbe.AllRotationalDisksActive"/> says every rotational
/// (or unknown) disk is spinning, re-checked on every storage round until it succeeds; afterwards
/// each rotational disk is updated only when <see cref="IDiskPowerProbe.IsSpunDown"/> is
/// <see langword="false"/>, otherwise its values are absent.
/// </para>
/// <para>
/// <b>Lifetime.</b> The tree is never closed while the hub lives: without subscribers sampling
/// stops and the tree stays open (the service exits after two idle minutes, Task 7).
/// <see cref="Dispose"/> stops and joins both threads, then disposes the tree.
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
    private long _lastSampleStart;
    private bool _sampled;
    private long _nextSampleDue;
    private long _nextStorageDue;
    private int _nextSubscriberId;

    // Sampler-owned state.
    private readonly List<Subscriber> _dueScratch = [];
    private readonly HashSet<string> _failedRoots = new(StringComparer.Ordinal);
    private Plan? _plan;
    private bool _pawnIo;
    private ulong _seq;

    // Storage-owned state.
    private bool _storageEnabled;
    private bool _storageGateLogged;
    private readonly Dictionary<string, bool?> _diskStates = new(StringComparer.Ordinal);

    // Shared, published by reference swap.
    private volatile Published? _published;
    private volatile IReadOnlyDictionary<string, double?> _storageValues = new Dictionary<string, double?>();
    private volatile bool _opened;
    private int _structureDirty;
    private readonly ConcurrentDictionary<string, long> _errorLoggedAt = new(StringComparer.Ordinal);

    // Workers.
    private readonly CancellationTokenSource _stop = new();
    private readonly ManualResetEventSlim _samplerWake = new();
    private readonly ManualResetEventSlim _storageWake = new();
    private Thread? _samplerThread;
    private Thread? _storageThread;
    private int _workersStarted;
    private int _disposed;

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

    /// <summary>Current schema revision: 1 after opening, +1 on every structural change; 0 before opening.</summary>
    internal int Revision => _published?.Revision ?? 0;

    /// <inheritdoc />
    public IDisposable Subscribe(uint intervalMs, Action<FeedUpdate> onUpdate)
    {
        ArgumentOutOfRangeException.ThrowIfZero(intervalMs);
        ArgumentNullException.ThrowIfNull(onUpdate);
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed) == 1, this);

        Subscriber subscriber;
        lock (_subLock)
        {
            subscriber = new Subscriber(this, Interlocked.Increment(ref _nextSubscriberId), MsToTicks(intervalMs), onUpdate)
            {
                NextDue = _time.GetTimestamp(), // the first delivery is due at once (with the schema)
            };
            _subscribers.Add(subscriber);
            RecomputeSamplingLocked();
        }

        if (StartWorkers && Interlocked.Exchange(ref _workersStarted, 1) == 0)
        {
            _samplerThread = new Thread(SamplerLoop) { Name = "oma-sampler", IsBackground = true };
            _storageThread = new Thread(StorageLoop) { Name = "oma-storage", IsBackground = true };
            _samplerThread.Start();
            _storageThread.Start();
        }

        _samplerWake.Set();
        _storageWake.Set();
        return subscriber;
    }

    /// <summary>One sampling tick: opens the tree the first time, updates every non-storage root, publishes a snapshot and delivers it to the due subscribers.</summary>
    internal void TickOnce()
    {
        if (Volatile.Read(ref _disposed) == 1)
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
        // (with their value) in this very tick.
        if (Interlocked.Exchange(ref _structureDirty, 0) == 1)
        {
            plan = RebuildPlan(plan);
        }

        var values = new double?[plan.LhmIds.Length];
        IReadOnlyDictionary<string, double?> storage = _storageValues;
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

        _seq++;
        _published = new Published(plan.Revision, plan.Built.Schema, new SnapshotMessage(_seq, tickUnixMs, values));

        lock (_subLock)
        {
            _lastSampleStart = tickStart;
            _sampled = true;
            _nextSampleDue = tickStart + _minIntervalTicks;
        }

        DeliverDue(_time.GetTimestamp());
    }

    /// <summary>One wake of the sampler loop: a sampling tick if one is due, otherwise only the due deliveries. Returns the delay until the next wake.</summary>
    internal TimeSpan RunDue()
    {
        if (Volatile.Read(ref _disposed) == 1)
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

    /// <summary>One storage round: the D6 gate, then an update of every disk known to be spinning, then a new value cache.</summary>
    internal void StorageOnce()
    {
        if (!_opened || Volatile.Read(ref _disposed) == 1)
        {
            return;
        }

        if (!_storageEnabled && !TryEnableStorage())
        {
            return;
        }

        var cache = new Dictionary<string, double?>(StringComparer.Ordinal);
        foreach (HardwareNode root in _tree.Roots)
        {
            if (root.Type != HardwareType.Storage)
            {
                continue;
            }

            if (_stop.IsCancellationRequested)
            {
                return;
            }

            if (root.Storage?.Rotational ?? true)
            {
                int drive = root.Storage?.DriveNumber ?? -1;
                bool? spunDown;
                try
                {
                    spunDown = drive >= 0 ? _disks.IsSpunDown(drive) : null;
                }
                catch (Exception e)
                {
                    LogRateLimited("power:" + root.Identifier, e, "Checking the power mode of {Root} failed", root.Identifier);
                    spunDown = null;
                }

                LogDiskStateChange(root.Identifier, drive, spunDown);
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

            CollectValues(root, cache);
        }

        // Never mutated after publication: the sampler only looks values up.
        _storageValues = cache;
    }

    /// <summary>One wake of the storage loop: a storage round when due (every 30 s, only with a subscriber, only once open). Returns the delay until the next round.</summary>
    internal TimeSpan RunStorageDue()
    {
        if (Volatile.Read(ref _disposed) == 1)
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
            _nextStorageDue = now + (long)(StorageInterval.TotalSeconds * _time.TimestampFrequency);
            return TicksUntil(_nextStorageDue);
        }
    }

    /// <summary>Stops and joins both workers, then disposes the tree (LHM Close, SMBus driver unload, GC).</summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) == 1)
        {
            return;
        }

        _stop.Cancel();
        _samplerWake.Set();
        _storageWake.Set();
        foreach (Thread? worker in new[] { _samplerThread, _storageThread })
        {
            if (worker is not null && worker != Thread.CurrentThread)
            {
                worker.Join();
            }
        }

        _tree.HardwareChanged -= OnHardwareChanged;
        try
        {
            _tree.Dispose();
        }
        catch (Exception e)
        {
            _log.LogError(e, "Closing the hardware tree failed");
        }

        _samplerWake.Dispose();
        _storageWake.Dispose();
        _stop.Dispose();
    }

    private Plan OpenTree()
    {
        long started = _time.GetTimestamp();
        _tree.Open();
        _pawnIo = _pawnIoAvailable();
        _log.LogInformation("PawnIO available: {PawnIo}", _pawnIo);

        // Changes raised during Open() are covered by the Roots read right after.
        Interlocked.Exchange(ref _structureDirty, 0);
        IReadOnlyList<HardwareNode> roots = _tree.Roots;
        Plan plan = new(roots, SchemaBuilder.Build(roots, _pawnIo), revision: 1);
        _plan = plan;
        _log.LogInformation(
            "Hardware tree opened in {Elapsed} ms: {Roots} roots, {Devices} devices, {Sensors} sensors (storage deferred until every rotational disk is active)",
            (long)_time.GetElapsedTime(started).TotalMilliseconds,
            roots.Count,
            plan.Built.Schema.Devices.Count,
            plan.Built.Schema.Sensors.Count);

        // Open() leaves tens of MB of garbage (s1-lhm.md §9.6): compact and decommit once.
        GC.Collect(2, GCCollectionMode.Aggressive, blocking: true, compacting: true);

        lock (_subLock)
        {
            _nextStorageDue = _time.GetTimestamp();
            _opened = true;
        }

        _storageWake.Set();
        return plan;
    }

    private Plan RebuildPlan(Plan current)
    {
        IReadOnlyList<HardwareNode> roots = _tree.Roots;
        BuiltSchema built = SchemaBuilder.Build(roots, _pawnIo);
        Plan plan = SchemaComparer.SameStructure(current.Built, built)
            ? new Plan(roots, current.Built, current.Revision)
            : new Plan(roots, built, current.Revision + 1);
        if (plan.Revision != current.Revision)
        {
            _log.LogInformation(
                "Hardware changed: schema revision {Revision}, {Devices} devices, {Sensors} sensors",
                plan.Revision,
                built.Schema.Devices.Count,
                built.Schema.Sensors.Count);
        }

        _plan = plan;
        return plan;
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
                if (now >= s.NextDue)
                {
                    s.NextDue = now + s.IntervalTicks;
                    _dueScratch.Add(s);
                }
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
        lock (_subLock)
        {
            if (!_subscribers.Remove(subscriber))
            {
                return;
            }

            RecomputeSamplingLocked();
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
        _nextSampleDue = _sampled ? _lastSampleStart + _minIntervalTicks : long.MinValue;
    }

    private void OnHardwareChanged() => Interlocked.Exchange(ref _structureDirty, 1);

    private void LogRateLimited(string key, Exception e, string message, params object?[] args)
    {
        long now = _time.GetTimestamp();
        long minimum = (long)(ErrorLogInterval.TotalSeconds * _time.TimestampFrequency);
        if (_errorLoggedAt.TryGetValue(key, out long last) && now - last < minimum)
        {
            return;
        }

        _errorLoggedAt[key] = now;
#pragma warning disable CA2254 // the templates are constants passed through from the call sites above
        _log.LogWarning(e, message, args);
#pragma warning restore CA2254
    }

    private long MsToTicks(uint ms) => (long)(ms * (double)_time.TimestampFrequency / 1000d);

    private TimeSpan TicksUntil(long due)
    {
        long remaining = due - _time.GetTimestamp();
        return remaining <= 0 ? TimeSpan.Zero : TimeSpan.FromSeconds(remaining / (double)_time.TimestampFrequency);
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
            catch (Exception e) when (e is OperationCanceledException or ObjectDisposedException)
            {
                break; // Dispose()
            }
        }
    }

    private static void SetQuietly(ManualResetEventSlim wake)
    {
        try
        {
            wake.Set();
        }
        catch (ObjectDisposedException)
        {
            // A timer that fired after Dispose(): nothing left to wake.
        }
    }

    /// <summary>Schema, bindings and snapshot published together, so a snapshot always matches the schema sent with it.</summary>
    private sealed record Published(int Revision, SchemaMessage Schema, SnapshotMessage Snapshot);

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

        /// <summary>Schema revision last delivered (0 = none yet); sampler-owned.</summary>
        public int DeliveredRevision { get; set; }

        public void Dispose() => hub.Unsubscribe(this);
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
