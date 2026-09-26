using LibreHardwareMonitor.Hardware;
using LibreHardwareMonitor.Hardware.Storage;
using Microsoft.Extensions.Logging;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// The LibreHardwareMonitor 0.9.6 <see cref="IHardwareTree"/>, and the only type that touches
/// <see cref="Computer"/>. Not unit-tested (it needs administrator rights and the real
/// hardware); exercised live in Task 15. See <c>docs/superpowers/references/m4/s1-lhm.md</c>.
/// </summary>
/// <remarks>
/// <para>
/// <b>D6.</b> <see cref="Open"/> enables CPU, motherboard, memory, controllers and PSUs with
/// storage disabled: LHM's <c>StorageGroup</c> constructor already runs
/// <c>StorageManager.ReloadStorages()</c>, and DiskInfoToolkit's identification reads sector 0,
/// which wakes a sleeping HDD. <see cref="EnableStorage"/> sets
/// <see cref="Computer.IsStorageEnabled"/> later (the setter adds the group to an open computer),
/// only when the hub's gate allows it.
/// </para>
/// <para>
/// <b>Threads.</b> Each root is cached as an immutable <see cref="HardwareNode"/> built on the
/// thread that owns that root (the one that calls <see cref="Update"/> on it: the sampler, or the
/// storage worker for disks), so a node is never read while another thread updates the same
/// hardware. LHM callbacks (<c>HardwareAdded/Removed</c>, <c>SensorAdded/Removed</c>) only set
/// flags; membership is reconciled on the next <see cref="Roots"/> read, under a lock that is
/// never held across hardware I/O. Roots, per-root nodes and the identifier → sensor map are
/// published together as one immutable <see cref="Composition"/>.
/// </para>
/// <para>
/// <b>Memory.</b> Every sensor gets <c>ValuesTimeWindow = TimeSpan.Zero</c> (LHM keeps a one-day
/// history per sensor otherwise), including sensors activated later.
/// </para>
/// </remarks>
public sealed class LhmTree : IHardwareTree
{
    private readonly ILogger<LhmTree> _log;
    private readonly Func<int, bool?> _hasSeekPenalty;
    private readonly object _structureLock = new();
    private readonly List<Entry> _entries = []; // guarded by _structureLock
    private volatile Composition _composition = Composition.Empty;
    private Computer? _computer;
    private int _membershipDirty;
    private int _storageEnabled;
    private int _disposed;

    /// <param name="log">Logger.</param>
    /// <param name="hasSeekPenalty">
    /// Rotational check for a <c>PhysicalDriveN</c> (<see cref="DiskPowerProbe.HasSeekPenalty"/>);
    /// <see langword="null"/> (unknown) counts as rotational.
    /// </param>
    public LhmTree(ILogger<LhmTree> log, Func<int, bool?> hasSeekPenalty)
    {
        _log = log;
        _hasSeekPenalty = hasSeekPenalty;
    }

    /// <inheritdoc />
    public event Action? HardwareChanged;

    /// <inheritdoc />
    public IReadOnlyList<HardwareNode> Roots
    {
        get
        {
            if (Interlocked.Exchange(ref _membershipDirty, 0) == 1)
            {
                Reconcile();
            }

            return _composition.Roots;
        }
    }

