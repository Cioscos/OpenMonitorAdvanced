namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// Cleans names LibreHardwareMonitor hands out (some pad them with spaces and NULs) for display
/// only. Never apply it to identities: device ids, <c>DriveKey</c>, identity hints and sensor
/// name comparisons keep the original text, so per-drive settings and ids do not change.
/// </summary>
internal static class DisplayName
{
    internal static string Clean(string? name)
    {
        if (string.IsNullOrEmpty(name))
        {
            return "";
        }

        return string.Concat(name.Where(static c => !char.IsControl(c))).Trim();
    }
}
