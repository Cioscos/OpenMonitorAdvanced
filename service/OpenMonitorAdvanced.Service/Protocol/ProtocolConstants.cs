namespace OpenMonitorAdvanced.Service.Protocol;

/// <summary>
/// Constants shared by both ends of the sensor IPC protocol (spec §6).
/// These values must stay in lockstep with the Rust crate <c>oma-ipc</c>
/// (<c>crates/oma-ipc/src/lib.rs</c>).
/// </summary>
public static class ProtocolConstants
{
    /// <summary>Current sensor IPC protocol version, sent in <see cref="Hello.ProtocolVersion"/>.</summary>
    public const uint Version = 1;

    /// <summary>Name of the sensor named pipe.</summary>
    public const string PipeName = "OpenMonitorAdvanced.Sensors.v1";

    /// <summary>Maximum size, in bytes, of a single frame's MessagePack payload.</summary>
    public const int MaxFrameBytes = 4 * 1024 * 1024;

    /// <summary>Minimum accepted <see cref="Subscribe.IntervalMs"/>.</summary>
    public const uint MinIntervalMs = 250;

    /// <summary>Maximum accepted <see cref="Subscribe.IntervalMs"/>.</summary>
    public const uint MaxIntervalMs = 5000;
}
