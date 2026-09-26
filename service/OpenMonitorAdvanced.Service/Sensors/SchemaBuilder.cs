using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using LibreHardwareMonitor.Hardware;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// Pure translation from a read-only <see cref="HardwareNode"/> tree to the wire
/// schema (<see cref="SchemaMessage"/>). No LHM, no I/O: <see cref="DriveDescriptor"/>
/// and the actual LHM sampling live in the sampling hub (Task 6). See
/// <c>docs/superpowers/references/m4/s1-lhm.md</c> §9.7 for the canonical mapping table
/// this class implements.
/// </summary>
public static partial class SchemaBuilder
{
    private const double TwoPow30 = 1024d * 1024d * 1024d;
    private const double TwoPow20 = 1024d * 1024d;

    private static readonly IReadOnlyDictionary<string, string> EmptyProperties = new Dictionary<string, string>();

    /// <summary>Unit (and, for Data/SmallData, the LHM-to-bytes scale) for every SensorType this service ever accepts.</summary>
    private static readonly IReadOnlyDictionary<SensorType, (string Kind, string Unit, double Scale)> FallbackByType =
        new Dictionary<SensorType, (string, string, double)>
        {
            [SensorType.Temperature] = ("temperature", "celsius", 1.0),
            [SensorType.Load] = ("load", "percent", 1.0),
            [SensorType.Clock] = ("clock", "megahertz", 1.0),
            [SensorType.Power] = ("power", "watt", 1.0),
            [SensorType.Voltage] = ("voltage", "volt", 1.0),
            [SensorType.Current] = ("current", "ampere", 1.0),
            [SensorType.Fan] = ("fan", "rpm", 1.0),
            [SensorType.Throughput] = ("throughput", "bytes_per_second", 1.0),
            [SensorType.Level] = ("percent", "percent", 1.0),
            [SensorType.Data] = ("data", "bytes", TwoPow30),
            [SensorType.SmallData] = ("data", "bytes", TwoPow20),
        };

    private static readonly IReadOnlyDictionary<string, string> UnitByKind =
        FallbackByType.Values.GroupBy(v => v.Kind).ToDictionary(g => g.Key, g => g.First().Unit);

    [GeneratedRegex(@"^CPU Core #\d+")]
    private static partial Regex CpuCorePattern();

    [GeneratedRegex(@"^CCD(\d+) \(Tdie\)$")]
    private static partial Regex CcdPattern();

    [GeneratedRegex(@"^Core #(\d+)$")]
    private static partial Regex CoreNumberPattern();

    [GeneratedRegex(@"^Core #(\d+) \(SMU\)$")]
    private static partial Regex CorePowerSmuPattern();

    [GeneratedRegex(@"^Core #(\d+) VID$")]
    private static partial Regex CoreVidPattern();

    [GeneratedRegex(@"^Core #(\d+) \(Effective\)$")]
    private static partial Regex CoreEffectiveClockPattern();

    [GeneratedRegex(@"^Temperature #(\d+)$")]
    private static partial Regex StorageTemperatureSensorPattern();

    [GeneratedRegex(@"^DIMM #\d+$")]
    private static partial Regex DimmTemperaturePattern();

