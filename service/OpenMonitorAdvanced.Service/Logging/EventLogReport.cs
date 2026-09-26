using System.Runtime.InteropServices;

namespace OpenMonitorAdvanced.Service.Logging;

/// <summary>
/// One warning in the Application event log, for when file logging is disabled and the service
/// has no console. The source is not registered (that would leave a registry key the uninstaller
/// does not remove), so the Event Viewer shows the text after a "description cannot be found"
/// preamble. Best effort: any failure is ignored.
/// </summary>
internal static class EventLogReport
{
    private const ushort EventlogWarningType = 0x0002;

    internal static void Warning(string source, string message)
    {
        try
        {
            IntPtr log = RegisterEventSourceW(null, source);
            if (log == IntPtr.Zero)
            {
                return;
            }

            try
            {
                ReportEventW(log, EventlogWarningType, 0, 1, IntPtr.Zero, 1, 0, [message], IntPtr.Zero);
            }
            finally
            {
                DeregisterEventSource(log);
            }
        }
        catch
        {
            // Nowhere left to report to.
        }
    }

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr RegisterEventSourceW(string? lpUNCServerName, string lpSourceName);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool ReportEventW(
        IntPtr hEventLog,
        ushort wType,
        ushort wCategory,
        uint dwEventID,
        IntPtr lpUserSid,
        ushort wNumStrings,
        uint dwDataSize,
        [MarshalAs(UnmanagedType.LPArray, ArraySubType = UnmanagedType.LPWStr)] string[] lpStrings,
        IntPtr lpRawData);

    [DllImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DeregisterEventSource(IntPtr hEventLog);
}
