using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// Structural comparison of two built schemas: devices and sensors in order with every field,
/// device properties as an unordered (key-sorted) set, the bindings and the service block. Record equality alone
/// would compare the <see cref="IReadOnlyList{T}"/>/<see cref="IReadOnlyDictionary{TKey,TValue}"/>
/// members by reference.
/// </summary>
internal static class SchemaComparer
{
    public static bool SameStructure(BuiltSchema a, BuiltSchema b)
    {
        if (a.Schema.Devices.Count != b.Schema.Devices.Count
            || !a.Schema.Sensors.SequenceEqual(b.Schema.Sensors)
            || !a.Bindings.SequenceEqual(b.Bindings)
            || !SameServiceState(a.Schema.Service, b.Schema.Service))
        {
            return false;
        }

        for (int i = 0; i < a.Schema.Devices.Count; i++)
        {
            WireDevice x = a.Schema.Devices[i];
            WireDevice y = b.Schema.Devices[i];
            if (x.Id != y.Id || x.Kind != y.Kind || x.Name != y.Name || x.Vendor != y.Vendor || !Equals(x.Hint, y.Hint) || !SameProperties(x.Properties, y.Properties))
            {
                return false;
            }
        }

        return true;
    }

    /// <summary>The service block counts as structure (spec M5 §2.8): a change is a new revision.</summary>
    public static bool SameServiceState(ServiceStateBlock x, ServiceStateBlock y) =>
        x.Reconfiguration == y.Reconfiguration
        && x.ActiveModules.SequenceEqual(y.ActiveModules)
        && x.SmartDisabledDrives.SequenceEqual(y.SmartDisabledDrives)
        && x.Drives.SequenceEqual(y.Drives);

    private static bool SameProperties(IReadOnlyDictionary<string, string> x, IReadOnlyDictionary<string, string> y) =>
        x.Count == y.Count
        && x.OrderBy(p => p.Key, StringComparer.Ordinal).SequenceEqual(y.OrderBy(p => p.Key, StringComparer.Ordinal));
}