    public static BuiltSchema Build(IReadOnlyList<HardwareNode> roots, bool pawnIoAvailable)
    {
        var devices = new List<WireDevice>();
        var sensors = new List<WireSensor>();
        var bindings = new List<SensorBinding>();

        BuildMemoryDevice(roots, pawnIoAvailable, devices, sensors, bindings);
        BuildStorageDevices(roots, devices, sensors, bindings);

        foreach (HardwareNode root in roots)
        {
            switch (root.Type)
            {
                case HardwareType.Cpu:
                    if (pawnIoAvailable)
                    {
                        BuildCpuDevice(root, devices, sensors, bindings);
                    }

                    break;

                case HardwareType.Motherboard:
                    if (pawnIoAvailable)
                    {
                        BuildSuperIoDevices(root, devices, sensors, bindings);
                    }

                    break;

                case HardwareType.Cooler:
                    ProcessDevice(root, DeviceId(root.Identifier), "fan_controller", root.Name, hint: null, GenericFallback, devices, sensors, bindings);
                    break;

                case HardwareType.Psu:
                    ProcessDevice(root, DeviceId(root.Identifier), "psu", root.Name, hint: null, GenericFallback, devices, sensors, bindings);
                    break;

                default:
                    // Memory and Storage are handled above (they need cross-root logic:
                    // DIMMs merge into /ram, disks need machine-wide serial uniqueness).
                    // Every other HardwareType (GpuNvidia/GpuAmd/GpuIntel, Network, Battery,
                    // PowerMonitor) is out of scope for the LHM mapping: those sources are
                    // either owned by another provider or not enabled on the LHM Computer
                    // instance the sampling hub (Task 6) constructs.
                    break;
            }
        }

        return new BuiltSchema(new SchemaMessage(devices, sensors), bindings);
    }

    private static void BuildCpuDevice(HardwareNode cpu, List<WireDevice> devices, List<WireSensor> sensors, List<SensorBinding> bindings)
    {
        uint index = ParseTrailingIndex(cpu.Identifier);
        ProcessDevice(cpu, DeviceId(cpu.Identifier), "cpu", cpu.Name, new CpuHint(index), s => Resolve(MatchCpuSensor, s), devices, sensors, bindings);
    }

    private static SensorMatch MatchCpuSensor(SensorNode s)
    {
        switch (s.Type)
        {
            case SensorType.Load:
                if (s.Name == "CPU Total")
                {
                    return Include("load", "total", "cpu.load.total");
                }

                if (s.Name == "CPU Core Max")
                {
                    return Include("load", "core-max", "cpu.load.coreMax");
                }

                if (CpuCorePattern().IsMatch(s.Name))
                {
                    return Discard();
                }

                return NoMatch();

            case SensorType.Temperature:
                if (s.Name == "Core (Tctl/Tdie)")
                {
                    return Include("temperature", "tctl", "cpu.temperature.tctl");
                }

                Match ccd = CcdPattern().Match(s.Name);
                if (ccd.Success)
                {
                    return Include("temperature", $"ccd-{ccd.Groups[1].Value}", "cpu.temperature.ccd", ccd.Groups[1].Value);
                }

                if (s.Name == "CPU Package")
                {
                    return Include("temperature", "package", "cpu.temperature.package");
                }

                Match coreTemp = CoreNumberPattern().Match(s.Name);
                if (coreTemp.Success)
                {
                    return Include("temperature", $"core-{coreTemp.Groups[1].Value}", "cpu.temperature.core", coreTemp.Groups[1].Value);
                }

                return NoMatch();

            case SensorType.Power:
                if (s.Name == "Package")
                {
                    return Include("power", "package", "cpu.power.package");
                }

                Match smu = CorePowerSmuPattern().Match(s.Name);
                if (smu.Success)
                {
                    return Include("power", $"core-{smu.Groups[1].Value}", "cpu.power.core", smu.Groups[1].Value);
                }

                if (s.Name == "SoC")
                {
                    return Include("power", "soc", "cpu.power.soc");
                }

                return NoMatch();

            case SensorType.Voltage:
                if (s.Name == "SoC")
                {
                    return Include("voltage", "soc", "cpu.voltage.soc");
                }

                Match vid = CoreVidPattern().Match(s.Name);
                if (vid.Success)
                {
                    return Include("voltage", $"core-{vid.Groups[1].Value}-vid", "cpu.voltage.coreVid", vid.Groups[1].Value);
                }

                return NoMatch();

            case SensorType.Clock:
                if (s.Name == "Bus Speed")
                {
                    return Include("clock", "bus", "cpu.clock.bus");
                }

                if (s.Name == "Cores (Average)")
                {
                    return Include("clock", "average", "cpu.clock.average");
                }

                if (s.Name == "Cores (Average Effective)")
                {
                    return Include("clock", "average-effective", "cpu.clock.averageEffective");
                }

                Match effClock = CoreEffectiveClockPattern().Match(s.Name);
                if (effClock.Success)
                {
                    return Include("clock", $"core-{effClock.Groups[1].Value}-effective", "cpu.clock.coreEffective", effClock.Groups[1].Value);
                }

                Match clock = CoreNumberPattern().Match(s.Name);
                if (clock.Success)
                {
                    return Include("clock", $"core-{clock.Groups[1].Value}", "cpu.clock.core", clock.Groups[1].Value);
                }

                return NoMatch();

            default:
                return NoMatch();
        }
    }