    /// <inheritdoc />
    public IReadOnlyList<HardwareNode> Open()
    {
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed) == 1, this);
        if (_computer is not null)
        {
            return Roots;
        }

        var computer = new Computer
        {
            IsCpuEnabled = true,
            IsMotherboardEnabled = true,
            IsMemoryEnabled = true,
            IsControllerEnabled = true,
            IsPsuEnabled = true,
            IsStorageEnabled = false, // D6: see EnableStorage
            IsGpuEnabled = false,
            IsNetworkEnabled = false,
            IsBatteryEnabled = false,
            IsPowerMonitorEnabled = false,
        };
        computer.HardwareAdded += OnMembershipChanged;
        computer.HardwareRemoved += OnMembershipChanged;
        try
        {
            computer.Open();
        }
        catch
        {
            computer.HardwareAdded -= OnMembershipChanged;
            computer.HardwareRemoved -= OnMembershipChanged;
            try
            {
                computer.Close();
            }
            catch (Exception e)
            {
                _log.LogWarning(e, "Closing LibreHardwareMonitor after a failed Open() failed");
            }

            throw;
        }

        _computer = computer;
        Volatile.Write(ref _membershipDirty, 1);
        return Roots;
    }

    /// <inheritdoc />
    public void Update(HardwareNode root)
    {
        if (!_composition.Entries.TryGetValue(root.Identifier, out Entry? entry))
        {
            return; // removed since the caller's snapshot of the roots
        }

        UpdateRecursive(entry.Hardware);

        // Rebuilt on this thread (the root's owner) right after its update: sensors LHM
        // activated or deactivated during Update() (and, after the first update, constant values
        // such as the NVMe spare threshold) reach the node without reading hardware that another
        // thread may be updating.
        if (Interlocked.Exchange(ref entry.Dirty, 0) == 1)
        {
            var sensors = new Dictionary<string, ISensor>(StringComparer.Ordinal);
            HardwareNode node = BuildNode(entry.Hardware, entry.Storage, sensors);
            lock (_structureLock)
            {
                entry.Node = node;
                entry.Sensors = sensors;
                Compose();
            }

            HardwareChanged?.Invoke();
        }
    }

    /// <inheritdoc />
    public double? Read(string sensorIdentifier) =>
        _composition.Sensors.TryGetValue(sensorIdentifier, out ISensor? sensor) && sensor.Value is float value ? value : null;

    /// <inheritdoc />
    public void EnableStorage()
    {
        Computer computer = _computer ?? throw new InvalidOperationException("The tree is not open.");
        if (Interlocked.Exchange(ref _storageEnabled, 1) == 1)
        {
            return;
        }

        try
        {
            // Creates the StorageGroup: enumerates and identifies every disk (D6: only after the
            // hub checked that every rotational disk is spinning). HardwareAdded fires per disk.
            computer.IsStorageEnabled = true;
        }
        catch
        {
            Volatile.Write(ref _storageEnabled, 0);
            throw;
        }
    }

    /// <summary>
    /// <c>Computer.Close()</c>, then <c>RAMSPDToolkit.Windows.Driver.DriverManager.UnloadDriver()</c>
    /// (LHM's memory group never releases its SMBus PawnIO modules), then a full collection so
    /// the failed module loads' handles are finalized (s1-lhm.md §6d, §9.5).
    /// </summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) == 1)
        {
            return;
        }

        Computer? computer = _computer;
        _computer = null;
        if (computer is null)
        {
            return;
        }

        computer.HardwareAdded -= OnMembershipChanged;
        computer.HardwareRemoved -= OnMembershipChanged;
        lock (_structureLock)
        {
            foreach (Entry entry in _entries)
            {
                entry.Detach();
            }

            _entries.Clear();
            _composition = Composition.Empty;
        }

        try
        {
            computer.Close();
        }
        catch (Exception e)
        {
            _log.LogWarning(e, "LibreHardwareMonitor Close() failed");
        }

        try
        {
            RAMSPDToolkit.Windows.Driver.DriverManager.UnloadDriver();
        }
        catch (Exception e)
        {
            _log.LogWarning(e, "Unloading the RAMSPDToolkit SMBus driver failed");
        }

        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
    }

    private void OnMembershipChanged(IHardware hardware)
    {
        // Runs inside LHM (possibly on DiskInfoToolkit's device-change thread): record only.
        Volatile.Write(ref _membershipDirty, 1);
        HardwareChanged?.Invoke();
    }

    private void Reconcile()
    {
        lock (_structureLock)
        {
            Computer? computer = _computer;
            if (computer is null)
            {
                return;
            }

            IList<IHardware> current = computer.Hardware; // a copy, taken under LHM's own lock
            var byHardware = _entries.ToDictionary<Entry, IHardware>(e => e.Hardware, ReferenceEqualityComparer.Instance);
            var alive = new HashSet<IHardware>(current, ReferenceEqualityComparer.Instance);
            foreach (Entry gone in _entries.Where(e => !alive.Contains(e.Hardware)))
            {
                gone.Detach();
                _log.LogInformation("Hardware removed: {Identifier}", gone.Identifier);
            }

            _entries.Clear();
            foreach (IHardware hardware in current)
            {
                if (byHardware.TryGetValue(hardware, out Entry? existing))
                {
                    _entries.Add(existing);
                    continue;
                }

                // Nobody updates this hardware before it is published below, so reading its
                // sensors here is safe from any thread.
                Entry entry = CreateEntry(hardware);
                _entries.Add(entry);
                _log.LogInformation("Hardware added: {Identifier} ({Type}, {Name})", entry.Identifier, hardware.HardwareType, hardware.Name);
            }

            Compose();
        }
    }

    private Entry CreateEntry(IHardware hardware)
    {
        var entry = new Entry(hardware, hardware.Identifier.ToString(), StorageInfoOf(hardware));
        entry.Attach(hardware);
        var sensors = new Dictionary<string, ISensor>(StringComparer.Ordinal);
        entry.Node = BuildNode(hardware, entry.Storage, sensors);
        entry.Sensors = sensors;
        return entry;
    }

    private StorageInfo? StorageInfoOf(IHardware hardware)
    {
        if (hardware is not StorageDevice device)
        {
            return null;
        }

        DiskInfoToolkit.Storage storage = device.Storage;
        int drive = storage.DriveNumber;
        if (drive < 0)
        {
            // No PhysicalDriveN: no descriptor, no hint, never a power-mode query (treated as
            // rotational with an unknown state, so it is never updated).
            _log.LogWarning("{Identifier} has no PhysicalDrive number; its values stay absent", hardware.Identifier.ToString());
            return new StorageInfo(drive, null, null, storage.SerialNumber, Rotational: true);
        }

        (string? model, string? serial) = DriveDescriptor.Read(drive);
        return new StorageInfo(drive, model, serial, storage.SerialNumber, Rotational: _hasSeekPenalty(drive) ?? true);
    }

    /// <summary>Must hold <c>_structureLock</c>.</summary>
    private void Compose()
    {
        var entries = new Dictionary<string, Entry>(StringComparer.Ordinal);
        var sensors = new Dictionary<string, ISensor>(StringComparer.Ordinal);
        var roots = new HardwareNode[_entries.Count];
        for (int i = 0; i < _entries.Count; i++)
        {
            Entry entry = _entries[i];
            roots[i] = entry.Node;
            entries[entry.Identifier] = entry;
            foreach (KeyValuePair<string, ISensor> pair in entry.Sensors)
            {
                sensors[pair.Key] = pair.Value;
            }
        }

        _composition = new Composition(roots, entries, sensors);
    }

    private static void UpdateRecursive(IHardware hardware)
    {
        hardware.Update();
        foreach (IHardware sub in hardware.SubHardware)
        {
            UpdateRecursive(sub);
        }
    }

    private static HardwareNode BuildNode(IHardware hardware, StorageInfo? storage, Dictionary<string, ISensor> sensors)
    {
        // Sorted, so the schema order does not depend on LHM's HashSet order.
        ISensor[] active = [.. hardware.Sensors.OrderBy(s => s.SensorType).ThenBy(s => s.Index)];
        var nodes = new SensorNode[active.Length];
        for (int i = 0; i < active.Length; i++)
        {
            ISensor sensor = active[i];
            sensor.ValuesTimeWindow = TimeSpan.Zero;
            string id = sensor.Identifier.ToString();
            sensors[id] = sensor;
            nodes[i] = new SensorNode(id, sensor.SensorType, sensor.Name, sensor.Index, sensor.Value);
        }

        HardwareNode[] children = [.. hardware.SubHardware.Select(sub => BuildNode(sub, null, sensors))];
        return new HardwareNode(hardware.Identifier.ToString(), hardware.HardwareType, hardware.Name, nodes, children, storage);
    }

    private sealed record Composition(HardwareNode[] Roots, Dictionary<string, Entry> Entries, Dictionary<string, ISensor> Sensors)
    {
        public static Composition Empty { get; } = new([], new Dictionary<string, Entry>(), new Dictionary<string, ISensor>());
    }

    private sealed class Entry(IHardware hardware, string identifier, StorageInfo? storage)
    {
        private readonly List<(IHardware Hardware, SensorEventHandler Handler)> _subscriptions = [];

        /// <summary>1 when the node must be rebuilt after the next update (initially: after the first one).</summary>
        public int Dirty = 1;

        public IHardware Hardware { get; } = hardware;

        public string Identifier { get; } = identifier;

        public StorageInfo? Storage { get; } = storage;

        /// <summary>Guarded by <c>_structureLock</c>.</summary>
        public HardwareNode Node { get; set; } = null!;

        /// <summary>Identifier → sensor for this root and its sub-hardware; guarded by <c>_structureLock</c>.</summary>
        public Dictionary<string, ISensor> Sensors { get; set; } = null!;

        public void Attach(IHardware hardware)
        {
            SensorEventHandler handler = sensor =>
            {
                // Runs inside LHM, on the thread updating this hardware: record only.
                sensor.ValuesTimeWindow = TimeSpan.Zero;
                Volatile.Write(ref Dirty, 1);
            };
            hardware.SensorAdded += handler;
            hardware.SensorRemoved += handler;
            _subscriptions.Add((hardware, handler));
            foreach (IHardware sub in hardware.SubHardware)
            {
                Attach(sub);
            }
        }

        public void Detach()
        {
            foreach ((IHardware hardware, SensorEventHandler handler) in _subscriptions)
            {
                hardware.SensorAdded -= handler;
                hardware.SensorRemoved -= handler;
            }

            _subscriptions.Clear();
        }
    }
}
