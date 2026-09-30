using System.Text;
using System.Text.RegularExpressions;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// The identity of a CPU as the CPUID leaves report it, plus the TjMax LHM read for an Intel CPU.
/// <paramref name="Vendor"/> is <c>"AMD"</c>, <c>"Intel"</c> or <c>"Unknown"</c>;
/// <paramref name="BrandString"/> is the raw CPUID brand string (not LHM's cleaned-up hardware
/// name); <paramref name="IntelTjMaxC"/> is the value of the "TjMax [°C]" parameter of the
/// "CPU Package" sensor, when there is one.
/// </summary>
public sealed record CpuInfo(string Vendor, string BrandString, int Family, int Model, double? IntelTjMaxC);

/// <summary>
/// Turns a <see cref="CpuInfo"/> into a TjMax for the <c>tjMaxC</c> device property. Rules of
/// <c>docs/superpowers/references/m5/s1-lhm-sensors.md</c> §3 (Intel) and §4.2 (AMD): when in
/// doubt there is no value, and the rules fall back to their fixed thresholds.
/// </summary>
public static partial class CpuIdentity
{
    // 0x0F (Core 2), 0x17 (Core 2 45 nm) and 0x1C (Atom 45 nm) take TjMax from fixed tables in LHM,
    // not from MSR_TEMPERATURE_TARGET.
    private static readonly int[] IntelTabulatedModels = [0x0F, 0x17, 0x1C];

    [GeneratedRegex(@"^AMD Ryzen (?<tier>[3579]) (?<pro>PRO )?(?<model>[0-9]{4}[A-Z0-9]*)(?<tail>.*)$")]
    private static partial Regex AmdBrandPattern();

    [GeneratedRegex(@"^( \d+-Core Processor| with Radeon( .+)? Graphics| w/ Radeon( .+)? Graphics| Processor| Dual Edition)?$")]
    private static partial Regex AmdBrandTailPattern();

    /// <summary>
    /// The table key of an AMD Ryzen brand string, e.g. <c>"AMD Ryzen 7 7800X3D 8-Core Processor"</c>
    /// gives <c>"Ryzen 7 7800X3D"</c>; null when the string is not one of the desktop shapes the
    /// table covers (engineering sample, Threadripper, Athlon, an unknown tail...).
    /// </summary>
    public static string? NormalizeAmdKey(string brandString)
    {
        Match m = AmdBrandPattern().Match(CleanBrand(brandString));
        if (!m.Success || !AmdBrandTailPattern().IsMatch(m.Groups["tail"].Value))
        {
            return null;
        }

        return $"Ryzen {m.Groups["tier"].Value} {m.Groups["pro"].Value}{m.Groups["model"].Value}";
    }

    /// <summary>
    /// The TjMax in °C, or null when it cannot be told: AMD needs a table entry whose family
    /// agrees with the CPUID family; Intel needs a value LHM read from the MSR (family 6, not one
    /// of the models with a fixed table).
    /// </summary>
    public static int? TjMaxC(CpuInfo cpu)
    {
        switch (cpu.Vendor)
        {
            case "AMD":
                string? key = NormalizeAmdKey(cpu.BrandString);
                if (key is null || !AmdTjMaxTable.TryGet(key, out int tjMax, out int? family))
                {
                    return null;
                }

                return family is int expected && expected != cpu.Family ? null : tjMax;

            case "Intel":
                if (cpu.Family != 6 || Array.IndexOf(IntelTabulatedModels, cpu.Model) >= 0)
                {
                    return null;
                }

                return cpu.IntelTjMaxC is double read && double.IsFinite(read) && read > 0 ? (int)Math.Round(read) : null;

            default:
                return null;
        }
    }

    /// <summary>
    /// Removes trademark marks, the replacement character and zero-width characters, collapses
    /// runs of white space (and NUL padding) into one space and trims. The comparison that follows
    /// is ordinal and case-sensitive.
    /// </summary>
    private static string CleanBrand(string brand)
    {
        string text = brand
            .Replace("®", string.Empty, StringComparison.Ordinal)
            .Replace("™", string.Empty, StringComparison.Ordinal)
            .Replace("(R)", string.Empty, StringComparison.Ordinal)
            .Replace("(TM)", string.Empty, StringComparison.Ordinal)
            .Replace("(tm)", string.Empty, StringComparison.Ordinal);

        var sb = new StringBuilder(text.Length);
        bool pendingSpace = false;
        foreach (char c in text)
        {
            if (c is '\uFFFD' or '\u200B' or '\u200C' or '\u200D' or '\u2060' or '\uFEFF')
            {
                continue;
            }

            if (char.IsWhiteSpace(c) || c == '\0')
            {
                pendingSpace = sb.Length > 0;
                continue;
            }

            if (pendingSpace)
            {
                sb.Append(' ');
                pendingSpace = false;
            }

            sb.Append(c);
        }

        return sb.ToString();
    }
}
