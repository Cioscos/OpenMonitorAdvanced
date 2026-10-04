namespace OpenMonitorAdvanced.Service.Protocol;

/// <summary>
/// Constants shared by both ends of the sensor IPC protocol (spec §6).
/// These values must stay in lockstep with the Rust crate <c>oma-ipc</c>
/// (<c>crates/oma-ipc/src/lib.rs</c>).
/// </summary>
public static class ProtocolConstants
{
    /// <summary>Current sensor IPC protocol version, sent in <see cref="HelloMessage.ProtocolVersion"/>.</summary>
    public const uint Version = 4;

    /// <summary>Name of the sensor named pipe.</summary>
    public const string PipeName = "OpenMonitorAdvanced.Sensors.v1";

    /// <summary>Maximum size, in bytes, of a single frame's MessagePack payload.</summary>
    public const int MaxFrameBytes = 4 * 1024 * 1024;

    /// <summary>Minimum accepted <see cref="SubscribeMessage.IntervalMs"/>.</summary>
    public const uint MinIntervalMs = 250;

    /// <summary>Maximum accepted <see cref="SubscribeMessage.IntervalMs"/>.</summary>
    public const uint MaxIntervalMs = 5000;

    /// <summary>The service modules a client may switch off, in wire order (Rust: <c>MODULES</c>).</summary>
    public static readonly IReadOnlyList<string> Modules = ["cpu", "motherboard", "memory", "storage", "controller", "psu"];

    /// <summary>Maximum number of drive keys in a <see cref="SubscribeMessage"/> (Rust: <c>MAX_DRIVE_KEYS</c>).</summary>
    public const int MaxDriveKeys = 64;

    /// <summary>Maximum number of frames in one <see cref="FrameBatchMessage"/> (Rust: <c>MAX_FRAMES_PER_BATCH</c>).</summary>
    public const int MaxFramesPerBatch = 512;

    /// <summary>Maximum number of entries in <see cref="PresentingProcessesMessage.Processes"/> (Rust: <c>MAX_PRESENTING_PROCESSES</c>).</summary>
    public const int MaxPresentingProcesses = 32;
}

/// <summary>Values of <see cref="FramesStatusMessage.State"/> (Rust: <c>frames_state</c>).</summary>
public static class FramesStates
{
    public const string Off = "off";
    public const string Starting = "starting";
    public const string Running = "running";
    public const string Denied = "denied";
    public const string Tampered = "tampered";
    public const string Missing = "missing";
    public const string Failed = "failed";
}
