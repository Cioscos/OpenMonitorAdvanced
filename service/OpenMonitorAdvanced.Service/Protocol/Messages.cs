namespace OpenMonitorAdvanced.Service.Protocol;

/// <summary>
/// Marker interface for the one envelope every frame payload decodes to
/// (the Rust side's <c>Message</c> enum, spec §6). Every field on every
/// message below is always present on the wire (<c>nil</c> for "absent");
/// none uses anything resembling <c>skip_serializing_if</c> — see
/// <c>protocol/fixtures/README.md</c>, ruling R10.
/// </summary>
public interface IMessage;

/// <summary>
/// The service's greeting. <paramref name="PawnIo"/> is the state of the PawnIO driver
/// (<c>ok</c>, <c>missing</c>, <c>unavailable</c>, <c>unknown</c> or <c>rebootPending</c>).
/// </summary>
public sealed record HelloMessage(uint ProtocolVersion, string ServiceVersion, string PawnIo) : IMessage;

/// <summary>
/// A client's request: the interval, the modules it does not want (names from
/// <see cref="ProtocolConstants.Modules"/>) and the drive keys (<see cref="Sensors.DriveKey"/>)
/// whose SMART it does not want. The decoder validates the lists.
/// </summary>
public sealed record SubscribeMessage(
    uint IntervalMs,
    IReadOnlyList<string> DisabledModules,
    IReadOnlyList<string> SmartDisabledDrives) : IMessage;

public sealed record SchemaMessage(
    IReadOnlyList<WireDevice> Devices,
    IReadOnlyList<WireSensor> Sensors,
    ServiceStateBlock Service) : IMessage;

/// <summary>
/// The service's effective configuration, global to all its clients. <c>Reconfiguration</c> is
/// <c>applied</c>, <c>pending</c> or <c>failed</c>.
/// </summary>
public sealed record ServiceStateBlock(
    IReadOnlyList<string> ActiveModules,
    IReadOnlyList<string> SmartDisabledDrives,
    string Reconfiguration,
    IReadOnlyList<string> SmartBlockedBy)
{
    /// <summary>Every module on, nothing disabled, nothing in progress.</summary>
    public static ServiceStateBlock AllActive { get; } = new(ProtocolConstants.Modules, [], "applied", []);
}

public sealed record SnapshotMessage(ulong Seq, ulong TimestampMs, IReadOnlyList<double?> Values) : IMessage;

public sealed record ErrorMessage(string Code, string Message) : IMessage;

public sealed record WireDevice(
    string Id,
    string Kind,
    string Name,
    string? Vendor,
    IReadOnlyDictionary<string, string> Properties,
    IdentityHint? Hint);

/// <summary>
/// Tagged union of device-identity hints, adjacently tagged with
/// <c>kind</c>/<c>value</c> (not <c>type</c>/<c>body</c>, to avoid colliding
/// with <see cref="WireDevice.Kind"/>).
/// </summary>
public abstract record IdentityHint;

public sealed record CpuHint(uint Index) : IdentityHint;

public sealed record StorageHint(uint PhysicalDrive, string? Model, string? Serial) : IdentityHint;

public sealed record MemoryHint : IdentityHint;

public sealed record WireSensor(
    string DeviceId,
    string Kind,
    string Name,
    string Unit,
    string LabelKey,
    string? LabelArg,
    string Category);
