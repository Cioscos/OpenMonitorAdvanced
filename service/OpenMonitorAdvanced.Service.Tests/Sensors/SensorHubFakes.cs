using System.Collections.Concurrent;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Sensors;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// Scripted <see cref="IHardwareTree"/>: <see cref="Initial"/> is what <see cref="Open"/>
/// exposes, <see cref="Storage"/> appears only on <see cref="EnableStorage"/> (the D6 gate),
/// and every <c>Open</c>/<c>Update</c>/<c>Read</c>/<c>Dispose</c> (= LHM <c>Close</c>) is counted.
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

    public event Action? HardwareChanged;

    public List<HardwareNode> Initial { get; } = [];

    public List<HardwareNode> Storage { get; } = [];

    public ConcurrentDictionary<string, double?> Values { get; } = new();

    public ConcurrentDictionary<string, bool> Failing { get; } = new();

    /// <summary>Runs at the start of <see cref="Update"/>, on the calling thread.</summary>
    public Action<HardwareNode>? BeforeUpdate { get; set; }

    public int OpenCount => Volatile.Read(ref _openCount);

    public int CloseCount => Volatile.Read(ref _closeCount);

    public int EnableStorageCount => Volatile.Read(ref _enableStorageCount);

    public IReadOnlyList<HardwareNode> Roots
    {
        get
        {
            lock (_gate)
            {
                return _roots.ToArray();
            }
        }
    }

    public IReadOnlyList<HardwareNode> Open()
    {
        Interlocked.Increment(ref _openCount);
        lock (_gate)
        {
            _roots = [.. Initial];
        }

        return Roots;
    }

    public void Update(HardwareNode root)
    {
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
}

internal sealed class FakeDisks : IDiskPowerProbe
{
    private int _spunDownQueries;
    private int _allActiveQueries;

    public volatile bool AllActive = true;

    public ConcurrentDictionary<int, bool?> SpunDown { get; } = new();

    public int SpunDownQueries => Volatile.Read(ref _spunDownQueries);

    public int AllActiveQueries => Volatile.Read(ref _allActiveQueries);

    public bool? IsSpunDown(int driveNumber)
    {
        Interlocked.Increment(ref _spunDownQueries);
        return SpunDown.TryGetValue(driveNumber, out bool? v) ? v : false;
    }

    public bool AllRotationalDisksActive()
    {
        Interlocked.Increment(ref _allActiveQueries);
        return AllActive;
    }
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
