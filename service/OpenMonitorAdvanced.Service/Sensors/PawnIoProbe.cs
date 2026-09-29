using System.Runtime.InteropServices;
using System.Security;
using Microsoft.Extensions.Logging;
using Microsoft.Win32;
using Microsoft.Win32.SafeHandles;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// Gathers the evidence on PawnIO and classifies it (<see cref="PawnIoClassifier"/>): its uninstall
/// key (64-bit registry view), whether its device opens and with which Win32 error, and the
/// installer's reboot marker. Without PawnIO LHM does not fail, it returns plausible zeros
/// (s1-lhm.md §2), so the schema leaves out every PawnIO-backed sensor unless the status is
/// <see cref="PawnIoStatus.Ok"/>. Runs once per process through <see cref="PawnIoState"/>; the
/// result is logged. Needs administrator rights to reach <c>Ok</c>; verified live in Task 15.
/// </summary>
public sealed class PawnIoProbe(ILogger<PawnIoProbe> log)
{
    private const string UninstallKey = @"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO";
    private const string MarkerKey = @"SOFTWARE\OpenMonitorAdvanced";
    private const string MarkerValue = "PawnIoRebootRequestedUtc";
    private const string DevicePath = @"\\?\GLOBALROOT\Device\PawnIO";
    private const uint GenericRead = 0x80000000;
    private const uint GenericWrite = 0x40000000;
    private const uint FileShareReadWrite = 0x00000001 | 0x00000002;
    private const uint OpenExisting = 3;

    public PawnIoStatus Probe()
    {
        KeyState keyState = KeyState.Absent;
        string? version = null;
        long? marker = null;
        try
        {
            using RegistryKey hklm = RegistryKey.OpenBaseKey(RegistryHive.LocalMachine, RegistryView.Registry64);
            using (RegistryKey? key = hklm.OpenSubKey(UninstallKey))
            {
                keyState = key is not null ? KeyState.Present : KeyState.Absent;
                version = key?.GetValue("DisplayVersion") as string;
            }

            using RegistryKey? markerKey = hklm.OpenSubKey(MarkerKey);
            string? raw = markerKey?.GetValue(MarkerValue) as string;
            marker = PawnIoClassifier.ParseMarker(raw);
            if (raw is not null && marker is null)
            {
                log.LogWarning("The PawnIO reboot marker {Value} is not a FILETIME; ignored", raw);
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or SecurityException)
        {
            keyState = KeyState.Unreadable;
            log.LogWarning(e, "Reading the PawnIO registry entries failed");
        }

        // SAFETY: opening the PawnIO device only creates a handle (no module is loaded, no I/O);
        // it is closed right away by the using declaration.
        using SafeFileHandle device = CreateFileW(DevicePath, GenericRead | GenericWrite, FileShareReadWrite, IntPtr.Zero, OpenExisting, 0, IntPtr.Zero);
        bool opens = !device.IsInvalid;
        int? error = opens ? null : Marshal.GetLastPInvokeError();

        // GetTickCount64 includes sleep and hibernation, and a fast-startup shutdown does not apply a
        // pending driver, so a marker newer than this boot is still waiting for a restart.
        long bootUtc = (DateTime.UtcNow - TimeSpan.FromMilliseconds(Environment.TickCount64)).ToFileTimeUtc();
        PawnIoStatus status = PawnIoClassifier.Classify(keyState, error, marker, bootUtc);
        log.LogInformation(
            "PawnIO {Status}: uninstall key {Key} (version {Version}), device {Device} (Win32 error {Error}), reboot marker {Marker}",
            PawnIoClassifier.ToWire(status),
            keyState,
            version ?? "unknown",
            opens ? "opens" : "does not open",
            error ?? 0,
            marker is null ? "absent" : marker > bootUtc ? "of this boot" : "of an earlier boot");
        return status;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeFileHandle CreateFileW(
        string lpFileName,
        uint dwDesiredAccess,
        uint dwShareMode,
        IntPtr lpSecurityAttributes,
        uint dwCreationDisposition,
        uint dwFlagsAndAttributes,
        IntPtr hTemplateFile);
}
