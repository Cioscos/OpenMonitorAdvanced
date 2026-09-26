namespace OpenMonitorAdvanced.Service.Protocol;

/// <summary>
/// Marker interface for the one envelope every frame payload decodes to
/// (the Rust side's <c>Message</c> enum, spec §6). Every field on every
/// message below is always present on the wire (<c>nil</c> for "absent");
/// none uses anything resembling <c>skip_serializing_if</c> — see
/// <c>protocol/fixtures/README.md</c>, ruling R10.
/// </summary>
public interface IMessage;

public sealed record Hello(uint ProtocolVersion, string ServiceVersion) : IMessage;

public sealed record Subscribe(uint IntervalMs) : IMessage;

public sealed record SchemaMessage(IReadOnlyList<WireDevice> Devices, IReadOnlyList<WireSensor> Sensors) : IMessage;

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
