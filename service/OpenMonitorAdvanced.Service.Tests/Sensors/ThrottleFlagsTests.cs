using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

public sealed class ThrottleFlagsTests
{
    private const string CpuId = "/intelcpu/0";

    private static CpuInfo Intel(bool readable, bool pln) => new("Intel", "Intel(R) Core(TM) i7-9700K", 6, 0x9E, null, readable, pln);

    private static HardwareNode CpuRoot(CpuInfo info, string id = CpuId, string name = "Intel Core i7-9700K") => new(
        id,
        HardwareType.Cpu,
        name,
        [new SensorNode(id + "/load/0", SensorType.Load, "CPU Total", 0)],
        [],
        Cpu: info);

    private static (FakeTree Tree, SensorHub Hub, List<FeedUpdate> Updates) Start(FakeTimeProvider time, params HardwareNode[] cpus)
    {
        var tree = new FakeTree();
        tree.Initial.AddRange(cpus);
        var hub = new SensorHub(tree, new FakeDisks(), new FakeActivity(), () => true, time, new ListLogger<SensorHub>()) { StartWorkers = false };
        var updates = new List<FeedUpdate>();
        hub.Subscribe(1000, updates.Add);
        return (tree, hub, updates);
    }

    private static SchemaMessage Schema(List<FeedUpdate> updates) => updates.Last(u => u.Schema is not null).Schema!;

    private static double? Value(List<FeedUpdate> updates, string name)
    {
        SchemaMessage schema = Schema(updates);
        int index = schema.Sensors.ToList().FindIndex(s => s.Name == name);
        Assert.True(index >= 0, name + " is not in the schema");
        return updates.Last().Snapshot.Values[index];
    }

    [Fact]
    public void IntelWithReadableThermStatusPublishesThrottleFlags()
    {
        var (_, hub, updates) = Start(new FakeTimeProvider(), CpuRoot(Intel(true, true)));
        using (hub)
        {
            hub.TickOnce();
            WireSensor[] flags = [.. Schema(updates).Sensors.Where(s => s.Kind == "flag")];
            Assert.Equal(["throttle-power", "throttle-thermal"], flags.Select(f => f.Name).Order());
            Assert.All(flags, f => Assert.Equal("boolean", f.Unit));
            Assert.Contains(flags, f => f.LabelKey == "cpu.throttle.thermal");
            Assert.Contains(flags, f => f.LabelKey == "cpu.throttle.power");
        }
    }

    [Fact]
    public void PowerFlagNeedsPowerLimitNotification()
    {
        var (_, hub, updates) = Start(new FakeTimeProvider(), CpuRoot(Intel(true, false)));
        using (hub)
        {
            hub.TickOnce();
            Assert.Equal(["throttle-thermal"], Schema(updates).Sensors.Where(s => s.Kind == "flag").Select(f => f.Name));
        }
    }

    [Fact]
    public void AmdHasNoThrottleFlags()
    {
        var (_, hub, updates) = Start(new FakeTimeProvider(), CpuRoot(new CpuInfo("AMD", "AMD Ryzen 7 7800X3D", 0x19, 0x61, null), "/amdcpu/0", "AMD Ryzen 7 7800X3D"));
        using (hub)
        {
            hub.TickOnce();
            Assert.DoesNotContain(Schema(updates).Sensors, s => s.Kind == "flag");
        }
    }

    [Fact]
    public void UnreadableThermStatusHasNoFlags()
    {
        var (_, hub, updates) = Start(new FakeTimeProvider(), CpuRoot(Intel(false, false)));
        using (hub)
        {
            hub.TickOnce();
            Assert.DoesNotContain(Schema(updates).Sensors, s => s.Kind == "flag");
        }
    }

    [Theory]
    [InlineData(0x1UL, 1.0, 0.0)]
    [InlineData(0x400UL, 0.0, 1.0)]
    [InlineData(0x401UL, 1.0, 1.0)]
    [InlineData(0x0UL, 0.0, 0.0)]
    public void ThermStatusBitsBecomeFlags(ulong status, double thermal, double power)
    {
        var (tree, hub, updates) = Start(new FakeTimeProvider(), CpuRoot(Intel(true, true)));
        using (hub)
        {
            tree.ThermStatus[CpuId] = status;
            hub.TickOnce();
            Assert.Equal(thermal, Value(updates, "throttle-thermal"));
            Assert.Equal(power, Value(updates, "throttle-power"));
        }
    }

    [Fact]
    public void ThermStatusIsReadOncePerCpuPerTick()
    {
        var (tree, hub, _) = Start(new FakeTimeProvider(), CpuRoot(Intel(true, true)), CpuRoot(Intel(true, true), "/intelcpu/1"));
        using (hub)
        {
            tree.ThermStatus[CpuId] = 1;
            tree.ThermStatus["/intelcpu/1"] = 1;
            hub.TickOnce();
            int afterFirst = tree.ThermStatusReads;
            hub.TickOnce();
            Assert.Equal(2, tree.ThermStatusReads - afterFirst);
        }
    }

    [Fact]
    public void FailedMsrReadGivesAbsentValues()
    {
        var time = new FakeTimeProvider();
        var (tree, hub, updates) = Start(time, CpuRoot(Intel(true, true)));
        using (hub)
        {
            tree.ThermStatus[CpuId] = 0x401;
            hub.TickOnce();
            tree.ThermStatus.Clear();
            time.Advance(TimeSpan.FromSeconds(2));
            hub.TickOnce();
            Assert.Null(Value(updates, "throttle-thermal"));
            Assert.Null(Value(updates, "throttle-power"));
        }
    }
}
