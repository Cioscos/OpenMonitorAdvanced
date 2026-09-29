using System.Globalization;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>What the service can say about the PawnIO driver (spec M5 §2.8, F3.3).</summary>
public enum PawnIoStatus
{
    /// <summary>The device opens: the PawnIO-backed sensors work.</summary>
    Ok,

    /// <summary>Not installed.</summary>
    Missing,

    /// <summary>Installed but not usable (driver stopped or blocked, or access denied).</summary>
    Unavailable,

    /// <summary>The probe could not tell.</summary>
    Unknown,

    /// <summary>Installed by our setup, which asked for a restart that has not happened yet.</summary>
    RebootPending,
}

/// <summary>Whether the PawnIO uninstall key could be read, and if so whether it exists.</summary>
public enum KeyState
{
    Present,
    Absent,
    Unreadable,
}

/// <summary>The pure part of the PawnIO probe: classification (F3.3 with ruling P11) and marker parsing.</summary>
public static class PawnIoClassifier
{
    private const int ErrorFileNotFound = 2;
    private const int ErrorPathNotFound = 3;

    /// <summary>
    /// Maps the probe's evidence to a status. <paramref name="openError"/> is <see langword="null"/>
    /// when the device opened, else its Win32 error. <paramref name="markerUtc"/> is the installer's
    /// <c>PawnIoRebootRequestedUtc</c> and <paramref name="bootUtc"/> the current boot time, both
    /// FILETIME UTC; only a marker newer than the boot counts as a restart still to come.
    /// </summary>
    public static PawnIoStatus Classify(KeyState key, int? openError, long? markerUtc, long bootUtc)
    {
        if (openError is null)
        {
            return PawnIoStatus.Ok; // P11: the device is what matters, the uninstall key is only logged.
        }

        bool deviceAbsent = openError is ErrorFileNotFound or ErrorPathNotFound;
        return (key, deviceAbsent) switch
        {
            (KeyState.Absent, true) => PawnIoStatus.Missing,
            (KeyState.Present, true) => markerUtc is { } marker && marker > bootUtc
                ? PawnIoStatus.RebootPending
                : PawnIoStatus.Unavailable,
            (KeyState.Present, false) => PawnIoStatus.Unavailable,
            _ => PawnIoStatus.Unknown,
        };
    }

    /// <summary>The marker as a positive decimal FILETIME, or <see langword="null"/> (absent, not a number, not positive).</summary>
    public static long? ParseMarker(string? raw) =>
        long.TryParse(raw, NumberStyles.None | NumberStyles.AllowLeadingWhite | NumberStyles.AllowTrailingWhite, CultureInfo.InvariantCulture, out long value) && value > 0
            ? value
            : null;

    /// <summary>The status as the protocol names it (<c>Hello.pawn_io</c>).</summary>
    public static string ToWire(PawnIoStatus status) => status switch
    {
        PawnIoStatus.Ok => "ok",
        PawnIoStatus.Missing => "missing",
        PawnIoStatus.Unavailable => "unavailable",
        PawnIoStatus.Unknown => "unknown",
        PawnIoStatus.RebootPending => "rebootPending",
        _ => "unknown",
    };
}

/// <summary>
/// The PawnIO status, computed once per process on first use and shared by the hub (which asks
/// whether PawnIO-backed sensors may be published) and by every client's <c>Hello</c>. The
/// service starts on demand and leaves after two idle minutes, so a change shows at the next start.
/// </summary>
public sealed class PawnIoState(Func<PawnIoStatus> probe)
{
    private readonly Lazy<PawnIoStatus> _status = new(probe, LazyThreadSafetyMode.ExecutionAndPublication);

    public PawnIoStatus Status => _status.Value;
}
