using LibreHardwareMonitor.Hardware;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// The service's LHM modules a client may switch off (spec M5 §2.8). Bit <c>i</c> is the
/// module at index <c>i</c> of <see cref="ProtocolConstants.Modules"/>.
/// </summary>
[Flags]
public enum ServiceModules
{
    None = 0,
    Cpu = 1,
    Motherboard = 2,
    Memory = 4,
    Storage = 8,
    Controller = 16,
    Psu = 32,
    All = 63,
}

/// <summary>
/// One subscriber's request: its interval (already clamped), the modules it does not want, the
/// drive keys (<see cref="DriveKey"/>) whose SMART it does not want and those, among the drives
/// that are off by default (<see cref="DriveFacts.SmartOffByDefault"/>), whose SMART it wants on.
/// </summary>
public sealed record FeedRequest(uint IntervalMs, ServiceModules Disabled, IReadOnlySet<string> SmartDisabledDrives, IReadOnlySet<string> SmartEnabledDrives)
{
    /// <summary>The request of a (decoder-validated) <see cref="SubscribeMessage"/> at <paramref name="intervalMs"/>.</summary>
    public static FeedRequest From(SubscribeMessage subscribe, uint intervalMs) => new(
        intervalMs,
        ServiceModuleNames.Parse(subscribe.DisabledModules),
        new HashSet<string>(subscribe.SmartDisabledDrives, StringComparer.Ordinal),
        new HashSet<string>(subscribe.SmartEnabledDrives, StringComparer.Ordinal));
}

/// <summary>
/// The configuration every subscriber's request adds up to. Compared by content (the drive sets
/// as sets), so an unchanged aggregate is recognised as such.
/// </summary>
public sealed record EffectiveConfig(ServiceModules Enabled, IReadOnlySet<string> SmartDisabledDrives, IReadOnlySet<string> SmartEnabledDrives)
{
    /// <summary>
    /// Every module on, no drive's SMART switched off and none switched on: the configuration
    /// before any request (a drive that is off by default stays off).
    /// </summary>
    public static EffectiveConfig AllOn { get; } = new(ServiceModules.All, new HashSet<string>(StringComparer.Ordinal), new HashSet<string>(StringComparer.Ordinal));

    /// <summary>The part the storage worker applies: the storage flag alone and the two SMART drive sets.</summary>
    public EffectiveConfig StoragePart => new(Enabled & ServiceModules.Storage, SmartDisabledDrives, SmartEnabledDrives);

    /// <summary>
    /// A module is on if at least one request keeps it on. Among the requests with storage on, a
    /// drive's SMART is off only if every one switches it off, and a default-off drive's SMART is
    /// on if at least one switches it on (with storage off everywhere no drive is listed).
    /// <see langword="null"/> without requests: the caller keeps its last configuration.
    /// </summary>
    public static EffectiveConfig? Compute(IReadOnlyCollection<FeedRequest> requests)
    {
        if (requests.Count == 0)
        {
            return null;
        }

        ServiceModules enabled = ServiceModules.None;
        HashSet<string>? smartDisabled = null;
        var smartEnabled = new HashSet<string>(StringComparer.Ordinal);
        foreach (FeedRequest request in requests)
        {
            ServiceModules wanted = ServiceModules.All & ~request.Disabled;
            enabled |= wanted;
            if (!wanted.HasFlag(ServiceModules.Storage))
            {
                continue;
            }

            smartEnabled.UnionWith(request.SmartEnabledDrives);
            if (smartDisabled is null)
            {
                smartDisabled = new HashSet<string>(request.SmartDisabledDrives, StringComparer.Ordinal);
            }
            else
            {
                smartDisabled.IntersectWith(request.SmartDisabledDrives);
            }
        }

        return new EffectiveConfig(enabled, smartDisabled ?? new HashSet<string>(StringComparer.Ordinal), smartEnabled);
    }

