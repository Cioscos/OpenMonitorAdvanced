namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>One ATA SMART attribute as the health rules need it; ours, so the rules stay free of the SMART library.</summary>
public readonly record struct AtaAttribute(byte Id, byte Current, byte Threshold, ulong Raw);

/// <summary>
/// The CrystalDiskInfo-style verdict of a SATA disk: <see cref="Critical"/> when an attribute fell to
/// its threshold, <see cref="SectorWarning"/> when a rotational disk has reallocated, pending or
/// uncorrectable sectors. A counter is <see langword="null"/> when the disk has no such attribute.
/// </summary>
public sealed record SataHealthResult(bool Critical, bool SectorWarning, uint? Reallocated, uint? Pending, uint? Uncorrectable);

/// <summary>Pure rules over a disk's ATA attributes (spec M9b §6).</summary>
public static class SataHealth
{
    public const byte ReallocatedId = 0x05;
    public const byte PendingId = 0xC5;
    public const byte UncorrectableId = 0xC6;

    /// <param name="rotational">
    /// Whether the disk may have a platter (seek penalty true or unknown). Only then do the sector
    /// counters warn: on a SATA SSD 05h counts retired NAND blocks, normal wear that
    /// CrystalDiskInfo also rates "Good" (user ruling, 2026-10-11). The counters are reported either way.
    /// </param>
    public static SataHealthResult Evaluate(bool rotational, IReadOnlyList<AtaAttribute> attributes)
    {
        bool critical = false;
        uint? reallocated = null, pending = null, uncorrectable = null;
        foreach (AtaAttribute a in attributes)
        {
            critical |= a.Threshold > 0 && a.Current <= a.Threshold;
            // The low 32 bits only: vendors keep other data (temperature history, flags) above them.
            uint count = (uint)(a.Raw & 0xFFFF_FFFF);
            switch (a.Id)
            {
                case ReallocatedId: reallocated = count; break;
                case PendingId: pending = count; break;
                case UncorrectableId: uncorrectable = count; break;
            }
        }

        bool warn = rotational && (reallocated > 0 || pending > 0 || uncorrectable > 0);
        return new SataHealthResult(critical, warn, reallocated, pending, uncorrectable);
    }
}