    private static void BuildMemoryDevice(IReadOnlyList<HardwareNode> roots, bool pawnIoAvailable, List<WireDevice> devices, List<WireSensor> sensors, List<SensorBinding> bindings)
    {
        HardwareNode? ram = roots.FirstOrDefault(r => r.Type == HardwareType.Memory && r.Identifier == "/ram");
        if (ram is null)
        {
            return;
        }

        string deviceId = DeviceId("/ram");
        var used = new HashSet<string>();
        var localSensors = new List<WireSensor>();
        var localBindings = new List<SensorBinding>();

        void AddSensor(SensorNode s, SensorMatch m)
        {
            string wireName = DisambiguateName(used, m.Kind!, m.Name!, s.Identifier);
            localSensors.Add(new WireSensor(deviceId, m.Kind!, wireName, m.Unit!, m.LabelKey!, m.LabelArg, m.Kind!));
            localBindings.Add(new SensorBinding(s.Identifier, m.Scale));
        }

        foreach (SensorNode s in ram.Sensors)
        {
            SensorMatch m = Resolve(MatchMemorySensor, s);
            if (m.Outcome == MatchOutcome.Include)
            {
                AddSensor(s, m);
            }
        }

        if (pawnIoAvailable)
        {
            IEnumerable<HardwareNode> dimms = roots.Where(r => r.Type == HardwareType.Memory && r.Identifier.StartsWith("/memory/dimm/", StringComparison.Ordinal));
            foreach (HardwareNode dimm in dimms)
            {
                uint i = ParseTrailingIndex(dimm.Identifier);
                foreach (SensorNode s in dimm.Sensors)
                {
                    // Matched by (hardware type, SensorType, LHM name), never by index: the
                    // primary reading is named "DIMM #<n>" (s1-lhm.md §9.1). The other LHM
                    // temperature sensors on a DIMM are constant limits named "Resolution",
                    // "Low"/"High", "Critical Low"/"Critical High limit" (plus SPD timings and
                    // the capacity constant, dropped along with them) — their index happens to
                    // be 1-5 on this machine, but that is incidental, not the matching rule.
                    if (s.Type == SensorType.Temperature && DimmTemperaturePattern().IsMatch(s.Name))
                    {
                        AddSensor(s, Include("temperature", $"dimm-{i}", "memory.temperature.dimm", i.ToString(CultureInfo.InvariantCulture)));
                    }
                }
            }
        }

        if (localSensors.Count == 0)
        {
            return;
        }

        devices.Add(new WireDevice(deviceId, "memory", ram.Name, Vendor: null, EmptyProperties, new MemoryHint()));
        sensors.AddRange(localSensors);
        bindings.AddRange(localBindings);
    }

    private static SensorMatch MatchMemorySensor(SensorNode s) => s.Type switch
    {
        SensorType.Load when s.Name == "Memory" => Include("load", "used", "memory.load"),
        SensorType.Data when s.Name == "Memory Used" => Include("data", "used", "memory.used", scale: TwoPow30),
        SensorType.Data when s.Name == "Memory Available" => Discard(),
        _ => NoMatch(),
    };

