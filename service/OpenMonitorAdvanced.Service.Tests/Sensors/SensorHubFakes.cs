using System.Collections.Concurrent;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Sensors;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// Scripted <see cref="IHardwareTree"/>: <see cref="Initial"/> is what <see cref="Open"/>
/// exposes (only the roots of the requested modules, by <see cref="HardwareNode.Type"/>),
/// <see cref="Storage"/> appears only on <see cref="EnableStorage"/> (the D6 gate),
/// <see cref="SetModules"/> adds and removes the <see cref="Initial"/> roots of the modules it
/// switches, and every <c>Open</c>/<c>SetModules</c>/<c>Update</c>/<c>Read</c>/<c>Dispose</c>
/// (= LHM <c>Close</c>) is counted.
/// </summary>
internal sealed class FakeTree : IHardwareTree
{
    private readonly object _gate = new();
    private readonly ConcurrentDictionary<string, int> _updates = new();
    private readonly ConcurrentDictionary<string, int> _reads = new();
    private List<HardwareNode> _roots = [];
    private int _openCount;
    private int _closeCount;
    private int _enableStorageCount;
    private int _updatesAfterClose;
    private int _throwOnNextRoots;
    private int _setModulesCalls;
    private int _setModulesFailures;
    private ServiceModules _modules;

    public event Action? HardwareChanged;

    public List<HardwareNode> Initial { get; } = [];

    public List<HardwareNode> Storage { get; } = [];

    public ConcurrentDictionary<string, double?> Values { get; } = new();

    public ConcurrentDictionary<string, bool> Failing { get; } = new();

    /// <summary>Runs at the start of <see cref="Update"/>, on the calling thread.</summary>
    public Action<HardwareNode>? BeforeUpdate { get; set; }

    /// <summary>Runs at the start of <see cref="SetModules"/>, on the calling thread.</summary>
    public Action<ServiceModules>? BeforeSetModules { get; set; }

    /// <summary>Every <see cref="Roots"/> read throws while set (a schema rebuild that keeps failing).</summary>
    public volatile bool ThrowOnRoots;

    /// <summary>The modules <see cref="Open"/> was asked for; <see langword="null"/> before it.</summary>
    public ServiceModules? OpenedModules { get; private set; }

    public int SetModulesCalls => Volatile.Read(ref _setModulesCalls);

    /// <summary>Every <see cref="SetModules"/> argument with its calling thread's name, in order.</summary>
    public ConcurrentQueue<(ServiceModules Modules, string? Thread)> SetModulesLog { get; } = new();

    /// <summary>The next <paramref name="count"/> <see cref="SetModules"/> calls throw without changing anything.</summary>
    public void FailNextSetModules(int count) => Volatile.Write(ref _setModulesFailures, count);

    public int OpenCount => Volatile.Read(ref _openCount);

    public int CloseCount => Volatile.Read(ref _closeCount);

    public int EnableStorageCount => Volatile.Read(ref _enableStorageCount);

    /// <summary><see cref="Update"/> calls made after <see cref="Dispose"/> (= LHM Close).</summary>
    public int UpdatesAfterClose => Volatile.Read(ref _updatesAfterClose);

    public string? OpenThreadName { get; private set; }

    /// <summary>The next <see cref="Roots"/> read throws once (a failing schema rebuild).</summary>
    public void ThrowOnNextRoots() => Volatile.Write(ref _throwOnNextRoots, 1);

    public IReadOnlyList<HardwareNode> Roots
    {
        get
        {
            if (Interlocked.Exchange(ref _throwOnNextRoots, 0) == 1 || ThrowOnRoots)
            {
                throw new InvalidOperationException("roots unavailable");
            }

            lock (_gate)
            {
                return _roots.ToArray();
            }
        }
    }

    public IReadOnlyList<HardwareNode> Open(ServiceModules enabled)
    {
        Interlocked.Increment(ref _openCount);
        OpenThreadName = Thread.CurrentThread.Name;
        OpenedModules = enabled;
        lock (_gate)
        {
            _modules = enabled;
            _roots = [.. Initial.Where(r => IsOn(r, enabled))];
        }

        return Roots;
    }

    public void SetModules(ServiceModules enabled)
    {
        Interlocked.Increment(ref _setModulesCalls);
        SetModulesLog.Enqueue((enabled, Thread.CurrentThread.Name));
        BeforeSetModules?.Invoke(enabled);
        if (Interlocked.Decrement(ref _setModulesFailures) >= 0)
        {
            throw new InvalidOperationException("a module failed to load");
        }

        Volatile.Write(ref _setModulesFailures, 0);
        lock (_gate)
        {
            ServiceModules added = enabled & ~_modules;
            _modules = enabled;
            _roots = [.. _roots.Where(r => IsOn(r, enabled)), .. Initial.Where(r => (added & HardwareModules.Of(r.Type)) != 0)];
        }

        HardwareChanged?.Invoke();
    }

    public void Update(HardwareNode root)
    {
        if (CloseCount > 0)
        {
            Interlocked.Increment(ref _updatesAfterClose);
        }

        BeforeUpdate?.Invoke(root);
        _updates.AddOrUpdate(root.Identifier, 1, (_, n) => n + 1);
        if (Failing.ContainsKey(root.Identifier))
        {
            throw new InvalidOperationException($"update of {root.Identifier} failed");
        }
    }

