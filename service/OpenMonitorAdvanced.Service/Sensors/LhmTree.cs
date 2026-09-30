using DiskInfoToolkit.Interop.Enums;
using LibreHardwareMonitor.Hardware;
using LibreHardwareMonitor.Hardware.Cpu;
using LibreHardwareMonitor.Hardware.Storage;
using Microsoft.Extensions.Logging;
using SmartAttribute = DiskInfoToolkit.SmartAttribute;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// The LibreHardwareMonitor 0.9.6 <see cref="IHardwareTree"/>, and the only type that touches
/// <see cref="Computer"/>. Its hardware paths need administrator rights and the real hardware, so
/// they are exercised live in Task 15; unit tests cover <see cref="Dispose"/> over a
/// <see cref="Computer"/> with no group enabled, and the groups that <see cref="Open"/> and
/// <see cref="SetModules"/> ask for over a computer never opened. See
/// <c>docs/superpowers/references/m4/s1-lhm.md</c>.
/// </summary>
/// <remarks>
/// <para>
/// <b>D6.</b> <see cref="Open"/> enables the requested CPU, motherboard, memory, controller and
/// PSU groups with storage disabled: LHM's <c>StorageGroup</c> constructor already runs
/// <c>StorageManager.ReloadStorages()</c>, and DiskInfoToolkit's identification reads sector 0,
/// which wakes a sleeping HDD. <see cref="EnableStorage"/> sets
/// <see cref="Computer.IsStorageEnabled"/> later (the setter adds the group to an open computer),
/// only when the hub's gate allows it, and nothing ever clears it (P10: switching it off and on
/// again would identify every disk again and leak the old group on a static event).
/// </para>
/// <para>
/// <b>Groups.</b> <see cref="SetModules"/> flips LHM's setters on an open computer: removing a
/// group closes it (PawnIO modules, Super I/O, HID handles) on the calling thread, adding one runs
/// its detection there. The hub calls it on the sampler only while its storage worker is parked,
/// since LHM's own guarantee (no group closed during an update) relies on <c>Computer.Accept</c>,
/// which this class does not use.
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
    private readonly Func<Computer> _createComputer;
    private readonly Action<Computer> _openComputer;
    private readonly object _structureLock = new();
    private readonly List<Entry> _entries = []; // guarded by _structureLock
    private volatile Composition _composition = Composition.Empty;
    private Computer? _computer;
    private int _membershipDirty;
    private int _storageEnabled;
    private int _disposed;

    public LhmTree(ILogger<LhmTree> log)
        : this(log, CreateComputer)
    {
    }

    /// <summary>
    /// Tests only: <paramref name="createComputer"/> replaces the <see cref="Computer"/> and
    /// <paramref name="openComputer"/> its <c>Open()</c> (a computer never opened only records the
    /// flags its setters are given, without building a group).
    /// </summary>
    internal LhmTree(ILogger<LhmTree> log, Func<Computer> createComputer, Action<Computer>? openComputer = null)
    {
        _log = log;
        _createComputer = createComputer;
        _openComputer = openComputer ?? (computer => computer.Open());
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
    public IReadOnlyList<HardwareNode> Open(ServiceModules enabled)
    {
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed) == 1, this);
        if (_computer is not null)
        {
            return Roots;
        }

        Computer computer = _createComputer();
        computer.IsCpuEnabled = enabled.HasFlag(ServiceModules.Cpu);
        computer.IsMotherboardEnabled = enabled.HasFlag(ServiceModules.Motherboard);
        computer.IsMemoryEnabled = enabled.HasFlag(ServiceModules.Memory);
        computer.IsControllerEnabled = enabled.HasFlag(ServiceModules.Controller);
        computer.IsPsuEnabled = enabled.HasFlag(ServiceModules.Psu);
        computer.IsStorageEnabled = false; // D6: see EnableStorage
        computer.IsGpuEnabled = false;
        computer.IsNetworkEnabled = false;
        computer.IsBatteryEnabled = false;
        computer.IsPowerMonitorEnabled = false;
        computer.HardwareAdded += OnMembershipChanged;
        computer.HardwareRemoved += OnMembershipChanged;
        try
        {
            _openComputer(computer);
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
    public byte? ReadNvmeCriticalWarning(string storageIdentifier) =>
        _composition.Entries.TryGetValue(storageIdentifier, out Entry? entry) ? CriticalWarningOf(entry.Hardware) : null;

    /// <summary>
    /// The Critical Warning attribute of an NVMe disk (<c>SmartAttributeType.CriticalWarning</c>,
    /// byte 0 of the SMART/Health log), as the last <c>Update()</c> left it. Never the raw id
    /// <c>0x01</c>, which is "Read Error Rate" on ATA. Members are read directly (no reflection).
    /// </summary>
    private static byte? CriticalWarningOf(IHardware hardware)
    {
        if (hardware is not StorageDevice { Storage: { IsNVMe: true, Smart: { } smart } })
        {
            return null;
        }

        foreach (SmartAttribute attribute in smart.SmartAttributes)
        {
            if (attribute.Info.Type == SmartAttributeType.CriticalWarning)
            {
                byte[]? raw = attribute.Attribute.RawValue;
                return raw is { Length: > 0 } ? raw[0] : null;
            }
        }

        return null;
    }

    /// <inheritdoc />
    /// <remarks>
    /// Removals first, then (after the memory group) a full collection with its finalizers, then
    /// additions: RAMSPDToolkit's <c>~SPDAccessor</c> restores each DIMM's SPD page over SMBus, so
    /// those writes finish, with the driver still loaded (see <see cref="Dispose"/>), before a new
    /// memory detection can use the bus.
    /// </remarks>
    public void SetModules(ServiceModules enabled)
    {
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed) == 1, this);
        Computer computer = _computer ?? throw new InvalidOperationException("The tree is not open.");
        try
        {
            bool memoryRemoved = computer.IsMemoryEnabled && !enabled.HasFlag(ServiceModules.Memory);
            Switch(computer, enabled, on: false);
            if (memoryRemoved)
            {
                Reconcile(); // drops our references to the closed DIMMs, so they can be finalized
                GC.Collect();
                GC.WaitForPendingFinalizers();
            }

            Switch(computer, enabled, on: true);
        }
        finally
        {
            // Whatever the setters did (also before one threw) is reflected before returning.
            Interlocked.Exchange(ref _membershipDirty, 0);
            Reconcile();
        }
    }

    /// <summary>Flips the setter of every group whose state differs and that <paramref name="enabled"/> turns <paramref name="on"/> (or off); never storage.</summary>
    private void Switch(Computer computer, ServiceModules enabled, bool on)
    {
        if (computer.IsCpuEnabled != on && enabled.HasFlag(ServiceModules.Cpu) == on)
        {
            LogSwitch(ServiceModules.Cpu, on);
            computer.IsCpuEnabled = on;
        }

        if (computer.IsMotherboardEnabled != on && enabled.HasFlag(ServiceModules.Motherboard) == on)
        {
            LogSwitch(ServiceModules.Motherboard, on);
            computer.IsMotherboardEnabled = on;
        }

        if (computer.IsMemoryEnabled != on && enabled.HasFlag(ServiceModules.Memory) == on)
        {
            LogSwitch(ServiceModules.Memory, on);
            computer.IsMemoryEnabled = on;
        }

        if (computer.IsControllerEnabled != on && enabled.HasFlag(ServiceModules.Controller) == on)
        {
            LogSwitch(ServiceModules.Controller, on);
            computer.IsControllerEnabled = on;
        }

        if (computer.IsPsuEnabled != on && enabled.HasFlag(ServiceModules.Psu) == on)
        {
            LogSwitch(ServiceModules.Psu, on);
            computer.IsPsuEnabled = on;
        }
    }

    private void LogSwitch(ServiceModules module, bool on) =>
        _log.LogInformation("{Action} the LibreHardwareMonitor {Module} group", on ? "Opening" : "Closing", module);

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
    /// <c>Computer.Close()</c> only. The tree is disposed once, when the process is about to exit
    /// (spec §2.2), and the OS releases every PawnIO handle then.
    /// </summary>
    /// <remarks>
    /// Task 15: never <c>RAMSPDToolkit…DriverManager.UnloadDriver()</c> and never a forced
    /// collection here. <c>MemoryGroup.Close()</c> drops the DIMMs, so their RAMSPDToolkit
    /// <c>SPDAccessor</c>s become finalizable, and <c>~SPDAccessor</c> restores the SPD page over
    /// SMBus through LHM's PawnIO module. Unloading the driver nulls that module, so the next
    /// finalizer run throws a NullReferenceException on the finalizer thread, which kills the
    /// stopping service (exit 1, SCM event 7031, restart after 60 s). With the driver loaded for
    /// the whole process lifetime (the static <c>DriverManager.Driver</c> keeps its modules
    /// reachable), a finalizer that runs after this point still finds a live module; .NET runs no
    /// finalizers at process exit.
    /// </remarks>
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
    }

    /// <summary>A computer with no group: <see cref="Open"/> chooses them.</summary>
    private static Computer CreateComputer() => new();

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
                // (in-memory) sensors here is safe from any thread. No disk I/O under this lock.
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

    /// <summary>
    /// In-memory identity only (DiskInfoToolkit's drive number and IDENTIFY serial): no I/O here,
    /// since this runs under <c>_structureLock</c> and possibly on the sampler thread. The
    /// descriptor model/serial and the rotational flag are left unknown (<c>Rotational: true</c>);
    /// the hub's storage worker resolves them through <see cref="IDiskPowerProbe.Describe"/>
    /// before the disk enters the schema.
    /// </summary>
    private StorageInfo? StorageInfoOf(IHardware hardware)
    {
        if (hardware is not StorageDevice device)
        {
            return null;
        }

        DiskInfoToolkit.Storage storage = device.Storage;
        if (storage.DriveNumber < 0)
        {
            _log.LogWarning("{Identifier} has no PhysicalDrive number: it cannot be described or power-checked, so it stays out of the schema", hardware.Identifier.ToString());
        }

        return new StorageInfo(storage.DriveNumber, null, null, storage.SerialNumber, Rotational: true, IsNvme: storage.IsNVMe);
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
        // The attribute list is filled by an Update(): a node rebuilt after one reports it.
        StorageInfo? withHealth = storage is null ? null : storage with { HasCriticalWarning = storage.IsNvme && CriticalWarningOf(hardware) is not null };
        return new HardwareNode(hardware.Identifier.ToString(), hardware.HardwareType, hardware.Name, nodes, children, withHealth, ReadCpuInfo(hardware, active));
    }

    /// <summary>
    /// The CPUID identity of a CPU (raw brand string, family, model) and, for an Intel CPU, the
    /// "TjMax [°C]" parameter of its "CPU Package" sensor. Members are read directly (no
    /// reflection), so the trimmer keeps them.
    /// </summary>
    private static CpuInfo? ReadCpuInfo(IHardware hardware, ISensor[] sensors)
    {
        if (hardware is not GenericCpu cpu || cpu.CpuId.Length == 0 || cpu.CpuId[0].Length == 0)
        {
            return null;
        }

        CpuId id = cpu.CpuId[0][0];
        double? intelTjMaxC = null;
        if (id.Vendor == Vendor.Intel)
        {
            ISensor? package = sensors.FirstOrDefault(s => s.SensorType == SensorType.Temperature && s.Name == "CPU Package");
            IParameter? tjMax = package?.Parameters.FirstOrDefault(p => p.Name == "TjMax [°C]");
            intelTjMaxC = tjMax?.Value;
        }

        return new CpuInfo(id.Vendor.ToString(), id.BrandString ?? string.Empty, (int)id.Family, (int)id.Model, intelTjMaxC);
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