    private static void BuildSuperIoDevices(HardwareNode motherboard, List<WireDevice> devices, List<WireSensor> sensors, List<SensorBinding> bindings)
    {
        foreach (HardwareNode child in motherboard.Children)
        {
            if (child.Type is not (HardwareType.SuperIO or HardwareType.EmbeddedController))
            {
                continue;
            }

            ProcessDevice(child, DeviceId(child.Identifier), "motherboard", child.Name, hint: null, GenericFallback, devices, sensors, bindings);
        }
    }

    private static void BuildStorageDevices(IReadOnlyList<HardwareNode> roots, List<WireDevice> devices, List<WireSensor> sensors, List<SensorBinding> bindings)
    {
        List<HardwareNode> storageNodes = roots.Where(r => r.Type == HardwareType.Storage).ToList();

        // A serial key is only usable for identity when it is unique among the disks
        // this call enumerates: two disks that share (model, serial) must never merge.
        var keyCounts = new Dictionary<string, int>();
        foreach (HardwareNode node in storageNodes)
        {
            string? serial = node.Storage?.DriveSerial?.Trim();
            if (string.IsNullOrEmpty(serial))
            {
                continue;
            }

            string key = StorageIdentityKey(node);
            keyCounts[key] = keyCounts.GetValueOrDefault(key) + 1;
        }

        foreach (HardwareNode node in storageNodes)
        {
            string? serial = node.Storage?.DriveSerial?.Trim();
            string deviceId = !string.IsNullOrEmpty(serial) && keyCounts[StorageIdentityKey(node)] == 1
                ? "lhm-" + Sha256HexOfModelSerial(StorageModel(node), serial)
                : "lhm-" + Sha256HexOfIdentifier(node.Identifier);

            StorageHint? hint = StorageDriveNumber(node) is uint driveNumber
                ? new StorageHint(driveNumber, node.Storage?.DescriptorModel, node.Storage?.DescriptorSerial)
                : null;

            BuildStorageDevice(node, deviceId, hint, devices, sensors, bindings);
        }
    }

    /// <summary>
    /// The <c>PhysicalDriveN</c> index for the storage hint, or <see langword="null"/> when it is
    /// unknown: DiskInfoToolkit reports -1 when <c>IOCTL_STORAGE_GET_DEVICE_NUMBER</c> fails
    /// (LHM identifier <c>/hdd/-1</c>), and that must never become a u32 hint.
    /// </summary>
    private static uint? StorageDriveNumber(HardwareNode node)
    {
        if (node.Storage is { } storage)
        {
            return storage.DriveNumber >= 0 ? (uint)storage.DriveNumber : null;
        }

        return uint.TryParse(node.Identifier.Split('/')[^1], NumberStyles.None, CultureInfo.InvariantCulture, out uint parsed) ? parsed : null;
    }

    private static string StorageModel(HardwareNode node) => node.Storage?.DescriptorModel ?? node.Name;

    private static string StorageIdentityKey(HardwareNode node) => StorageModel(node) + "\0" + node.Storage?.DriveSerial?.Trim();

    private static void BuildStorageDevice(HardwareNode node, string deviceId, StorageHint? hint, List<WireDevice> devices, List<WireSensor> sensors, List<SensorBinding> bindings)
    {
        var used = new HashSet<string>();
        var local = new List<WireSensor>();
        var localBindings = new List<SensorBinding>();
        var properties = new Dictionary<string, string>();

        foreach (SensorNode s in node.Sensors)
        {
            if (s.Type == SensorType.Level && s.Name == "Available Spare Threshold")
            {
                if (s.Value is { } v)
                {
                    properties["availableSpareThresholdPct"] = v.ToString("0.###", CultureInfo.InvariantCulture);
                }

                continue;
            }

            SensorMatch m = Resolve(MatchStorageSensor, s);
            if (m.Outcome != MatchOutcome.Include)
            {
                continue;
            }

            string wireName = DisambiguateName(used, m.Kind!, m.Name!, s.Identifier);
            local.Add(new WireSensor(deviceId, m.Kind!, wireName, m.Unit!, m.LabelKey!, m.LabelArg, m.Kind!));
            localBindings.Add(new SensorBinding(s.Identifier, m.Scale));
        }

        if (local.Count == 0 && properties.Count == 0)
        {
            return;
        }

        devices.Add(new WireDevice(deviceId, "storage", node.Name, Vendor: null, properties, hint));
        sensors.AddRange(local);
        bindings.AddRange(localBindings);
    }

