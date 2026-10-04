using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// The per-drive entries of the schema's service block (design M6b §3.1, §4.2): which drives
/// have their SMART off and the state each one reports. Pure.
/// </summary>
internal static class DriveStates
{
    internal const string Active = "active";
    internal const string Standby = "standby";
    internal const string Idle = "idle";
    internal const string Unknown = "unknown";
    internal const string SmartOff = "smartOff";
    internal const string NoMedia = "noMedia";

    /// <summary>
    /// Precedence: <c>noMedia</c>, <c>smartOff</c>, <c>standby</c> (Windows reports the disk off,
    /// or the disk answered standby), <c>idle</c> (it needs a power check and was not asked in
    /// this round, for lack of recent activity), then <c>active</c>/<c>unknown</c>: a drive that
    /// was not asked is <c>active</c> only when <see cref="DriveFacts.RequiresPowerCheck"/> is
    /// false; otherwise it is <c>unknown</c>.
    /// </summary>
    internal static string Of(DriveCheck check, bool smartOff)
    {
        if (check.Drive.Availability == DriveAvailability.NoMedia)
        {
            return NoMedia;
        }

        if (smartOff)
        {
            return SmartOff;
        }

        if (check.PoweredOff)
        {
            return Standby;
        }

        if (check.Idle)
        {
            return Idle;
        }

        if (!check.Asked)
        {
            return check.Drive.RequiresPowerCheck ? Unknown : Active;
        }

        return check.SpunDown switch
        {
            true => Standby,
            false => Active,
            null => Unknown,
        };
    }

    /// <summary>
    /// Whether the service leaves that drive alone after the first identification: storage is
    /// off, its <paramref name="key"/> is among the disabled ones, or it is off by default and
    /// its key is not among the enabled ones (a drive without a key cannot be switched on).
    /// </summary>
    internal static bool IsSmartOff(DriveFacts facts, string? key, EffectiveConfig config) =>
        !config.Enabled.HasFlag(ServiceModules.Storage)
        || (key is not null && config.SmartDisabledDrives.Contains(key))
        || (facts.SmartOffByDefault && (key is null || !config.SmartEnabledDrives.Contains(key)));

    /// <summary>
    /// The drive list of one storage round, in drive order. <paramref name="gateOpen"/> is false
    /// until LHM's storage group has identified the disks: only then can a drive block SMART.
    /// </summary>
    internal static WireDrive[] ToWire(IEnumerable<DriveCheck> checks, EffectiveConfig config, bool gateOpen) =>
    [
        .. checks
            .OrderBy(c => c.Drive.DriveNumber)
            .Select(c => new WireDrive(
                (uint)c.Drive.DriveNumber,
                c.Key,
                c.Drive.Model,
                Of(c, IsSmartOff(c.Drive, c.Key, config)),
                BlocksSmart: !gateOpen && c.Blocks)),
    ];

    /// <summary>The same drives with storage switched off: none is queried and none waits at a gate.</summary>
    internal static WireDrive[] AllOff(IEnumerable<WireDrive> drives) =>
        [.. drives.Select(d => d with { State = SmartOff, BlocksSmart = false })];
}
