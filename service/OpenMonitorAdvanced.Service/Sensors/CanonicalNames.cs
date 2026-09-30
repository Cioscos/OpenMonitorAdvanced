namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// Every <c>label_key</c> value <see cref="SchemaBuilder.Build"/> can emit, i.e. every
/// key that must exist as <c>sensor.&lt;key&gt;</c> in both i18n catalogs
/// (<c>app/src/lib/i18n/en.json</c>, <c>it.json</c>). See
/// <c>docs/superpowers/references/m4/s1-lhm.md</c> §9.7 for the source mapping table.
/// </summary>
public static class CanonicalNames
{
    public static IReadOnlyList<string> LabelKeys { get; } =
    [
        "cpu.load.total",
        "cpu.load.coreMax",
        "cpu.temperature.tctl",
        "cpu.temperature.ccd",
        "cpu.temperature.package",
        "cpu.temperature.core",
        "cpu.temperature.tdie",
        "cpu.temperature.pCore",
        "cpu.temperature.eCore",
        "cpu.temperature.coreMax",
        "cpu.temperature.coreAverage",
        "cpu.power.package",
        "cpu.power.core",
        "cpu.power.soc",
        "cpu.voltage.soc",
        "cpu.voltage.coreVid",
        "cpu.clock.bus",
        "cpu.clock.average",
        "cpu.clock.averageEffective",
        "cpu.clock.core",
        "cpu.clock.pCore",
        "cpu.clock.eCore",
        "cpu.clock.coreEffective",
        "memory.load",
        "memory.used",
        "memory.temperature.dimm",
        "storage.temperature",
        "storage.temperatureSensor",
        "storage.active",
        "storage.read",
        "storage.write",
        "storage.life",
        "storage.availableSpare",
        "storage.percentUsed",
        "storage.criticalWarning",
        "storage.hostRead",
        "storage.hostWritten",
        "storage.powerOnHours",
        "storage.powerCycles",
        "lhm.raw",
    ];
}
