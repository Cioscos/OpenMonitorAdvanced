using System.Security.Cryptography;
using System.Text;
using LibreHardwareMonitor.Hardware;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// Builds fake <see cref="HardwareNode"/> trees using the real LHM names and identifiers
/// recorded from this machine's elevated run (<c>docs/superpowers/references/m4/s1-lhm.md</c>
/// §9.1), and asserts the canonical mapping table in §9.7 of that same document.
/// </summary>
public sealed class SchemaBuilderTests
{
    private static SensorNode Sensor(string identifier, SensorType type, string name, int index, float? value = null) =>
        new(identifier, type, name, index, value);

    private static string Sha256HexOf(string text) => Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(text)));

    private static WireSensor Find(BuiltSchema schema, string deviceId, string kind, string name) =>
        Assert.Single(schema.Schema.Sensors, s => s.DeviceId == deviceId && s.Kind == kind && s.Name == name);

    /// <summary>
    /// <see cref="SchemaBuilder.Build"/> plus the wire invariants checked on every test tree, so
    /// no test can produce a schema the app would refuse.
    /// </summary>
    private static BuiltSchema Build(IReadOnlyList<HardwareNode> roots, bool pawnIoAvailable, IReadOnlyDictionary<string, string>? storageDeviceIds = null)
    {
        BuiltSchema built = SchemaBuilder.Build(roots, pawnIoAvailable, storageDeviceIds);
        AssertWireInvariants(built);
        return built;
    }

    /// <summary>
    /// The invariants the app's <c>validate_schema</c> (crates/oma-win/src/svc/link.rs) enforces,
    /// which reject the whole schema on the first violation: non-empty device ids without
    /// <c>/</c>, unique; every sensor on a published device, with a non-empty kind and name
    /// without <c>/</c>, and a unique <c>device_id/kind/name</c>. Also: every label key is one
    /// the UI translates (<see cref="CanonicalNames.LabelKeys"/>), and the bindings stay aligned.
    /// </summary>
    internal static void AssertWireInvariants(BuiltSchema built)
    {
        var devices = new HashSet<string>(StringComparer.Ordinal);
        foreach (WireDevice device in built.Schema.Devices)
        {
            Assert.False(string.IsNullOrEmpty(device.Id), "empty device id");
            Assert.DoesNotContain('/', device.Id);
            Assert.True(devices.Add(device.Id), $"duplicate device id {device.Id}");
        }

        var sensors = new HashSet<string>(StringComparer.Ordinal);
        foreach (WireSensor sensor in built.Schema.Sensors)
        {
            Assert.True(devices.Contains(sensor.DeviceId), $"sensor {sensor.Name} refers to unknown device {sensor.DeviceId}");
            Assert.False(string.IsNullOrEmpty(sensor.Kind), "empty sensor kind");
            Assert.DoesNotContain('/', sensor.Kind);
            Assert.False(string.IsNullOrEmpty(sensor.Name), "empty sensor name");
            Assert.DoesNotContain('/', sensor.Name);
            Assert.True(sensors.Add($"{sensor.DeviceId}/{sensor.Kind}/{sensor.Name}"), $"duplicate sensor id {sensor.DeviceId}/{sensor.Kind}/{sensor.Name}");
            Assert.True(CanonicalNames.LabelKeys.Contains(sensor.LabelKey), $"label key {sensor.LabelKey} is not in CanonicalNames.LabelKeys");
        }

        Assert.Equal(built.Schema.Sensors.Count, built.Bindings.Count);
        Assert.Equal(built.StorageDeviceIds.Count, built.StorageDeviceIds.Values.Distinct(StringComparer.Ordinal).Count());
    }

    [Fact]
    public void TwoStorageRootsWithTheSameIdentifierAreSkipped()
    {
        // DiskInfoToolkit reports "/hdd/-1" for every disk whose IOCTL_STORAGE_GET_DEVICE_NUMBER
        // fails: two such disks without a serial would share the identifier, the device id and
        // the sensor identifiers, so neither can be read unambiguously.
        HardwareNode Unnumbered(string name) => new(
            "/hdd/-1",
            HardwareType.Storage,
            name,
            [Sensor("/hdd/-1/temperature/0", SensorType.Temperature, "Temperature", 0)],
            [],
            new StorageInfo(-1, null, null, null, Rotational: true));
        var ram = new HardwareNode("/ram", HardwareType.Memory, "Total Memory", [Sensor("/ram/load/0", SensorType.Load, "Memory", 0)], []);

        BuiltSchema schema = Build([ram, Unnumbered("Disk A"), Unnumbered("Disk B")], pawnIoAvailable: true);

        Assert.DoesNotContain(schema.Schema.Devices, d => d.Kind == "storage");
        Assert.Single(schema.Schema.Devices, d => d.Kind == "memory");
        Assert.False(schema.StorageDeviceIds.ContainsKey("/hdd/-1"));
        Assert.Equal(["/hdd/-1"], schema.SkippedRoots);
    }

    [Fact]
    public void ADeviceIdAlreadyPublishedIsSkipped()
    {
        // A pin (the id published for /hdd/0 while it had a twin) equals the id /hdd/1 would get
        // now: the second device is skipped instead of publishing a duplicate id.
        HardwareNode Disk(int n) => new(
            $"/hdd/{n}",
            HardwareType.Storage,
            "Drive",
            [Sensor($"/hdd/{n}/temperature/0", SensorType.Temperature, "Temperature", 0)],
            [],
            new StorageInfo(n, "Drive", null, null, Rotational: true));
        var pins = new Dictionary<string, string> { ["/hdd/0"] = "lhm-" + Sha256HexOf("/hdd/1") };

        // The pinned device wins in either root order.
        foreach (HardwareNode[] roots in new[] { new[] { Disk(0), Disk(1) }, new[] { Disk(1), Disk(0) } })
        {
            BuiltSchema schema = Build(roots, pawnIoAvailable: true, pins);

            WireDevice device = Assert.Single(schema.Schema.Devices);
            Assert.Equal("lhm-" + Sha256HexOf("/hdd/1"), device.Id);
            Assert.Equal("/hdd/0/temperature/0", Assert.Single(schema.Bindings).LhmIdentifier);
            Assert.False(schema.StorageDeviceIds.ContainsKey("/hdd/1"));
            Assert.Equal(["/hdd/1"], schema.SkippedRoots);
        }
    }

    [Fact]
    public void TwoRootsWithTheSameIdentifierPublishOnlyTheFirst()
    {
        HardwareNode Cooler(string name) => new(
            "/heatmaster/0",
            HardwareType.Cooler,
            name,
            [Sensor("/heatmaster/0/fan/0", SensorType.Fan, "Fan #1", 0)],
            []);

        BuiltSchema schema = Build([Cooler("First"), Cooler("Second")], pawnIoAvailable: true);

        Assert.Equal("First", Assert.Single(schema.Schema.Devices).Name);
        Assert.Single(schema.Schema.Sensors);
        Assert.Equal(["/heatmaster/0"], schema.SkippedRoots);
    }

    [Fact]
    public void SecondOrderNameCollisionsGetUniqueNames()
    {
        // "drive" is taken by the first sensor, "drive-9" (the first fallback) by the second,
        // so the third must not reuse "drive-9".
        var storage = new HardwareNode(
            "/hdd/0",
            HardwareType.Storage,
            "Drive",
            [
                Sensor("/hdd/0/temperature/0", SensorType.Temperature, "Temperature", 0),
                Sensor("/hdd/0/temperature/9", SensorType.Temperature, "Composite Temperature", 9),
                Sensor("/hdd/0/sub/temperature/9", SensorType.Temperature, "Temperature", 9),
                Sensor("/hdd/0/other/temperature/9", SensorType.Temperature, "Temperature", 9),
            ],
            [],
            new StorageInfo(0, "Drive", null, null, true));

        BuiltSchema schema = Build([storage], pawnIoAvailable: true);

        Assert.Equal(4, schema.Schema.Sensors.Count);
        Assert.Equal(["drive", "drive-9", "drive-9-2", "drive-9-3"], schema.Schema.Sensors.Select(s => s.Name));
        Assert.Equal(
            ["/hdd/0/temperature/0", "/hdd/0/temperature/9", "/hdd/0/sub/temperature/9", "/hdd/0/other/temperature/9"],
            schema.Bindings.Select(b => b.LhmIdentifier));
    }

    [Fact]
    public void EveryCanonicalLabelKeyIsEmittedByAFullTree()
    {
        var cpu = new HardwareNode(
            "/amdcpu/0",
            HardwareType.Cpu,
            "CPU",
            [
                Sensor("/amdcpu/0/load/0", SensorType.Load, "CPU Total", 0),
                Sensor("/amdcpu/0/load/1", SensorType.Load, "CPU Core Max", 1),
                Sensor("/amdcpu/0/temperature/2", SensorType.Temperature, "Core (Tctl/Tdie)", 2),
                Sensor("/amdcpu/0/temperature/3", SensorType.Temperature, "CCD1 (Tdie)", 3),
                Sensor("/amdcpu/0/temperature/4", SensorType.Temperature, "CPU Package", 4),
                Sensor("/amdcpu/0/temperature/5", SensorType.Temperature, "Core #1", 5),
                Sensor("/amdcpu/0/power/0", SensorType.Power, "Package", 0),
                Sensor("/amdcpu/0/power/1", SensorType.Power, "Core #1 (SMU)", 1),
                Sensor("/amdcpu/0/power/2", SensorType.Power, "SoC", 2),
                Sensor("/amdcpu/0/voltage/0", SensorType.Voltage, "SoC", 0),
                Sensor("/amdcpu/0/voltage/2", SensorType.Voltage, "Core #1 VID", 2),
                Sensor("/amdcpu/0/voltage/3", SensorType.Voltage, "Unknown rail", 3),
                Sensor("/amdcpu/0/clock/0", SensorType.Clock, "Bus Speed", 0),
                Sensor("/amdcpu/0/clock/1", SensorType.Clock, "Cores (Average)", 1),
                Sensor("/amdcpu/0/clock/2", SensorType.Clock, "Cores (Average Effective)", 2),
                Sensor("/amdcpu/0/clock/3", SensorType.Clock, "Core #1", 3),
                Sensor("/amdcpu/0/clock/4", SensorType.Clock, "Core #1 (Effective)", 4),
            ],
            []);
        var ram = new HardwareNode(
            "/ram",
            HardwareType.Memory,
            "Total Memory",
            [Sensor("/ram/load/0", SensorType.Load, "Memory", 0), Sensor("/ram/data/0", SensorType.Data, "Memory Used", 0)],
            []);
        var dimm = new HardwareNode("/memory/dimm/1", HardwareType.Memory, "DIMM", [Sensor("/memory/dimm/1/temperature/0", SensorType.Temperature, "DIMM #1", 0)], []);
        var disk = new HardwareNode(
            "/nvme/2",
            HardwareType.Storage,
            "Drive",
            [
                Sensor("/nvme/2/temperature/0", SensorType.Temperature, "Composite Temperature", 0),
                Sensor("/nvme/2/temperature/1", SensorType.Temperature, "Temperature #1", 1),
                Sensor("/nvme/2/load/51", SensorType.Load, "Total Activity", 51),
                Sensor("/nvme/2/throughput/54", SensorType.Throughput, "Read Rate", 54),
                Sensor("/nvme/2/throughput/55", SensorType.Throughput, "Write Rate", 55),
                Sensor("/nvme/2/level/20", SensorType.Level, "Life", 20),
                Sensor("/nvme/2/level/100", SensorType.Level, "Available Spare", 100),
                Sensor("/nvme/2/level/102", SensorType.Level, "Percentage Used", 102),
                Sensor("/nvme/2/data/21", SensorType.Data, "Data Read", 21),
                Sensor("/nvme/2/data/22", SensorType.Data, "Data Written", 22),
                Sensor("/nvme/2/factor/23", SensorType.Factor, "Power On Hours", 23),
                Sensor("/nvme/2/factor/24", SensorType.Factor, "Power On Count", 24),
            ],
            [],
            new StorageInfo(2, "Drive", null, "S1", Rotational: false));

        BuiltSchema schema = Build([cpu, ram, dimm, disk], pawnIoAvailable: true);

        Assert.Equal(
            CanonicalNames.LabelKeys.Order(StringComparer.Ordinal),
            schema.Schema.Sensors.Select(s => s.LabelKey).Distinct().Order(StringComparer.Ordinal));
    }

    [Fact]
    public void CpuSensorsGetCanonicalNamesAndTheCpuHint()
    {
        var cpu = new HardwareNode(
            "/amdcpu/0",
            HardwareType.Cpu,
            "AMD Ryzen 7 7800X3D",
            [
                Sensor("/amdcpu/0/load/0", SensorType.Load, "CPU Total", 0),
                Sensor("/amdcpu/0/temperature/2", SensorType.Temperature, "Core (Tctl/Tdie)", 2),
                Sensor("/amdcpu/0/temperature/3", SensorType.Temperature, "CCD1 (Tdie)", 3),
                Sensor("/amdcpu/0/power/0", SensorType.Power, "Package", 0),
                Sensor("/amdcpu/0/power/1", SensorType.Power, "Core #1 (SMU)", 1),
                Sensor("/amdcpu/0/voltage/2", SensorType.Voltage, "Core #1 VID", 2),
                Sensor("/amdcpu/0/clock/0", SensorType.Clock, "Bus Speed", 0),
                Sensor("/amdcpu/0/clock/2", SensorType.Clock, "Cores (Average Effective)", 2),
                Sensor("/amdcpu/0/clock/4", SensorType.Clock, "Core #1 (Effective)", 4),
                Sensor("/amdcpu/0/factor/0", SensorType.Factor, "Core #1", 0),
            ],
            []);

        BuiltSchema schema = Build([cpu], pawnIoAvailable: true);

        WireDevice device = Assert.Single(schema.Schema.Devices);
        Assert.Equal("cpu", device.Kind);
        Assert.Equal(new CpuHint(0), device.Hint);
        string id = device.Id;

        Assert.Equal(9, schema.Schema.Sensors.Count); // the Factor sensor is dropped

        WireSensor total = Find(schema, id, "load", "total");
        Assert.Equal("cpu.load.total", total.LabelKey);
        Assert.Null(total.LabelArg);

        WireSensor tctl = Find(schema, id, "temperature", "tctl");
        Assert.Equal("cpu.temperature.tctl", tctl.LabelKey);

        WireSensor ccd = Find(schema, id, "temperature", "ccd-1");
        Assert.Equal("cpu.temperature.ccd", ccd.LabelKey);
        Assert.Equal("1", ccd.LabelArg);

        WireSensor package = Find(schema, id, "power", "package");
        Assert.Equal("cpu.power.package", package.LabelKey);

        WireSensor corePower = Find(schema, id, "power", "core-1");
        Assert.Equal("cpu.power.core", corePower.LabelKey);
        Assert.Equal("1", corePower.LabelArg);

        WireSensor vid = Find(schema, id, "voltage", "core-1-vid");
        Assert.Equal("cpu.voltage.coreVid", vid.LabelKey);
        Assert.Equal("1", vid.LabelArg);

        WireSensor bus = Find(schema, id, "clock", "bus");
        Assert.Equal("cpu.clock.bus", bus.LabelKey);

        WireSensor avgEffective = Find(schema, id, "clock", "average-effective");
        Assert.Equal("cpu.clock.averageEffective", avgEffective.LabelKey);

        WireSensor coreEffective = Find(schema, id, "clock", "core-1-effective");
        Assert.Equal("cpu.clock.coreEffective", coreEffective.LabelKey);
        Assert.Equal("1", coreEffective.LabelArg);

        Assert.DoesNotContain(schema.Schema.Sensors, s => s.LabelKey == "lhm.raw");
    }

    [Fact]
    public void PerCoreLoadsAreDropped()
    {
        var cpu = new HardwareNode(
            "/amdcpu/0",
            HardwareType.Cpu,
            "AMD Ryzen 7 7800X3D",
            [
                Sensor("/amdcpu/0/load/0", SensorType.Load, "CPU Total", 0),
                Sensor("/amdcpu/0/load/2", SensorType.Load, "CPU Core #1", 2),
                Sensor("/amdcpu/0/load/3", SensorType.Load, "CPU Core #1 Thread #1", 3),
                Sensor("/amdcpu/0/load/1", SensorType.Load, "CPU Core Max", 1),
            ],
            []);

        BuiltSchema schema = Build([cpu], pawnIoAvailable: true);

        Assert.Equal(2, schema.Schema.Sensors.Count);
        Assert.Contains(schema.Schema.Sensors, s => s.Kind == "load" && s.Name == "total");
        Assert.Contains(schema.Schema.Sensors, s => s.Kind == "load" && s.Name == "core-max");
    }

    [Fact]
    public void MemoryAndDimmsBecomeOneDevice()
    {
        var ram = new HardwareNode(
            "/ram",
            HardwareType.Memory,
            "Total Memory",
            [
                Sensor("/ram/load/0", SensorType.Load, "Memory", 0),
                Sensor("/ram/data/0", SensorType.Data, "Memory Used", 0),
                Sensor("/ram/data/1", SensorType.Data, "Memory Available", 1),
            ],
            []);
        var vram = new HardwareNode(
            "/vram",
            HardwareType.Memory,
            "Virtual Memory",
            [
                Sensor("/vram/data/2", SensorType.Data, "Memory Used", 2),
                Sensor("/vram/load/1", SensorType.Load, "Memory", 3),
            ],
            []);
        var dimm1 = new HardwareNode(
            "/memory/dimm/1",
            HardwareType.Memory,
            "Corsair - CMH32GX5M2B6400C36 (#1)",
            [
                Sensor("/memory/dimm/1/temperature/0", SensorType.Temperature, "DIMM #1", 0),
                Sensor("/memory/dimm/1/temperature/2", SensorType.Temperature, "High", 2),
            ],
            []);
        var dimm3 = new HardwareNode(
            "/memory/dimm/3",
            HardwareType.Memory,
            "Corsair - CMH32GX5M2B6400C36 (#3)",
            [Sensor("/memory/dimm/3/temperature/0", SensorType.Temperature, "DIMM #3", 0)],
            []);

        BuiltSchema schema = Build([ram, vram, dimm1, dimm3], pawnIoAvailable: true);

        WireDevice device = Assert.Single(schema.Schema.Devices);
        Assert.Equal("memory", device.Kind);
        Assert.Equal(new MemoryHint(), device.Hint);
        Assert.Equal("lhm-" + Sha256HexOf("/ram"), device.Id);

        Assert.Equal(4, schema.Schema.Sensors.Count);
        Find(schema, device.Id, "load", "used");
        SensorBinding usedBinding = schema.Bindings[schema.Schema.Sensors.ToList().FindIndex(s => s.Kind == "data" && s.Name == "used")];
        Assert.Equal(1024d * 1024d * 1024d, usedBinding.Scale);
        Find(schema, device.Id, "temperature", "dimm-1");
        Find(schema, device.Id, "temperature", "dimm-3");
        Assert.DoesNotContain(schema.Schema.Sensors, s => s.DeviceId != device.Id);
    }

    [Fact]
    public void DimmTemperatureIsMatchedByNameNotByIndex()
    {
        // On this DIMM the constant limit ("High") happens to sit at LHM index 0 and the real
        // reading ("DIMM #2") at index 3 — the reverse of the usual layout — to prove the match
        // is on (hardware type, SensorType, LHM name), never on the sensor's index.
        var ram = new HardwareNode("/ram", HardwareType.Memory, "Total Memory", [Sensor("/ram/load/0", SensorType.Load, "Memory", 0)], []);
        var dimm2 = new HardwareNode(
            "/memory/dimm/2",
            HardwareType.Memory,
            "Corsair - CMH32GX5M2B6400C36 (#2)",
            [
                Sensor("/memory/dimm/2/temperature/0", SensorType.Temperature, "High", 0),
                Sensor("/memory/dimm/2/temperature/3", SensorType.Temperature, "DIMM #2", 3),
            ],
            []);

        BuiltSchema schema = Build([ram, dimm2], pawnIoAvailable: true);

        WireDevice device = Assert.Single(schema.Schema.Devices);
        Find(schema, device.Id, "temperature", "dimm-2");
        Assert.DoesNotContain(schema.Schema.Sensors, s => s.Kind == "temperature" && s.Name != "dimm-2");
        Assert.Single(schema.Schema.Sensors, s => s.Kind == "temperature");

        // Not just the wire name: the binding must point at the "DIMM #2" LHM sensor (index 3),
        // never at the "High" limit constant that happens to sit at index 0.
        int sensorIndex = schema.Schema.Sensors.ToList().FindIndex(s => s.Kind == "temperature");
        Assert.Equal("/memory/dimm/2/temperature/3", schema.Bindings[sensorIndex].LhmIdentifier);
    }

    [Fact]
    public void StorageMapsDuplicatesHealthAndCounters()
    {
        var storage = new HardwareNode(
            "/nvme/2",
            HardwareType.Storage,
            "Fanxiang S880 2TB",
            [
                Sensor("/nvme/2/temperature/0", SensorType.Temperature, "Composite Temperature", 0),
                Sensor("/nvme/2/temperature/1", SensorType.Temperature, "Temperature #1", 1),
                Sensor("/nvme/2/temperature/10", SensorType.Temperature, "Warning Temperature", 10),
                Sensor("/nvme/2/temperature/11", SensorType.Temperature, "Critical Temperature", 11),
                Sensor("/nvme/2/load/51", SensorType.Load, "Total Activity", 51),
                Sensor("/nvme/2/load/30", SensorType.Load, "Used Space", 30),
                Sensor("/nvme/2/throughput/54", SensorType.Throughput, "Read Rate", 54),
                Sensor("/nvme/2/throughput/55", SensorType.Throughput, "Write Rate", 55),
                Sensor("/nvme/2/level/20", SensorType.Level, "Life", 20),
                Sensor("/nvme/2/level/100", SensorType.Level, "Available Spare", 100),
                Sensor("/nvme/2/level/101", SensorType.Level, "Available Spare Threshold", 101, 10f),
                Sensor("/nvme/2/level/102", SensorType.Level, "Percentage Used", 102),
                Sensor("/nvme/2/data/21", SensorType.Data, "Data Read", 21),
                Sensor("/nvme/2/data/22", SensorType.Data, "Data Written", 22),
                Sensor("/nvme/2/data/31", SensorType.Data, "Total Space", 31),
                Sensor("/nvme/2/factor/23", SensorType.Factor, "Power On Hours", 23),
                Sensor("/nvme/2/factor/24", SensorType.Factor, "Power On Count", 24),
            ],
            [],
            new StorageInfo(2, "Fanxiang S880 2TB", "eui.abc.", "IDENTIFY-1", Rotational: false));

        BuiltSchema schema = Build([storage], pawnIoAvailable: true);

        WireDevice device = Assert.Single(schema.Schema.Devices);
        Assert.Equal("storage", device.Kind);
        var hint = Assert.IsType<StorageHint>(device.Hint);
        Assert.Equal(2u, hint.PhysicalDrive);
        Assert.Equal("Fanxiang S880 2TB", hint.Model);
        Assert.Equal("eui.abc.", hint.Serial);

        Assert.Equal("10", device.Properties["availableSpareThresholdPct"]);

        Find(schema, device.Id, "temperature", "drive");
        Find(schema, device.Id, "temperature", "sensor-1");
        Find(schema, device.Id, "load", "active");
        Find(schema, device.Id, "throughput", "read");
        Find(schema, device.Id, "throughput", "write");
        Find(schema, device.Id, "percent", "life");
        Find(schema, device.Id, "percent", "available-spare");
        Find(schema, device.Id, "percent", "wear");
        WireSensor hostRead = Find(schema, device.Id, "data", "host-read");
        Assert.Equal("bytes", hostRead.Unit);
        WireSensor hours = Find(schema, device.Id, "counter", "power-on-hours");
        Assert.Equal("hours", hours.Unit);
        WireSensor cycles = Find(schema, device.Id, "counter", "power-cycles");
        Assert.Equal("count", cycles.Unit);

        Assert.DoesNotContain(schema.Schema.Sensors, s => s.Name == "sensor-10" || s.Name == "sensor-11");
        Assert.DoesNotContain(schema.Schema.Sensors, s => s.Kind == "load" && s.Name.Contains("used", StringComparison.OrdinalIgnoreCase));
        Assert.DoesNotContain(schema.Schema.Sensors, s => s.LabelKey == "lhm.raw");
        Assert.Equal(12, schema.Schema.Sensors.Count);
    }

    [Fact]
    public void StorageDeviceIdSurvivesRenumbering()
    {
        var info = new StorageInfo(2, "Model X", "serial-descr", "IDENTIFY-STABLE", Rotational: false);
        var asNvme2 = new HardwareNode("/nvme/2", HardwareType.Storage, "Model X", [], [], info);
        var asNvme5 = new HardwareNode("/nvme/5", HardwareType.Storage, "Model X", [], [], info with { DriveNumber = 5 });

        // Wrap each alone with one throwaway sensor so the device is emitted.
        SensorNode dummy(string id) => Sensor(id, SensorType.Temperature, "Temperature", 0);
        var node1 = asNvme2 with { Sensors = [dummy("/nvme/2/temperature/0")] };
        var node5 = asNvme5 with { Sensors = [dummy("/nvme/5/temperature/0")] };

        BuiltSchema s1 = Build([node1], pawnIoAvailable: true);
        BuiltSchema s2 = Build([node5], pawnIoAvailable: true);

        Assert.Equal(s1.Schema.Devices[0].Id, s2.Schema.Devices[0].Id);
    }

    [Fact]
    public void APinnedStorageDeviceIdIsKeptAndNeverShared()
    {
        HardwareNode Twin(int n) => new(
            $"/hdd/{n}",
            HardwareType.Storage,
            "Same Model",
            [Sensor($"/hdd/{n}/temperature/0", SensorType.Temperature, "Temperature", 0)],
            [],
            new StorageInfo(n, "Same Model", null, "DUP", Rotational: true));

        BuiltSchema alone = Build([Twin(0)], pawnIoAvailable: true);
        string published = Assert.Single(alone.Schema.Devices).Id;
        Assert.Equal(published, alone.StorageDeviceIds["/hdd/0"]);

        var pins = new Dictionary<string, string> { ["/hdd/0"] = published };
        BuiltSchema both = Build([Twin(0), Twin(1)], pawnIoAvailable: true, pins);

        Assert.Equal(published, both.StorageDeviceIds["/hdd/0"]);
        Assert.Equal("lhm-" + Sha256HexOf("/hdd/1"), both.StorageDeviceIds["/hdd/1"]); // the twin never merges into it
        Assert.Equal(2, both.Schema.Devices.Select(d => d.Id).Distinct().Count());
    }

    [Fact]
    public void ANegativeDriveNumberGivesNoStorageHint()
    {
        // DiskInfoToolkit reports DriveNumber -1 when IOCTL_STORAGE_GET_DEVICE_NUMBER fails
        // (LHM identifier "/hdd/-1"): it must never become a u32 PhysicalDrive hint.
        var disk = new HardwareNode(
            "/hdd/-1",
            HardwareType.Storage,
            "Drive",
            [Sensor("/hdd/-1/temperature/0", SensorType.Temperature, "Temperature", 0)],
            [],
            new StorageInfo(-1, null, null, "IDENTIFY", Rotational: true));

        BuiltSchema schema = Build([disk], pawnIoAvailable: true);

        WireDevice device = Assert.Single(schema.Schema.Devices);
        Assert.Equal("storage", device.Kind);
        Assert.Null(device.Hint);
    }

    [Fact]
    public void MissingOrDuplicateStorageSerialsDoNotMergeDevices()
    {
        SensorNode dummy(string id) => Sensor(id, SensorType.Temperature, "Temperature", 0);

        // Two identical disks, no serial at all.
        var noSerialA = new HardwareNode("/hdd/0", HardwareType.Storage, "Same Model", [dummy("/hdd/0/temperature/0")], [], new StorageInfo(0, "Same Model", null, null, true));
        var noSerialB = new HardwareNode("/hdd/1", HardwareType.Storage, "Same Model", [dummy("/hdd/1/temperature/0")], [], new StorageInfo(1, "Same Model", null, null, true));
        BuiltSchema noSerial = Build([noSerialA, noSerialB], pawnIoAvailable: true);
        Assert.Equal(2, noSerial.Schema.Devices.Select(d => d.Id).Distinct().Count());

        // Two identical disks with the *same* (bogus/duplicated) serial.
        var dupA = new HardwareNode("/hdd/0", HardwareType.Storage, "Same Model", [dummy("/hdd/0/temperature/0")], [], new StorageInfo(0, "Same Model", null, "DUP", true));
        var dupB = new HardwareNode("/hdd/1", HardwareType.Storage, "Same Model", [dummy("/hdd/1/temperature/0")], [], new StorageInfo(1, "Same Model", null, "DUP", true));
        BuiltSchema dup = Build([dupA, dupB], pawnIoAvailable: true);
        Assert.Equal(2, dup.Schema.Devices.Select(d => d.Id).Distinct().Count());
    }

    [Fact]
    public void WireNamesDoNotRepeatTheKindAndSensorIdsAreUnique()
    {
        var storage = new HardwareNode(
            "/hdd/0",
            HardwareType.Storage,
            "Drive",
            [
                Sensor("/hdd/0/temperature/0", SensorType.Temperature, "Temperature", 0),
                // Two different LHM names converging on the same canonical name (drive temperature).
                Sensor("/hdd/0/temperature/9", SensorType.Temperature, "Composite Temperature", 9),
            ],
            [],
            new StorageInfo(0, "Drive", null, null, true));

        BuiltSchema schema = Build([storage], pawnIoAvailable: true);

        Assert.Equal(2, schema.Schema.Sensors.Count);
        Assert.All(schema.Schema.Sensors, s => Assert.False(s.Name.StartsWith(s.Kind + "/", StringComparison.Ordinal)));
        Assert.Equal(schema.Schema.Sensors.Select(s => $"{s.Kind}/{s.Name}").Distinct().Count(), schema.Schema.Sensors.Count);
        Assert.Contains(schema.Schema.Sensors, s => s.Kind == "temperature" && s.Name == "drive");
        Assert.Contains(schema.Schema.Sensors, s => s.Kind == "temperature" && s.Name == "drive-9");
    }

    [Fact]
    public void SuperIoBecomesAMotherboardDeviceWithRawLabels()
    {
        var superIo = new HardwareNode(
            "/lpc/it8689e/0",
            HardwareType.SuperIO,
            "ITE IT8689E",
            [
                Sensor("/lpc/it8689e/0/temperature/0", SensorType.Temperature, "System", 0),
                Sensor("/lpc/it8689e/0/fan/1", SensorType.Fan, "CPU Fan", 1),
                Sensor("/lpc/it8689e/0/control/0", SensorType.Control, "Fan #1 control", 0),
            ],
            []);
        var motherboard = new HardwareNode("/motherboard", HardwareType.Motherboard, "Gigabyte B650 GAMING X AX", [], [superIo]);

        BuiltSchema schema = Build([motherboard], pawnIoAvailable: true);

        WireDevice device = Assert.Single(schema.Schema.Devices);
        Assert.Equal("motherboard", device.Kind);
        Assert.Equal("ITE IT8689E", device.Name);
        Assert.Null(device.Hint);

        Assert.Equal(2, schema.Schema.Sensors.Count);
        WireSensor temp = Find(schema, device.Id, "temperature", "lhm-temperature-0");
        Assert.Equal("lhm.raw", temp.LabelKey);
        Assert.Equal("System", temp.LabelArg);
        WireSensor fan = Find(schema, device.Id, "fan", "lhm-fan-1");
        Assert.Equal("lhm.raw", fan.LabelKey);
        Assert.Equal("CPU Fan", fan.LabelArg);

        Assert.DoesNotContain(schema.Schema.Sensors, s => s.Kind == "control");
    }

    [Fact]
    public void BareMotherboardWithNoSensorsIsNotPublished()
    {
        var motherboard = new HardwareNode("/motherboard", HardwareType.Motherboard, "Gigabyte B650 GAMING X AX", [], []);

        BuiltSchema schema = Build([motherboard], pawnIoAvailable: true);

        Assert.Empty(schema.Schema.Devices);
    }

    [Fact]
    public void ControllersAndPsusUseFallbackNames()
    {
        var cooler = new HardwareNode(
            "/heatmaster/0",
            HardwareType.Cooler,
            "NZXT Heatmaster",
            [Sensor("/heatmaster/0/fan/0", SensorType.Fan, "Fan #1", 0)],
            []);
        var psu = new HardwareNode(
            "/psu/corsair/0",
            HardwareType.Psu,
            "Corsair HX1000i",
            [Sensor("/psu/corsair/0/power/0", SensorType.Power, "Total Power", 0)],
            []);

        BuiltSchema schema = Build([cooler, psu], pawnIoAvailable: true);

        Assert.Equal(2, schema.Schema.Devices.Count);
        WireDevice fanController = Assert.Single(schema.Schema.Devices, d => d.Kind == "fan_controller");
        WireDevice psuDevice = Assert.Single(schema.Schema.Devices, d => d.Kind == "psu");

        WireSensor fan = Find(schema, fanController.Id, "fan", "lhm-fan-0");
        Assert.Equal("lhm.raw", fan.LabelKey);
        Assert.Equal("Fan #1", fan.LabelArg);

        WireSensor power = Find(schema, psuDevice.Id, "power", "lhm-power-0");
        Assert.Equal("lhm.raw", power.LabelKey);
        Assert.Equal("Total Power", power.LabelArg);
    }

    [Fact]
    public void UnsupportedSensorTypesAreDropped()
    {
        var cooler = new HardwareNode(
            "/heatmaster/0",
            HardwareType.Cooler,
            "NZXT Heatmaster",
            [
                Sensor("/heatmaster/0/control/0", SensorType.Control, "Fan #1 control", 0),
                Sensor("/heatmaster/0/timespan/0", SensorType.TimeSpan, "Uptime", 0),
                Sensor("/heatmaster/0/energy/0", SensorType.Energy, "Consumption", 0),
                Sensor("/heatmaster/0/noise/0", SensorType.Noise, "Noise", 0),
            ],
            []);

        BuiltSchema schema = Build([cooler], pawnIoAvailable: true);

        Assert.Empty(schema.Schema.Devices);
        Assert.Empty(schema.Schema.Sensors);
    }

    [Fact]
    public void WithoutPawnIoCpuBoardAndDimmsAreNotPublished()
    {
        var cpu = new HardwareNode("/amdcpu/0", HardwareType.Cpu, "AMD Ryzen 7 7800X3D", [Sensor("/amdcpu/0/power/0", SensorType.Power, "Package", 0)], []);
        var superIo = new HardwareNode("/lpc/it8689e/0", HardwareType.SuperIO, "ITE IT8689E", [Sensor("/lpc/it8689e/0/temperature/0", SensorType.Temperature, "System", 0)], []);
        var motherboard = new HardwareNode("/motherboard", HardwareType.Motherboard, "Gigabyte B650 GAMING X AX", [], [superIo]);
        var ram = new HardwareNode("/ram", HardwareType.Memory, "Total Memory", [Sensor("/ram/load/0", SensorType.Load, "Memory", 0)], []);
        var dimm = new HardwareNode("/memory/dimm/1", HardwareType.Memory, "DIMM #1", [Sensor("/memory/dimm/1/temperature/0", SensorType.Temperature, "DIMM #1", 0)], []);
        var disk = new HardwareNode("/hdd/0", HardwareType.Storage, "Drive", [Sensor("/hdd/0/temperature/0", SensorType.Temperature, "Temperature", 0)], [], new StorageInfo(0, "Drive", null, null, true));

        BuiltSchema schema = Build([cpu, motherboard, ram, dimm, disk], pawnIoAvailable: false);

        Assert.DoesNotContain(schema.Schema.Devices, d => d.Kind == "cpu");
        Assert.DoesNotContain(schema.Schema.Devices, d => d.Kind == "motherboard");
        Assert.DoesNotContain(schema.Schema.Sensors, s => s.LabelKey == "memory.temperature.dimm");

        WireDevice memory = Assert.Single(schema.Schema.Devices, d => d.Kind == "memory");
        Find(schema, memory.Id, "load", "used");

        Assert.Contains(schema.Schema.Devices, d => d.Kind == "storage");
    }

    [Fact]
    public void DeviceIdsAreSha256OfTheIdentifier()
    {
        var superIo = new HardwareNode("/lpc/it8689e/0", HardwareType.SuperIO, "ITE IT8689E", [Sensor("/lpc/it8689e/0/temperature/0", SensorType.Temperature, "System", 0)], []);
        var motherboard = new HardwareNode("/motherboard", HardwareType.Motherboard, "Gigabyte B650 GAMING X AX", [], [superIo]);

        BuiltSchema schema = Build([motherboard], pawnIoAvailable: true);

        WireDevice device = Assert.Single(schema.Schema.Devices);
        Assert.Matches("^lhm-[0-9a-f]{64}$", device.Id);
        Assert.Equal("lhm-" + Sha256HexOf("/lpc/it8689e/0"), device.Id);
    }
}
