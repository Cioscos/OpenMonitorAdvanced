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
/// publishes the outcome as one immutable <see cref="StorageRound"/>: the state of every drive,
/// the resolved disks and the raw values keyed by LHM sensor identifier, which the sampler only
/// looks up (values older than two rounds, or from before an idle period, count as absent). A
/// disk enters the schema only once the storage worker has resolved it, so the sampler never
/// waits on disk I/O. The two threads share no lock across hardware I/O: <c>_subLock</c> only
/// guards subscriber bookkeeping; a storage round is one reference swap at its end, the sampler
/// takes one round per tick and uses it for the service block, the schema and the values alike,
/// and schema, bindings and snapshot are swapped as one immutable <see cref="Published"/> reference.
/// </para>
/// <para>
/// <b>D6.</b> The tree is opened without storage. <see cref="IHardwareTree.EnableStorage"/> is
/// called only when <see cref="IDiskPowerProbe.CheckGate"/> says every drive that needs it
/// (controller ruling R17) is spinning, re-checked on every storage round until it succeeds (the
/// drives that block are flagged in the schema's <c>drives</c>). Afterwards nothing blocks: every
/// round lists the drives again (<see cref="IDiskPowerProbe.Enumerate"/>) and asks each one whose
/// <see cref="DriveFacts.RequiresPowerCheck"/> holds and whose SMART is on
/// (<see cref="DriveStates.IsSmartOff"/>) for its power mode once, whether LHM exposes it or
/// not; a disk is updated only when that answer is "active", otherwise its values are absent.
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
/// The service block reports the groups actually open, the storage worker's applied storage part,
/// the request's status and the drive list. When a schema rebuild fails the current plan gets
/// the new block anyway (<c>failed</c> if the request's schema could not be built), so no client
/// waits on a request that cannot be shown.
/// </para>
/// </remarks>
public sealed class SensorHub : ISensorFeed, IDisposable
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

    // Storage-owned state.
    private long _storageSyncedVersion;
    private bool _storageEnabled;
    private bool _storageGateLogged;
    private readonly Dictionary<string, bool?> _diskStates = new(StringComparer.Ordinal);
    private readonly HashSet<string> _undescribedLogged = new(StringComparer.Ordinal);

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
    private volatile EffectiveConfig _storageApplied = EffectiveConfig.AllOn.StoragePart; // written by the storage worker
    private readonly StoragePark _park = new();
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
    /// One sampling tick: opens the tree the first time, updates every non-storage root, publishes a
    /// snapshot and delivers it to the due subscribers. With <paramref name="keepSchedule"/> (the
    /// extra sample for a new request) the sampling anchor and the next due time stay as they are.
    /// </summary>
    internal void TickOnce(bool keepSchedule = false)
    {
        if (IsDisposed)
        {
            return;
        }

        long tickStart = _time.GetTimestamp();
        ulong tickUnixMs = (ulong)_time.GetUtcNow().ToUnixTimeMilliseconds();

        // One storage round for the whole tick: its drive list goes into the service block, its
        // resolved disks into the schema and its values into the snapshot. A round published
        // while this tick runs is the next tick's, so its states never meet this one's values.
        _view = _round;
        ApplyDesired();
        Plan plan = _plan ?? OpenTree();
        if (IsDisposed)
        {
            return; // disposed from this very thread while it switched groups
        }

        // A request's schema effect (or a new service block, or the disks of a new storage round)
        // is built before the updates, so a switched-off group is not updated from this tick on
        // and this snapshot uses the new bindings.
        bool rebuildFailed = false;
        if (!plan.Filter.Equals(_schemaFilter)
            || !ReferenceEquals(plan.Resolved, _view.Resolved)
            || !SchemaComparer.SameServiceState(plan.Built.Schema.Service, _serviceState))
        {
            Interlocked.Exchange(ref _structureDirty, 0);
            plan = TryRebuildPlan(plan);
            rebuildFailed = _rebuildFailing;
        }

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
        // (with their value) in this very tick. A failed rebuild keeps the current devices and is
        // retried on the next tick.
        if (!rebuildFailed && Interlocked.Exchange(ref _structureDirty, 0) == 1)
        {
            plan = TryRebuildPlan(plan);
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
            if (keepSchedule && _sampled)
            {
                // An off-schedule sample for a new request: the phase every subscriber is on stays.
                anchor = _sampleAnchor;
            }
            else
            {
                anchor = ScheduleNextSampleLocked(tickStart);
            }
        }

        // The block the published schema carries reflects the served request only once the plan
        // was rebuilt with it (a failed rebuild keeps the older block, and the older version).
        if (SchemaComparer.SameServiceState(plan.Built.Schema.Service, _serviceState))
        {
            _reflectedVersion = _servedDesired?.Version ?? 0;
        }

        _seq++;
        _published = new Published(plan.Revision, plan.Built.Schema, new SnapshotMessage(_seq, tickUnixMs, values, new bool[values.Length]), anchor, _reflectedVersion);
        DeliverDue(_time.GetTimestamp());
    }

    /// <summary>
    /// Anchors this sample to the schedule when it is on time (or late by less than an interval);
    /// resynced to the actual start when early (a direct call) or further behind. Returns the anchor.
    /// </summary>
    private long ScheduleNextSampleLocked(long tickStart)
    {
        long scheduled = _nextSampleDue;
        bool onSchedule = scheduled != long.MinValue && scheduled <= tickStart && tickStart - scheduled < _minIntervalTicks;
        long anchor = onSchedule ? scheduled : tickStart;
        _sampleAnchor = anchor;
        _sampled = true;
        _nextSampleDue = anchor + _minIntervalTicks;
        return anchor;
    }

    /// <summary>One wake of the sampler loop: a sampling tick if one is due, otherwise only the due deliveries. Returns the delay until the next wake.</summary>
    internal TimeSpan RunDue()
    {
        if (IsDisposed)
        {
            return Timeout.InfiniteTimeSpan;
        }

        long now = _time.GetTimestamp();
        bool due;
        bool newRequest;
        lock (_subLock)
        {
            if (_subscribers.Count == 0)
            {
                return Timeout.InfiniteTimeSpan;
            }

            due = _plan is null || now >= _nextSampleDue;

            // A new desired configuration changes the schema's service block: sampled at once
            // (off schedule, without moving it), so the requesting client is answered with it.
            newRequest = !ReferenceEquals(_desired, _servedDesired);
        }

        if (due || newRequest)
        {
            TickOnce(keepSchedule: !due); // applies the desired configuration first
        }
        else
        {
            // Woken by the storage worker's park, a delivery deadline or a new subscriber: the
            // applier steps anyway; a new service block goes out with the next sample.
            ApplyDesired();
            DeliverDue(now);
        }

        lock (_subLock)
        {
            if (_subscribers.Count == 0)
            {
                return Timeout.InfiniteTimeSpan;
            }

            long wake = _nextSampleDue;
            foreach (Subscriber s in _subscribers)
            {
                if (s.RequestVersion > _reflectedVersion)
                {
                    continue; // waits for a sample that reflects its request (at the latest the next one)
                }

                wake = Math.Min(wake, s.NextDue);
            }

            return TicksUntil(wake);
        }
    }

    /// <summary>
    /// One storage round: the D6 gate or, once it is open, the drive list with one power check
    /// per drive; then for every disk LHM exposes its identity (first touch only) and its update;
    /// then one new <see cref="StorageRound"/>.
    /// </summary>
    internal void StorageOnce()
    {
        SyncStorageConfig();
        if (!_opened || IsDisposed)
        {
            return;
        }

        EffectiveConfig config = _storageApplied;
        if (!config.Enabled.HasFlag(ServiceModules.Storage))
        {
            return; // switched off softly (P10): no gate, no enumeration, no description, no power check, no update
        }

        long roundStart = _time.GetTimestamp();
        IReadOnlyList<DriveCheck>? checks = _storageEnabled ? CheckPowerStates(config) : TryEnableStorage(config, roundStart);
        if (checks is null)
        {
            return;
        }

        // The round's one answer per drive: it decides the update here and the state in the list.
        var answers = new Dictionary<int, DriveCheck>(checks.Count);
        foreach (DriveCheck check in checks)
        {
            answers[check.Drive.DriveNumber] = check;
        }

        IReadOnlyDictionary<string, DiskResolution> previous = _round.Resolved;
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
            if (DriveStates.IsSmartOff(resolution.Facts, resolution.Key, config))
            {
                continue; // SMART off for this disk (P7, or off by default): no update, not in the schema
            }

            StorageInfo info = resolution.Info;
            if (resolution.Facts.Availability == DriveAvailability.NoMedia)
            {
                // LHM enumerated this disk, so "not ready / no media" cannot mean "no platter to
                // wake" (a USB bridge may answer so while its disk sleeps): it cannot be
                // confirmed active, so it is not updated.
                LogDiskStateChange(root.Identifier, info.DriveNumber, spunDown: null);
                continue;
            }

            if (info.Rotational)
            {
                // A drive the round did not ask (gone from the enumeration, say) is unknown.
                bool? spunDown = answers.TryGetValue(info.DriveNumber, out DriveCheck? answer) && answer.Asked ? answer.SpunDown : null;
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
            HardwareNode fresh = FindRoot(root.Identifier) ?? root;
            CollectValues(fresh, cache);
            resolved[root.Identifier] = CollectCriticalWarning(fresh, resolution, cache);
        }

        // Never mutated after publication: the sampler only looks them up.
        PublishRound(roundStart, DriveStates.ToWire(checks, config, gateOpen: true), resolved, cache);
    }

    /// <summary>
    /// The one place a <see cref="StorageRound"/> is published: a single reference swap, with no
    /// lock. <see langword="null"/> keeps the drives or the resolved disks as they are; unchanged
    /// ones keep their instance, which is how the sampler tells that nothing has to be rebuilt.
    /// The storage worker publishes at the end of a round (or when storage is switched off); only
    /// the values are dropped from another thread, when the last client leaves.
    /// </summary>
    private void PublishRound(
        long timestamp,
        IReadOnlyList<WireDrive>? drives,
        IReadOnlyDictionary<string, DiskResolution>? resolved,
        IReadOnlyDictionary<string, double?> values)
    {
        StorageRound current;
        StorageRound next;
        do
        {
            current = _round;
            next = new StorageRound(
                current.Generation + 1,
                timestamp,
                drives is null || drives.SequenceEqual(current.Drives) ? current.Drives : drives,
                resolved is null || SameResolution(current.Resolved, resolved) ? current.Resolved : resolved,
                values);
        }
        while (Interlocked.CompareExchange(ref _round, next, current) != current);
    }

    /// <summary>One wake of the storage loop: a storage round when due (every 30 s, only with a subscriber, only once open). Returns the delay until the next round.</summary>
    internal TimeSpan RunStorageDue()
    {
        if (IsDisposed)
        {
            return Timeout.InfiniteTimeSpan;
        }

        // The boundary of the loop: the storage part of a request is taken here, and here only the
        // worker parks for the sampler's setters, doing no I/O until released (the loop's wait
        // ends on the release's wake or on Dispose).
        SyncStorageConfig();
        bool parked = _park.Park(out bool acknowledged);
        if (acknowledged)
        {
            SetQuietly(_samplerWake);
        }

        if (parked)
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

        // Only the requested groups are ever built; storage waits for the D6 gate.
        ServiceModules groups = (_servedDesired?.Config.Enabled ?? ServiceModules.All) & HardwareModules.TreeGroups;
        _tree.Open(groups);
        _applier.Open(groups);
        ApplyDesired();
        _pawnIo = _pawnIoAvailable();
        _log.LogInformation("PawnIO available: {PawnIo}", _pawnIo);

        // Changes raised during Open() are covered by the Roots read right after.
        Interlocked.Exchange(ref _structureDirty, 0);
        Plan plan = BuildPlan(_tree.Roots, current: null);
        _plan = plan;
        _log.LogInformation(
            "Hardware tree opened in {Elapsed} ms with {Groups}: {Roots} roots, {Devices} devices, {Sensors} sensors (storage deferred until every rotational disk is active)",
            (long)_time.GetElapsedTime(started).TotalMilliseconds,
            groups,
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

    /// <summary>
    /// <see cref="RebuildPlan"/>, or on failure the current plan with the current service block
    /// (<c>failed</c> when the request's schema could not be built), so a waiting client is
    /// answered; the rebuild is retried on the next tick.
    /// </summary>
    private Plan TryRebuildPlan(Plan current)
    {
        if (_rebuildFailing)
        {
            _rebuildFailing = false;
            UpdateServiceState();
        }

        try
        {
            return RebuildPlan(current);
        }
        catch (Exception e)
        {
            Interlocked.Exchange(ref _structureDirty, 1);
            _rebuildFailing = true;
            UpdateServiceState();
            LogRateLimited("schema-rebuild", e, "Rebuilding the schema failed; the current devices stay in use and the rebuild is retried on the next tick");
            if (SchemaComparer.SameServiceState(current.Built.Schema.Service, _serviceState))
            {
                return current;
            }

            BuiltSchema stamped = current.Built with { Schema = current.Built.Schema with { Service = _serviceState } };
            _plan = new Plan(current.Roots, stamped, current.Revision + 1, current.Filter, current.Resolved);
            return _plan;
        }
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
    /// The schema covers every non-storage root of a requested module plus the disks the tick's
    /// storage round has resolved (with their resolved identity) whose SMART is on; unresolved
    /// disks stay out until then. The plan only updates and reads the roots of requested modules.
    /// </summary>
    private Plan BuildPlan(IReadOnlyList<HardwareNode> roots, Plan? current)
    {
        EffectiveConfig filter = _schemaFilter;
        IReadOnlyDictionary<string, DiskResolution> resolved = _view.Resolved;
        var planRoots = new List<HardwareNode>(roots.Count);
        var schemaRoots = new List<HardwareNode>(roots.Count);
        foreach (HardwareNode root in roots)
        {
            if (!HardwareModules.IsOn(root.Type, filter.Enabled))
            {
                continue;
            }

            planRoots.Add(root);
            if (root.Type != HardwareType.Storage)
            {
                schemaRoots.Add(root);
            }
            else if (resolved.TryGetValue(root.Identifier, out DiskResolution? resolution)
                && !DriveStates.IsSmartOff(resolution.Facts, resolution.Key, filter))
            {
                schemaRoots.Add(root with { Storage = resolution.Info });
            }
        }

        // A published id stays pinned while its disk keeps the same complete identity, so an
        // identical disk resolved in a later round never changes it (it gets its own id instead).
        var pins = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach ((string rootId, (DiskResolution pinned, string id)) in _storagePins)
        {
            if (resolved.TryGetValue(rootId, out DiskResolution? resolution) && resolution.Complete && Identity(resolution.Info) == Identity(pinned.Info))
            {
                pins[rootId] = id;
            }
        }

        BuiltSchema built = SchemaBuilder.Build(schemaRoots, _pawnIo, pins, _serviceState);
        foreach (string skipped in built.SkippedRoots)
        {
            LogNotUnique(skipped);
        }

        // A disk the request hides keeps its pin, so it comes back with the id its clients know.
        var hidden = _storagePins.Where(pin => DriveStates.IsSmartOff(pin.Value.Disk.Facts, pin.Value.Disk.Key, filter)).ToList();
        _storagePins.Clear();
        foreach ((string rootId, DiskResolution resolution) in resolved)
        {
            if (resolution.Complete && built.StorageDeviceIds.TryGetValue(rootId, out string? id))
            {
                _storagePins[rootId] = (resolution, id);
            }
        }

        foreach ((string rootId, (DiskResolution Disk, string Id) pin) in hidden)
        {
            _storagePins.TryAdd(rootId, pin);
        }

        if (current is null)
        {
            return new Plan(planRoots, built, revision: 1, filter, resolved);
        }

        return SchemaComparer.SameStructure(current.Built, built)
            ? new Plan(planRoots, current.Built, current.Revision, filter, resolved)
            : new Plan(planRoots, built, current.Revision + 1, filter, resolved);
    }

    /// <summary>The disk identity a pinned id depends on: the NVMe health flags come and go with updates and never change an id.</summary>
    private static StorageInfo Identity(StorageInfo info) => info with { IsNvme = false, HasCriticalWarning = false };

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
        return new DiskResolution(info, facts, complete);
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

    /// <summary>Sampler: the values of the tick's storage round, unless they are too old to be current.</summary>
    private IReadOnlyDictionary<string, double?> FreshStorageValues(long now)
    {
        StorageRound round = _view;
        bool fresh = round.Timestamp != long.MinValue && now - round.Timestamp <= 2 * SecondsToTicks(StorageInterval);
        return fresh ? round.Values : StorageRound.Empty.Values;
    }

    /// <summary>
    /// The D6 gate, asked on every storage round until LHM's storage group is enabled. Returns the
    /// gate's checks once it is: the round goes on with those answers, asking no drive twice.
    /// Otherwise <see langword="null"/>, after publishing the drive list the gate saw.
    /// </summary>
    private IReadOnlyList<DriveCheck>? TryEnableStorage(EffectiveConfig config, long roundStart)
    {
        IReadOnlyList<DriveCheck>? checks;
        try
        {
            checks = _disks.CheckGate();
        }
        catch (Exception e)
        {
            LogRateLimited("storage-gate", e, "Checking the disks' power state failed; storage stays disabled");
            checks = null;
        }

        if (checks is null || checks.Any(c => c.Blocks))
        {
            if (checks is not null)
            {
                PublishRound(roundStart, DriveStates.ToWire(checks, config, gateOpen: false), resolved: null, StorageRound.Empty.Values);
            }

            if (!_storageGateLogged)
            {
                _storageGateLogged = true;
                _log.LogInformation("Storage stays disabled: a rotational disk is in standby or its state is unknown (re-checked every {Seconds} s)", StorageInterval.TotalSeconds);
            }

            return null;
        }

        try
        {
            _tree.EnableStorage();
        }
        catch (Exception e)
        {
            LogRateLimited("storage-enable", e, "Enabling storage failed");

            // No drive blocks: the list says so, and the next round tries again.
            PublishRound(roundStart, DriveStates.ToWire(checks, config, gateOpen: false), resolved: null, StorageRound.Empty.Values);
            return null;
        }

        _storageEnabled = true;
        _log.LogInformation("Every rotational disk is active: storage enabled");
        return checks;
    }

    /// <summary>
    /// Storage worker, once the gate is open: every drive of a fresh enumeration (access 0), with
    /// one power check for each that needs it and whose SMART is on, whether LHM exposes it or
    /// not. A drive whose SMART is off is not asked: nothing is sent to it periodically.
    /// <see langword="null"/> when the drives cannot be listed: no disk is updated blind.
    /// </summary>
    private IReadOnlyList<DriveCheck>? CheckPowerStates(EffectiveConfig config)
    {
        IReadOnlyList<DriveFacts> drives;
        try
        {
            drives = _disks.Enumerate();
        }
        catch (Exception e)
        {
            LogRateLimited("storage-enumerate", e, "Listing the drives failed; no disk is updated in this storage round");
            return null;
        }

        var checks = new List<DriveCheck>(drives.Count);
        foreach (DriveFacts drive in drives)
        {
            if (_stop.IsCancellationRequested)
            {
                return null;
            }

            var unasked = new DriveCheck(drive, Asked: false, SpunDown: null);
            if (!drive.RequiresPowerCheck || DriveStates.IsSmartOff(drive, unasked.Key, config))
            {
                checks.Add(unasked);
                continue;
            }

            bool? spunDown;
            try
            {
                spunDown = _disks.IsSpunDown(drive.DriveNumber, drive.Model, drive.Serial);
            }
            catch (Exception e)
            {
                LogRateLimited("power:" + drive.DriveNumber, e, "Checking the power mode of PhysicalDrive{Drive} failed", drive.DriveNumber);
                spunDown = null;
            }

            checks.Add(unasked with { Asked = true, SpunDown = spunDown });
        }

        return checks;
    }

    /// <summary>
    /// Storage worker: takes the storage part of a new request. Switched off (P10), the values
    /// and the resolved disks are dropped at once and the last drive list stays, every entry
    /// <c>smartOff</c>, with no I/O and the LHM group left open; switched on, or with another
    /// SMART selection, a round runs at once.
    /// </summary>
    private void SyncStorageConfig()
    {
        DesiredConfig? desired = _desired;
        if (desired is null || desired.Version == _storageSyncedVersion)
        {
            return;
        }

        _storageSyncedVersion = desired.Version;
        EffectiveConfig part = desired.Config.StoragePart;
        if (part.Equals(_storageApplied))
        {
            return;
        }

        if (!part.Enabled.HasFlag(ServiceModules.Storage))
        {
            PublishRound(long.MinValue, DriveStates.AllOff(_round.Drives), StorageRound.Empty.Resolved, StorageRound.Empty.Values);
        }
        else
        {
            lock (_subLock)
            {
                _nextStorageDue = _time.GetTimestamp();
            }
        }

        _storageApplied = part;
        SetQuietly(_samplerWake); // the sampler reports the request as applied
    }

    /// <summary>
    /// The NVMe critical warning of a just-updated disk as a 0/1 flag under the disk's own
    /// identifier (its binding's key), read from the SMART attributes that same update left
    /// (no extra command). Returns the resolution with the disk's current NVMe flags: the first
    /// update is what makes the attribute exist, and that changes the schema.
    /// </summary>
    private DiskResolution CollectCriticalWarning(HardwareNode fresh, DiskResolution resolution, Dictionary<string, double?> cache)
    {
        StorageInfo? current = fresh.Storage;
        if (current is null)
        {
            return resolution;
        }

        if (current is { IsNvme: true, HasCriticalWarning: true })
        {
            byte? raw;
            try
            {
                raw = _tree.ReadNvmeCriticalWarning(fresh.Identifier);
            }
            catch (Exception e)
            {
                LogRateLimited("critical-warning:" + fresh.Identifier, e, "Reading the critical warning of {Root} failed", fresh.Identifier);
                raw = null;
            }

            cache[fresh.Identifier] = raw is byte b ? ((b & CriticalWarningMask) != 0 ? 1.0 : 0.0) : null;
        }

        StorageInfo info = resolution.Info;
        return info.IsNvme == current.IsNvme && info.HasCriticalWarning == current.HasCriticalWarning
            ? resolution
            : resolution with { Info = info with { IsNvme = current.IsNvme, HasCriticalWarning = current.HasCriticalWarning } };
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
            PublishRound(long.MinValue, drives: null, resolved: null, StorageRound.Empty.Values);
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
            : _applier.Step(desired, _storageApplied.Equals(_servedStoragePart));
        UpdateServiceState();
    }

    /// <summary>
    /// Sampler: the service block from what is actually applied: the groups open in the tree,
    /// the storage part the storage worker took, the request's status (<c>failed</c> too while
    /// the request's schema cannot be built) and the drive list of the tick's storage round.
    /// </summary>
    private void UpdateServiceState()
    {
        ReconfigurationStatus status = _rebuildFailing && _plan is { } plan && !plan.Filter.Equals(_schemaFilter)
            ? ReconfigurationStatus.Failed
            : _status;
        EffectiveConfig storage = _storageApplied;
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

    /// <summary>
    /// A disk's resolved identity, with the <paramref name="Facts"/> it was described by; only a
    /// <paramref name="Complete"/> one is reused (and its device id pinned).
    /// </summary>
    private sealed record DiskResolution(StorageInfo Info, DriveFacts Facts, bool Complete)
    {
        /// <summary>Its <see cref="DriveKey"/> from the descriptor model and serial; <see langword="null"/> without them.</summary>
        public string? Key { get; } = DriveKey.Compute(Info.DescriptorModel, Info.DescriptorSerial);
    }

    /// <summary>
    /// What the storage worker knows after one round, published as a whole: the state of every
    /// drive, the disks resolved for the schema (by LHM identifier) and the raw values (by LHM
    /// sensor identifier) with the round's monotonic start (<see cref="long.MinValue"/>: no
    /// values). <paramref name="Generation"/> grows by one per publication. Never mutated.
    /// </summary>
    private sealed record StorageRound(
        long Generation,
        long Timestamp,
        IReadOnlyList<WireDrive> Drives,
        IReadOnlyDictionary<string, DiskResolution> Resolved,
        IReadOnlyDictionary<string, double?> Values)
    {
        public static StorageRound Empty { get; } = new(0, long.MinValue, [], new Dictionary<string, DiskResolution>(), new Dictionary<string, double?>());
    }

    /// <summary>
    /// A schema revision with its per-binding sampling plan (sampler-owned, replaced as a whole),
    /// over the roots of the modules <paramref name="filter"/> keeps on.
    /// </summary>
    private sealed class Plan
    {
        public Plan(IReadOnlyList<HardwareNode> roots, BuiltSchema built, int revision, EffectiveConfig filter, IReadOnlyDictionary<string, DiskResolution> resolved)
        {
            Roots = roots;
            Built = built;
            Revision = revision;
            Filter = filter;
            Resolved = resolved;
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
                if (binding.Source == BindingSource.NvmeCriticalWarning)
                {
                    // Keyed by the storage hardware identifier, and only ever cached by the storage worker.
                    FromStorage[i] = true;
                    Owner[i] = binding.LhmIdentifier;
                }
                else if (owners.TryGetValue(binding.LhmIdentifier, out (string Root, bool Storage) owner))
                {
                    FromStorage[i] = owner.Storage;
                    Owner[i] = owner.Root;
                }
            }

            Debug.Assert(built.Schema.Sensors.Count == count, "bindings are index-aligned with the schema sensors");
        }

        public IReadOnlyList<HardwareNode> Roots { get; }

        public BuiltSchema Built { get; }

        public int Revision { get; }

        /// <summary>The request whose schema effect this plan has.</summary>
        public EffectiveConfig Filter { get; }

        /// <summary>The resolved disks (of a <see cref="StorageRound"/>) this plan was built from: another instance means a rebuild.</summary>
        public IReadOnlyDictionary<string, DiskResolution> Resolved { get; }

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

/// <summary>
/// Structural comparison of two built schemas: devices and sensors in order with every field,
/// device properties as an unordered (key-sorted) set, the bindings and the service block. Record equality alone
/// would compare the <see cref="IReadOnlyList{T}"/>/<see cref="IReadOnlyDictionary{TKey,TValue}"/>
/// members by reference.
/// </summary>
internal static class SchemaComparer
{
    public static bool SameStructure(BuiltSchema a, BuiltSchema b)
    {
        if (a.Schema.Devices.Count != b.Schema.Devices.Count
            || !a.Schema.Sensors.SequenceEqual(b.Schema.Sensors)
            || !a.Bindings.SequenceEqual(b.Bindings)
            || !SameServiceState(a.Schema.Service, b.Schema.Service))
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

    /// <summary>The service block counts as structure (spec M5 §2.8): a change is a new revision.</summary>
    public static bool SameServiceState(ServiceStateBlock x, ServiceStateBlock y) =>
        x.Reconfiguration == y.Reconfiguration
        && x.ActiveModules.SequenceEqual(y.ActiveModules)
        && x.SmartDisabledDrives.SequenceEqual(y.SmartDisabledDrives)
        && x.Drives.SequenceEqual(y.Drives);

    private static bool SameProperties(IReadOnlyDictionary<string, string> x, IReadOnlyDictionary<string, string> y) =>
        x.Count == y.Count
        && x.OrderBy(p => p.Key, StringComparer.Ordinal).SequenceEqual(y.OrderBy(p => p.Key, StringComparer.Ordinal));
}
