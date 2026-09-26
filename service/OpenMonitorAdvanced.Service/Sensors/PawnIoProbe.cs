using System.Runtime.InteropServices;
using System.Security;
using Microsoft.Extensions.Logging;
using Microsoft.Win32;
using Microsoft.Win32.SafeHandles;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// Whether PawnIO is usable: its uninstall key exists (64-bit registry view) and its device
/// opens. Without it LHM does not fail, it returns plausible zeros (s1-lhm.md §2), so the schema
/// leaves out every PawnIO-backed sensor. The hub asks once, when it opens the tree, and the
/// result is logged. Needs administrator rights to be true; verified live in Task 15.
/// </summary>
public sealed class PawnIoProbe(ILogger<PawnIoProbe> log)
{
    private const string UninstallKey = @"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO";
    private const string DevicePath = @"\\?\GLOBALROOT\Device\PawnIO";
    private const uint GenericRead = 0x80000000;
    private const uint GenericWrite = 0x40000000;
    private const uint FileShareReadWrite = 0x00000001 | 0x00000002;
    private const uint OpenExisting = 3;

    public bool IsAvailable()
    {
        bool installed = false;
        string? version = null;
        try
        {
            using RegistryKey hklm = RegistryKey.OpenBaseKey(RegistryHive.LocalMachine, RegistryView.Registry64);
            using RegistryKey? key = hklm.OpenSubKey(UninstallKey);
            installed = key is not null;
            version = key?.GetValue("DisplayVersion") as string;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or SecurityException)
        {
            log.LogWarning(e, "Reading the PawnIO uninstall key failed");
        }

        // SAFETY: opening the PawnIO device only creates a handle (no module is loaded, no I/O);
        // it is closed right away by the using declaration.
        using SafeFileHandle device = CreateFileW(DevicePath, GenericRead | GenericWrite, FileShareReadWrite, IntPtr.Zero, OpenExisting, 0, IntPtr.Zero);
        bool opens = !device.IsInvalid;
        int error = opens ? 0 : Marshal.GetLastPInvokeError();

        bool available = installed && opens;
        log.LogInformation(
            "PawnIO {Result}: uninstall key {Key} (version {Version}), device {Device} (Win32 error {Error})",
            available ? "available" : "unavailable",
            installed ? "present" : "absent",
            version ?? "unknown",
            opens ? "opens" : "does not open",
            error);
        return available;
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
