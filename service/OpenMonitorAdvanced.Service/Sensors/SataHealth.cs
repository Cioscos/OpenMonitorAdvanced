namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>One ATA SMART attribute as the health rules need it; ours, so the rules stay free of the SMART library.</summary>
public readonly record struct AtaAttribute(byte Id, byte Current, byte Threshold, ulong Raw);

/// <summary>
/// The CrystalDiskInfo-style verdict of a SATA disk: <see cref="Critical"/> when an attribute fell to
/// its threshold, <see cref="SectorWarning"/> when reallocated, pending or uncorrectable sectors exist.
/// A counter is <see langword="null"/> when the disk has no such attribute.
/// </summary>
public sealed record SataHealthResult(bool Critical, bool SectorWarning, uint? Reallocated, uint? Pending, uint? Uncorrectable);

/// <summary>Pure rules over a disk's ATA attributes (spec M9b §6).</summary>
public static class SataHealth
{
    public const byte ReallocatedId = 0x05;
    public const byte PendingId = 0xC5;
    public const byte UncorrectableId = 0xC6;

    public static SataHealthResult Evaluate(IReadOnlyList<AtaAttribute> attributes)
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

        bool warn = reallocated > 0 || pending > 0 || uncorrectable > 0;
        return new SataHealthResult(critical, warn, reallocated, pending, uncorrectable);
    }
}