    public double? Read(string sensorIdentifier)
    {
        _reads.AddOrUpdate(sensorIdentifier, 1, (_, n) => n + 1);
        return Values.TryGetValue(sensorIdentifier, out double? v) ? v : null;
    }

    public void EnableStorage()
    {
        Interlocked.Increment(ref _enableStorageCount);
        lock (_gate)
        {
            _roots.AddRange(Storage);
        }

        HardwareChanged?.Invoke();
    }

    /// <summary>Replaces the whole tree and raises <see cref="HardwareChanged"/> (hardware added/removed, sensor activated).</summary>
    public void Replace(params HardwareNode[] roots)
    {
        lock (_gate)
        {
            _roots = [.. roots];
        }

        HardwareChanged?.Invoke();
    }

    public int Updates(string rootIdentifier) => _updates.GetValueOrDefault(rootIdentifier);

    public int Reads(string sensorIdentifier) => _reads.GetValueOrDefault(sensorIdentifier);

    public void Dispose() => Interlocked.Increment(ref _closeCount);

    /// <summary>A root outside the switchable groups (storage, or a type no module owns) is always there.</summary>
    private static bool IsOn(HardwareNode root, ServiceModules enabled)
    {
        ServiceModules module = HardwareModules.Of(root.Type);
        return (module & HardwareModules.TreeGroups) == 0 || (enabled & module) != 0;
    }
}

internal sealed class FakeDisks : IDiskPowerProbe
{
    private int _spunDownQueries;
    private int _allActiveQueries;
    private int _describeCalls;
    private readonly ConcurrentDictionary<int, int> _spunDownQueriesOf = new();
    private readonly ConcurrentDictionary<int, int> _describeCallsOf = new();

    public volatile bool AllActive = true;

    /// <summary>What <see cref="GateBlockers"/> answers while <see cref="AllActive"/> is false (default: drive 0 in standby).</summary>
    public List<DriveBlocker> Blockers { get; } = [];

    public ConcurrentDictionary<int, bool?> SpunDown { get; } = new();

    public int SpunDownQueries => Volatile.Read(ref _spunDownQueries);

    public int AllActiveQueries => Volatile.Read(ref _allActiveQueries);

    public int DescribeCalls => Volatile.Read(ref _describeCalls);

    public int SpunDownQueriesOf(int drive) => _spunDownQueriesOf.GetValueOrDefault(drive);

    public int DescribeCallsOf(int drive) => _describeCallsOf.GetValueOrDefault(drive);

    /// <summary>Per-drive facts; a drive not listed is described as a spinning SATA HDD.</summary>
    public ConcurrentDictionary<int, DriveFacts?> Facts { get; } = new();

    /// <summary>Runs at the start of <see cref="Describe"/>, on the calling thread.</summary>
    public Action<int>? BeforeDescribe { get; set; }

    public ConcurrentQueue<string?> DescribeThreads { get; } = new();

    public DriveFacts? Describe(int driveNumber)
    {
        Interlocked.Increment(ref _describeCalls);
        _describeCallsOf.AddOrUpdate(driveNumber, 1, (_, n) => n + 1);
        DescribeThreads.Enqueue(Thread.CurrentThread.Name);
        BeforeDescribe?.Invoke(driveNumber);
        return Facts.TryGetValue(driveNumber, out DriveFacts? facts)
            ? facts
            : new DriveFacts(driveNumber, DriveAvailability.Present, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL", BusType: 0x0B, SeekPenalty: true);
    }

    public bool? IsSpunDown(int driveNumber)
    {
        Interlocked.Increment(ref _spunDownQueries);
        _spunDownQueriesOf.AddOrUpdate(driveNumber, 1, (_, n) => n + 1);
        return SpunDown.TryGetValue(driveNumber, out bool? v) ? v : false;
    }

    public IReadOnlyList<DriveBlocker> GateBlockers()
    {
        Interlocked.Increment(ref _allActiveQueries);
        if (AllActive)
        {
            return [];
        }

        return Blockers.Count > 0
            ? [.. Blockers]
            : [new DriveBlocker(new DriveFacts(0, DriveAvailability.Present, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL", BusType: 0x0B, SeekPenalty: true), SpunDown: true)];
    }
}

/// <summary>Builds <see cref="FeedRequest"/>s for tests.</summary>
internal static class Requests
{
    public static FeedRequest Of(uint intervalMs, ServiceModules disabled = ServiceModules.None, params string[] smartDisabledDrives) =>
        new(intervalMs, disabled, new HashSet<string>(smartDisabledDrives, StringComparer.Ordinal));

    /// <summary>A subscription with every source on, as the M4 tests made it.</summary>
    public static IFeedSubscription Subscribe(this ISensorFeed feed, uint intervalMs, Action<FeedUpdate> onUpdate) =>
        feed.Subscribe(Of(intervalMs), onUpdate);
}

internal sealed record LogEntry(LogLevel Level, string Message, Exception? Exception);

internal sealed class ListLogger<T> : ILogger<T>
{
    private readonly ConcurrentQueue<LogEntry> _entries = new();

    public IReadOnlyList<LogEntry> Entries => _entries.ToArray();

    public IDisposable? BeginScope<TState>(TState state)
        where TState : notnull => null;

    public bool IsEnabled(LogLevel logLevel) => true;

    public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter) =>
        _entries.Enqueue(new LogEntry(logLevel, formatter(state, exception), exception));
}