    public bool Equals(EffectiveConfig? other) =>
        other is not null
        && Enabled == other.Enabled
        && SmartDisabledDrives.SetEquals(other.SmartDisabledDrives)
        && SmartEnabledDrives.SetEquals(other.SmartEnabledDrives);

    public override int GetHashCode() =>
        HashCode.Combine(Enabled, SmartDisabledDrives.Count, SetHash(SmartDisabledDrives), SmartEnabledDrives.Count, SetHash(SmartEnabledDrives));

    /// <summary>Independent of the enumeration order, like <see cref="ISet{T}.SetEquals"/>.</summary>
    private static int SetHash(IReadOnlySet<string> drives)
    {
        int hash = 0;
        foreach (string drive in drives)
        {
            hash ^= StringComparer.Ordinal.GetHashCode(drive);
        }

        return hash;
    }
}

/// <summary>Which <see cref="ServiceModules"/> group an LHM root belongs to.</summary>
public static class HardwareModules
{
    /// <summary>
    /// The groups switched with LHM's setters (<see cref="IHardwareTree.SetModules"/>). Storage is
    /// not one of them: it is enabled once through the D6 gate and switched off "softly" (P10).
    /// </summary>
    public const ServiceModules TreeGroups = ServiceModules.All & ~ServiceModules.Storage;

    /// <summary>
    /// The group of a root of that type: <c>IsControllerEnabled</c> yields coolers (fan and pump
    /// controllers); the Super I/O and the embedded controller come with the motherboard.
    /// <see cref="ServiceModules.None"/> for a type the service never enables (GPU, network,
    /// battery, power monitor): no request filters it.
    /// </summary>
    public static ServiceModules Of(HardwareType type) => type switch
    {
        HardwareType.Cpu => ServiceModules.Cpu,
        HardwareType.Motherboard or HardwareType.SuperIO or HardwareType.EmbeddedController => ServiceModules.Motherboard,
        HardwareType.Memory => ServiceModules.Memory,
        HardwareType.Storage => ServiceModules.Storage,
        HardwareType.Cooler => ServiceModules.Controller,
        HardwareType.Psu => ServiceModules.Psu,
        _ => ServiceModules.None,
    };

    /// <summary>Whether a root of that type is on in <paramref name="enabled"/> (always, for a type no module owns).</summary>
    public static bool IsOn(HardwareType type, ServiceModules enabled)
    {
        ServiceModules module = Of(type);
        return module == ServiceModules.None || (enabled & module) != 0;
    }
}

/// <summary>Wire names of <see cref="ServiceModules"/>, in <see cref="ProtocolConstants.Modules"/> order.</summary>
public static class ServiceModuleNames
{
    /// <summary>The flags of <paramref name="names"/>; an unknown name throws (the decoder already rejects them).</summary>
    public static ServiceModules Parse(IEnumerable<string> names)
    {
        ServiceModules modules = ServiceModules.None;
        foreach (string name in names)
        {
            int index = IndexOf(name);
            if (index < 0)
            {
                throw new ArgumentException($"unknown module '{name}'", nameof(names));
            }

            modules |= (ServiceModules)(1 << index);
        }

        return modules;
    }

    /// <summary>The names of the modules set in <paramref name="modules"/>, in wire order.</summary>
    public static IReadOnlyList<string> ToWire(ServiceModules modules)
    {
        var names = new List<string>(ProtocolConstants.Modules.Count);
        for (int i = 0; i < ProtocolConstants.Modules.Count; i++)
        {
            if (((int)modules & (1 << i)) != 0)
            {
                names.Add(ProtocolConstants.Modules[i]);
            }
        }

        return names;
    }

    private static int IndexOf(string name)
    {
        for (int i = 0; i < ProtocolConstants.Modules.Count; i++)
        {
            if (string.Equals(ProtocolConstants.Modules[i], name, StringComparison.Ordinal))
            {
                return i;
            }
        }

        return -1;
    }
}
