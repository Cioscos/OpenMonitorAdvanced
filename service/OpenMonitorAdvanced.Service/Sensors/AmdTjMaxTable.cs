namespace OpenMonitorAdvanced.Service.Sensors;

// Source: AMD's official processor specifications table
// (https://www.amd.com/en/products/specifications/processors.html), field
// "maxOperatingTemperatureTjmax" ("Max. Operating Temperature (Tjmax)"), read on 2026-09-30.
// Transcribed from docs/superpowers/references/m5/s1-lhm-sensors.md sections 4.4 and 4.5
// (164 desktop entries: consumer Ryzen 3000-9000 and Ryzen PRO desktop).
// The expected CPUID family is our own consistency check derived from the AMD codename
// (Matisse, Picasso and Renoir = 0x17; Vermeer, Cezanne, Raphael and Phoenix = 0x19;
// Granite Ridge = 0x1A), not an AMD datum; the only entry without a codename has none.
// Left out on purpose: Ryzen 1000/2000 (Tctl offset), Threadripper, mobile, Ryzen AI, and
// the Ryzen 3 3100 and 3300X, for which AMD publishes no value.

/// <summary>
/// The AMD TjMax table keyed by the normalized brand key (<see cref="CpuIdentity.NormalizeAmdKey"/>).
/// </summary>
internal static class AmdTjMaxTable
{
    /// <summary>Key to (TjMax in degrees Celsius, expected CPUID family or null for no family check).</summary>
    internal static IReadOnlyDictionary<string, (int TjMaxC, int? Family)> Entries { get; } =
        new Dictionary<string, (int TjMaxC, int? Family)>(StringComparer.Ordinal)
        {
            // Ryzen desktop, consumer
            ["Ryzen 3 3200G"] = (95, 0x17),
            ["Ryzen 3 3200GE"] = (95, 0x17),
            ["Ryzen 5 3400G"] = (95, 0x17),
            ["Ryzen 5 3400GE"] = (95, 0x17),
            ["Ryzen 5 3500"] = (95, 0x17),
            ["Ryzen 5 3600"] = (95, 0x17),
            ["Ryzen 5 3600X"] = (95, 0x17),
            ["Ryzen 5 3600XT"] = (95, 0x17),
            ["Ryzen 7 3700X"] = (95, 0x17),
            ["Ryzen 7 3800X"] = (95, 0x17),
            ["Ryzen 7 3800XT"] = (95, 0x17),
            ["Ryzen 9 3900"] = (95, 0x17),
            ["Ryzen 9 3900X"] = (95, 0x17),
            ["Ryzen 9 3900XT"] = (95, 0x17),
            ["Ryzen 9 3950X"] = (95, 0x17),
            ["Ryzen 3 4100"] = (95, 0x17),
            ["Ryzen 3 4300G"] = (95, 0x17),
            ["Ryzen 3 4300GE"] = (95, 0x17),
            ["Ryzen 5 4500"] = (95, 0x17),
            ["Ryzen 5 4600G"] = (95, 0x17),
            ["Ryzen 5 4600GE"] = (95, 0x17),
            ["Ryzen 7 4700G"] = (95, 0x17),
            ["Ryzen 7 4700GE"] = (95, 0x17),
            ["Ryzen 7 4700LE"] = (95, 0x17),
            ["Ryzen 3 5300G"] = (95, 0x19),
            ["Ryzen 3 5300GE"] = (95, 0x19),
            ["Ryzen 3 5305G"] = (95, 0x19),
            ["Ryzen 3 5305GE"] = (95, 0x19),
            ["Ryzen 5 5500"] = (90, 0x19),
            ["Ryzen 5 5500F"] = (95, 0x19),
            ["Ryzen 5 5500GT"] = (95, 0x19),
            ["Ryzen 5 5500X3D"] = (90, 0x19),
            ["Ryzen 5 5600"] = (90, 0x19),
            ["Ryzen 5 5600F"] = (95, 0x19),
            ["Ryzen 5 5600G"] = (95, 0x19),
            ["Ryzen 5 5600GE"] = (95, 0x19),
            ["Ryzen 5 5600GT"] = (95, 0x19),
            ["Ryzen 5 5600T"] = (95, 0x19),
            ["Ryzen 5 5600X"] = (95, 0x19),
            ["Ryzen 5 5600X3D"] = (90, 0x19),
            ["Ryzen 5 5600XT"] = (95, 0x19),
            ["Ryzen 5 5605G"] = (95, 0x19),
            ["Ryzen 5 5605GE"] = (95, 0x19),
            ["Ryzen 7 5700"] = (95, 0x19),
            ["Ryzen 7 5700G"] = (95, 0x19),
            ["Ryzen 7 5700GE"] = (95, 0x19),
            ["Ryzen 7 5700X"] = (90, 0x19),
            ["Ryzen 7 5700X3D"] = (90, 0x19),
            ["Ryzen 7 5705G"] = (95, 0x19),
            ["Ryzen 7 5705GE"] = (95, 0x19),
            ["Ryzen 7 5800"] = (95, 0x19),
            ["Ryzen 7 5800X"] = (90, 0x19),
            ["Ryzen 7 5800X3D"] = (90, 0x19),
            ["Ryzen 7 5800XT"] = (90, 0x19),
            ["Ryzen 9 5900"] = (95, 0x19),
            ["Ryzen 9 5900X"] = (90, 0x19),
            ["Ryzen 9 5900XT"] = (90, 0x19),
            ["Ryzen 9 5950X"] = (90, 0x19),
            ["Ryzen 5 7400"] = (95, 0x19),
            ["Ryzen 5 7400F"] = (95, 0x19),
            ["Ryzen 5 7500"] = (95, 0x19),
            ["Ryzen 5 7500F"] = (95, 0x19),
            ["Ryzen 5 7500X3D"] = (89, 0x19),
            ["Ryzen 5 7600"] = (95, 0x19),
            ["Ryzen 5 7600X"] = (95, 0x19),
            ["Ryzen 5 7600X3D"] = (89, 0x19),
            ["Ryzen 7 7700"] = (95, 0x19),
            ["Ryzen 7 7700X"] = (95, 0x19),
            ["Ryzen 7 7700X3D"] = (89, 0x19),
            ["Ryzen 7 7800X3D"] = (89, 0x19),
            ["Ryzen 9 7900"] = (95, 0x19),
            ["Ryzen 9 7900X"] = (95, 0x19),
            ["Ryzen 9 7900X3D"] = (89, 0x19),
            ["Ryzen 9 7950X"] = (95, 0x19),
            ["Ryzen 9 7950X3D"] = (89, 0x19),
            ["Ryzen 3 8300G"] = (95, 0x19),
            ["Ryzen 3 8300GE"] = (95, 0x19),
            ["Ryzen 3 8305G"] = (95, 0x19),
            ["Ryzen 3 8305GE"] = (95, 0x19),
            ["Ryzen 5 8400F"] = (95, 0x19),
            ["Ryzen 5 8500G"] = (95, 0x19),
            ["Ryzen 5 8500GE"] = (95, 0x19),
            ["Ryzen 5 8505G"] = (95, 0x19),
            ["Ryzen 5 8505GE"] = (95, 0x19),
            ["Ryzen 5 8600G"] = (95, 0x19),
            ["Ryzen 5 8605G"] = (95, 0x19),
            ["Ryzen 7 8700F"] = (95, 0x19),
            ["Ryzen 7 8700G"] = (95, 0x19),
            ["Ryzen 7 8705G"] = (95, 0x19),
            ["Ryzen 5 9500F"] = (95, 0x1A),
            ["Ryzen 5 9600"] = (95, 0x1A),
            ["Ryzen 5 9600X"] = (95, 0x1A),
            ["Ryzen 7 9700F"] = (95, 0x1A),
            ["Ryzen 7 9700X"] = (95, 0x1A),
            ["Ryzen 7 9800X3D"] = (95, 0x1A),
            ["Ryzen 7 9850X3D"] = (95, 0x1A),
            ["Ryzen 9 9900X"] = (95, 0x1A),
            ["Ryzen 9 9900X3D"] = (95, 0x1A),
            ["Ryzen 9 9950X"] = (95, 0x1A),
            ["Ryzen 9 9950X3D"] = (95, 0x1A),
            ["Ryzen 9 9950X3D2"] = (95, 0x1A),
            // Ryzen PRO desktop
            ["Ryzen 3 PRO 3200G"] = (95, 0x17),
            ["Ryzen 3 PRO 3200GE"] = (95, 0x17),
            ["Ryzen 5 PRO 3350G"] = (95, 0x17),
            ["Ryzen 5 PRO 3350GE"] = (95, null),
            ["Ryzen 5 PRO 3400G"] = (95, 0x17),
            ["Ryzen 5 PRO 3400GE"] = (95, 0x17),
            ["Ryzen 5 PRO 3600"] = (95, 0x17),
            ["Ryzen 7 PRO 3700"] = (95, 0x17),
            ["Ryzen 9 PRO 3900"] = (95, 0x17),
            ["Ryzen 3 PRO 4350G"] = (95, 0x17),
            ["Ryzen 3 PRO 4350GE"] = (95, 0x17),
            ["Ryzen 3 PRO 4355G"] = (95, 0x17),
            ["Ryzen 3 PRO 4355GE"] = (95, 0x17),
            ["Ryzen 5 PRO 4650G"] = (95, 0x17),
            ["Ryzen 5 PRO 4650GE"] = (95, 0x17),
            ["Ryzen 5 PRO 4655G"] = (95, 0x17),
            ["Ryzen 5 PRO 4655GE"] = (95, 0x17),
            ["Ryzen 7 PRO 4750G"] = (95, 0x17),
            ["Ryzen 7 PRO 4750GE"] = (95, 0x17),
            ["Ryzen 3 PRO 5350G"] = (95, 0x19),
            ["Ryzen 3 PRO 5350GE"] = (95, 0x19),
            ["Ryzen 3 PRO 5355G"] = (95, 0x19),
            ["Ryzen 3 PRO 5355GE"] = (95, 0x19),
            ["Ryzen 5 PRO 5645"] = (95, 0x19),
            ["Ryzen 5 PRO 5650G"] = (95, 0x19),
            ["Ryzen 5 PRO 5650GE"] = (95, 0x19),
            ["Ryzen 5 PRO 5655G"] = (95, 0x19),
            ["Ryzen 5 PRO 5655GE"] = (95, 0x19),
            ["Ryzen 7 PRO 5750G"] = (95, 0x19),
            ["Ryzen 7 PRO 5750GE"] = (95, 0x19),
            ["Ryzen 7 PRO 5755G"] = (95, 0x19),
            ["Ryzen 7 PRO 5755GE"] = (95, 0x19),
            ["Ryzen 7 PRO 5845"] = (95, 0x19),
            ["Ryzen 9 PRO 5945"] = (95, 0x19),
            ["Ryzen 5 PRO 7445"] = (95, 0x19),
            ["Ryzen 5 PRO 7645"] = (95, 0x19),
            ["Ryzen 7 PRO 7745"] = (95, 0x19),
            ["Ryzen 9 PRO 7945"] = (95, 0x19),
            ["Ryzen 3 PRO 8300G"] = (95, 0x19),
            ["Ryzen 3 PRO 8300GE"] = (95, 0x19),
            ["Ryzen 3 PRO 8305G"] = (95, 0x19),
            ["Ryzen 3 PRO 8305GE"] = (95, 0x19),
            ["Ryzen 5 PRO 8500G"] = (95, 0x19),
            ["Ryzen 5 PRO 8500GE"] = (95, 0x19),
            ["Ryzen 5 PRO 8505G"] = (95, 0x19),
            ["Ryzen 5 PRO 8505GE"] = (95, 0x19),
            ["Ryzen 5 PRO 8600G"] = (95, 0x19),
            ["Ryzen 5 PRO 8600GE"] = (95, 0x19),
            ["Ryzen 5 PRO 8605G"] = (95, 0x19),
            ["Ryzen 5 PRO 8605GE"] = (95, 0x19),
            ["Ryzen 7 PRO 8700G"] = (95, 0x19),
            ["Ryzen 7 PRO 8700GE"] = (95, 0x19),
            ["Ryzen 7 PRO 8705G"] = (95, 0x19),
            ["Ryzen 7 PRO 8705GE"] = (95, 0x19),
            ["Ryzen 5 PRO 9645"] = (95, 0x1A),
            ["Ryzen 5 PRO 9655"] = (95, 0x1A),
            ["Ryzen 7 PRO 9745"] = (95, 0x1A),
            ["Ryzen 7 PRO 9755"] = (95, 0x1A),
            ["Ryzen 7 PRO 9755X3D"] = (95, 0x1A),
            ["Ryzen 9 PRO 9945"] = (95, 0x1A),
            ["Ryzen 9 PRO 9955"] = (95, 0x1A),
            ["Ryzen 9 PRO 9965"] = (95, 0x1A),
            ["Ryzen 9 PRO 9965X3D"] = (95, 0x1A),
        };

    /// <summary>
    /// Looks <paramref name="key"/> up; <paramref name="family"/> is null for the documented
    /// entry that has no family check.
    /// </summary>
    internal static bool TryGet(string key, out int tjMaxC, out int? family)
    {
        if (Entries.TryGetValue(key, out (int TjMaxC, int? Family) entry))
        {
            tjMaxC = entry.TjMaxC;
            family = entry.Family;
            return true;
        }

        tjMaxC = 0;
        family = null;
        return false;
    }
}