    private static SensorMatch MatchStorageSensor(SensorNode s)
    {
        switch (s.Type)
        {
            case SensorType.Temperature:
                if (s.Name is "Temperature" or "Composite Temperature")
                {
                    return Include("temperature", "drive", "storage.temperature");
                }

                Match sensorTemp = StorageTemperatureSensorPattern().Match(s.Name);
                if (sensorTemp.Success)
                {
                    return Include("temperature", $"sensor-{sensorTemp.Groups[1].Value}", "storage.temperatureSensor", sensorTemp.Groups[1].Value);
                }

                if (s.Name.StartsWith("Warning", StringComparison.Ordinal) || s.Name.StartsWith("Critical", StringComparison.Ordinal))
                {
                    return Discard();
                }

                return NoMatch();

            case SensorType.Load:
                if (s.Name == "Total Activity")
                {
                    return Include("load", "active", "storage.active");
                }

                if (s.Name is "Used Space" or "Read Activity" or "Write Activity")
                {
                    return Discard();
                }

                return NoMatch();

            case SensorType.Throughput:
                if (s.Name == "Read Rate")
                {
                    return Include("throughput", "read", "storage.read");
                }

                if (s.Name == "Write Rate")
                {
                    return Include("throughput", "write", "storage.write");
                }

                return NoMatch();

            case SensorType.Level:
                if (s.Name == "Life")
                {
                    return Include("percent", "life", "storage.life");
                }

                if (s.Name == "Available Spare")
                {
                    return Include("percent", "available-spare", "storage.availableSpare");
                }

                if (s.Name == "Percentage Used")
                {
                    return Include("percent", "wear", "storage.percentUsed");
                }

                return NoMatch();

            case SensorType.Data:
                if (s.Name is "Free Space" or "Total Space")
                {
                    return Discard();
                }

                if (s.Name == "Data Read")
                {
                    return Include("data", "host-read", "storage.hostRead", scale: TwoPow30);
                }

                if (s.Name == "Data Written")
                {
                    return Include("data", "host-written", "storage.hostWritten", scale: TwoPow30);
                }

                return NoMatch();

            case SensorType.Factor:
                if (s.Name == "Power On Hours")
                {
                    return Include("counter", "power-on-hours", "storage.powerOnHours", unit: "hours");
                }

                if (s.Name == "Power On Count")
                {
                    return Include("counter", "power-cycles", "storage.powerCycles", unit: "count");
                }

                return NoMatch();

            default:
                return NoMatch();
        }
    }

    /// <summary>
    /// Runs a category-specific matcher first; a sensor it does not recognise
    /// (<see cref="MatchOutcome.NoMatch"/>) falls back to the stable
    /// <c>lhm-&lt;sensortype&gt;-&lt;index&gt;</c> name with the <c>lhm.raw</c> label, or is
    /// dropped if its SensorType is not one this service ever publishes (Control, Factor
    /// outside the table, Energy, Frequency, Noise, Flow, Conductivity, Humidity, TimeSpan,
    /// Timing, …).
    /// </summary>
    private static SensorMatch Resolve(Func<SensorNode, SensorMatch> specific, SensorNode s)
    {
        SensorMatch m = specific(s);
        return m.Outcome == MatchOutcome.NoMatch ? GenericFallback(s) : m;
    }

