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
/// <see cref="ProtocolConstants.Modules"/>), the drive keys (<see cref="Sensors.DriveKey"/>)
/// whose SMART it does not want and those, among the drives that are off by default, whose SMART
/// it wants on. The decoder validates the lists; no key may be in both drive lists.
/// </summary>
public sealed record SubscribeMessage(
    uint IntervalMs,
    IReadOnlyList<string> DisabledModules,
    IReadOnlyList<string> SmartDisabledDrives,
    IReadOnlyList<string> SmartEnabledDrives) : IMessage;

public sealed record SchemaMessage(
    IReadOnlyList<WireDevice> Devices,
    IReadOnlyList<WireSensor> Sensors,
    ServiceStateBlock Service) : IMessage;

/// <summary>
/// The service's effective configuration, global to all its clients. <c>Reconfiguration</c> is
/// <c>applied</c>, <c>pending</c> or <c>failed</c>. <c>Drives</c> lists the drives the service knows
/// about, in <c>physical_drive</c> order.
/// </summary>
public sealed record ServiceStateBlock(
    IReadOnlyList<string> ActiveModules,
    IReadOnlyList<string> SmartDisabledDrives,
    string Reconfiguration,
    IReadOnlyList<WireDrive> Drives)
{
    /// <summary>Every module on, nothing disabled, nothing in progress.</summary>
    public static ServiceStateBlock AllActive { get; } = new(ProtocolConstants.Modules, [], "applied", []);
}

/// <summary>
/// One drive of the service block. <c>Key</c> (<see cref="Sensors.DriveKey"/>) and <c>Model</c> are
/// <see langword="null"/> when unknown. <c>State</c> is <c>active</c>, <c>standby</c>, <c>unknown</c>,
/// <c>smartOff</c> or <c>noMedia</c>; <c>BlocksSmart</c> says the drive keeps the SMART gate closed.
/// </summary>
public sealed record WireDrive(uint PhysicalDrive, string? Key, string? Model, string State, bool BlocksSmart);

/// <summary>
/// One sample of every sensor. <c>Held</c> has the length and order of <c>Values</c>; <see langword="true"/>
/// means the value is kept from an earlier measurement, and only a present value can be held.
/// </summary>
public sealed record SnapshotMessage(
    ulong Seq,
    ulong TimestampMs,
    IReadOnlyList<double?> Values,
    IReadOnlyList<bool> Held) : IMessage;

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

/// <summary>App to service: switches the frame engine on or off and picks its optional columns.</summary>
public sealed record FramesConfigureMessage(bool Enabled, bool TrackPcLatency, bool TrackGpu) : IMessage;

/// <summary>App to service: the process whose frames the app wants (<see langword="null"/> for none).</summary>
public sealed record FramesTargetMessage(uint? Pid) : IMessage;

/// <summary>Service to app: state of the frame engine (a <see cref="FramesStates"/> value).</summary>
public sealed record FramesStatusMessage(string State, string? Detail, string? PresentMonVersion) : IMessage;

/// <summary>A process that presented recently, as seen by the frame engine.</summary>
public sealed record PresentingProcess(uint Pid, string Name, double DisplayedFps, string PresentMode, uint Swapchains);

/// <summary>Service to app: the processes presenting now (at most <see cref="ProtocolConstants.MaxPresentingProcesses"/>).</summary>
public sealed record PresentingProcessesMessage(ulong AtQpc, IReadOnlyList<PresentingProcess> Processes) : IMessage;

/// <summary>
/// One presented frame of the target process. <c>FrameType</c> is <c>app</c>, <c>generated_intel_xefg</c>,
/// <c>generated_amd_afmf</c>, <c>generated_other</c> or <c>unknown</c>.
/// </summary>
public sealed record WireFrame(
    ulong Qpc,
    ulong Swapchain,
    string FrameType,
    bool Displayed,
    double MsBetweenPresents,
    double? MsBetweenDisplayChange,
    double? MsUntilDisplayed,
    double? MsAppFrametime,
    double? MsPcLatency,
    double? MsGpuBusy,
    ulong? PclFrameId);

/// <summary>
/// Service to app: frames of the target since the last batch (at most
/// <see cref="ProtocolConstants.MaxFramesPerBatch"/>); <c>Dropped</c> counts the ones left out.
/// </summary>
public sealed record FrameBatchMessage(uint Pid, IReadOnlyList<WireFrame> Frames, uint Dropped) : IMessage;