    private static SensorMatch GenericFallback(SensorNode s)
    {
        if (!FallbackByType.TryGetValue(s.Type, out (string Kind, string Unit, double Scale) info))
        {
            return Discard();
        }

        string name = $"lhm-{s.Type.ToString().ToLowerInvariant()}-{s.Index.ToString(CultureInfo.InvariantCulture)}";
        return Include(info.Kind, name, "lhm.raw", s.Name, info.Unit, info.Scale);
    }

    private static void ProcessDevice(
        HardwareNode node,
        string deviceId,
        string kind,
        string name,
        IdentityHint? hint,
        Func<SensorNode, SensorMatch> resolve,
        List<WireDevice> devices,
        List<WireSensor> sensors,
        List<SensorBinding> bindings)
    {
        var used = new HashSet<string>();
        var local = new List<WireSensor>();
        var localBindings = new List<SensorBinding>();

        foreach (SensorNode s in node.Sensors)
        {
            SensorMatch m = resolve(s);
            if (m.Outcome != MatchOutcome.Include)
            {
                continue;
            }

            string wireName = DisambiguateName(used, m.Kind!, m.Name!, s.Identifier);
            local.Add(new WireSensor(deviceId, m.Kind!, wireName, m.Unit!, m.LabelKey!, m.LabelArg, m.Kind!));
            localBindings.Add(new SensorBinding(s.Identifier, m.Scale));
        }

        if (local.Count == 0)
        {
            return;
        }

        devices.Add(new WireDevice(deviceId, kind, name, Vendor: null, EmptyProperties, hint));
        sensors.AddRange(local);
        bindings.AddRange(localBindings);
    }

    /// <summary>
    /// Two LHM sensors on the same device can map to the same <c>kind/name</c> (e.g. two
    /// different fallback tables converging); when that happens the second one gets a
    /// suffix from the last segment of its own LHM identifier, so it never overwrites the
    /// first and the caller's bindings list stays aligned with the sensor list.
    /// </summary>
    private static string DisambiguateName(HashSet<string> used, string kind, string name, string identifier)
    {
        string key = $"{kind}/{name}";
        if (used.Add(key))
        {
            return name;
        }

        string suffix = identifier.Split('/')[^1];
        string alternate = $"{name}-{suffix}";
        used.Add($"{kind}/{alternate}");
        return alternate;
    }

    private static uint ParseTrailingIndex(string identifier) => uint.Parse(identifier.Split('/')[^1], CultureInfo.InvariantCulture);

    private static string DeviceId(string identifier) => "lhm-" + Sha256HexOfIdentifier(identifier);

    private static string Sha256HexOfIdentifier(string identifier) => Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(identifier)));

    private static string Sha256HexOfModelSerial(string model, string serial)
    {
        byte[] modelBytes = Encoding.UTF8.GetBytes(model);
        byte[] serialBytes = Encoding.UTF8.GetBytes(serial);
        byte[] combined = new byte[modelBytes.Length + 1 + serialBytes.Length];
        modelBytes.CopyTo(combined, 0);
        combined[modelBytes.Length] = 0;
        serialBytes.CopyTo(combined, modelBytes.Length + 1);
        return Convert.ToHexStringLower(SHA256.HashData(combined));
    }

    private static SensorMatch Include(string kind, string name, string labelKey, string? labelArg = null, string? unit = null, double scale = 1.0) =>
        new(MatchOutcome.Include, kind, name, labelKey, labelArg, unit ?? UnitByKind[kind], scale);

    private static SensorMatch Discard() => new(MatchOutcome.Discard);

    private static SensorMatch NoMatch() => new(MatchOutcome.NoMatch);

    private enum MatchOutcome
    {
        NoMatch,
        Discard,
        Include,
    }

    private readonly record struct SensorMatch(
        MatchOutcome Outcome,
        string? Kind = null,
        string? Name = null,
        string? LabelKey = null,
        string? LabelArg = null,
        string? Unit = null,
        double Scale = 1.0);
}
