using System.Text.Json;
using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

public sealed class UmcTimingsTests
{
    private const string Zen4Expected =
        """{"generation":"ddr5","mclkMhz":3000,"dataRate":6000,"cmd2t":false,"gdm":true,"refreshMode":"mixed","cl":36,"trcdRd":48,"trcdWr":48,"trp":48,"tras":104,"trc":152,"trrdS":8,"trrdL":16,"tfaw":32,"twtrS":8,"twtrL":32,"tcwl":34,"twr":72,"trtp":24,"trefi":11677,"trfc":943,"trfc2":512,"trfcNs":170.667}""";

    private static readonly CpuInfo Zen4 = new("AMD", "AMD Ryzen 7 7800X3D 8-Core Processor", 0x19, 0x61, null);

    /// <summary>The registers of channel 0 of the spike dump (SMN offsets within a channel).</summary>
    private static Dictionary<uint, uint> DumpChannel()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "umc-7800x3d.json");
        using JsonDocument doc = JsonDocument.Parse(File.ReadAllText(path));
        var registers = new Dictionary<uint, uint>();
        foreach (JsonProperty p in doc.RootElement.GetProperty("channels").GetProperty("0").EnumerateObject())
        {
            registers[Convert.ToUInt32(p.Name, 16)] = Convert.ToUInt32(p.Value.GetString(), 16);
        }

        return registers;
    }

    private static List<UmcChannel> Channels(params Dictionary<uint, uint>[] registers) =>
        [.. registers.Select((r, i) => new UmcChannel(i, r))];

    private static Dictionary<uint, uint> With(Dictionary<uint, uint> source, uint address, uint value) =>
        new(source) { [address] = value };

    private static JsonElement Parse(string? json)
    {
        Assert.NotNull(json);
        return JsonDocument.Parse(json).RootElement;
    }

    [Fact]
    public void Ddr5DumpOfThisMachineMatchesZenTimings()
    {
        string? json = UmcTimings.Decode(Channels(DumpChannel(), DumpChannel()), DramGeneration.Ddr5);

        Assert.Equal(Zen4Expected, json);
    }

    [Fact]
    public void PerBankOnlyModeUsesTrfcForTheNanoseconds()
    {
        uint refresh = (DumpChannel()[0x5012C] & ~(7u << 16)) | 2; // per-bank, FGR 0

        JsonElement timings = Parse(UmcTimings.Decode(Channels(With(DumpChannel(), 0x5012C, refresh)), DramGeneration.Ddr5));

        Assert.Equal("pbOnly", timings.GetProperty("refreshMode").GetString());
        Assert.Equal(314.333, timings.GetProperty("trfcNs").GetDouble(), 3);
    }

    [Fact]
    public void TrfcSbTakesTheFirstNonZeroRegister()
    {
        Dictionary<uint, uint> regs = With(DumpChannel(), 0x502C4, 416);

        JsonElement timings = Parse(UmcTimings.Decode(Channels(regs), DramGeneration.Ddr5));

        Assert.Equal(416, timings.GetProperty("trfcSb").GetInt32());
    }

    [Fact]
    public void ChannelsWithDifferentTimingsAreListedPerChannel()
    {
        Dictionary<uint, uint> b = With(DumpChannel(), 0x50204, (DumpChannel()[0x50204] & ~0x3Fu) | 40);

        JsonElement timings = Parse(UmcTimings.Decode(Channels(DumpChannel(), b), DramGeneration.Ddr5));

        Assert.Equal("ddr5", timings.GetProperty("generation").GetString());
        Assert.False(timings.TryGetProperty("cl", out _));
        JsonElement channels = timings.GetProperty("channels");
        Assert.Equal(2, channels.GetArrayLength());
        Assert.Equal("A", channels[0].GetProperty("channel").GetString());
        Assert.Equal(36, channels[0].GetProperty("cl").GetInt32());
        Assert.Equal("B", channels[1].GetProperty("channel").GetString());
        Assert.Equal(40, channels[1].GetProperty("cl").GetInt32());
    }

    [Theory]
    [InlineData(0x50204u, 0u)] // CL 0 and every other field of the register 0
    [InlineData(0x50204u, 0x30306800u | 2)] // CL 2 (the 6-bit field cannot hold 90; 80 is the upper bound)
    [InlineData(0x50200u, 0x80050000u | 300)] // MCLK 300 MHz
    public void ImplausibleChannelsAreDropped(uint address, uint value)
    {
        var dropped = new List<string>();
        string? json = UmcTimings.Decode(Channels(With(DumpChannel(), address, value)), DramGeneration.Ddr5, dropped.Add);

        Assert.Null(json);
        Assert.Single(dropped);
    }

    [Fact]
    public void AnImplausibleChannelDoesNotHideTheGoodOne()
    {
        Dictionary<uint, uint> bad = With(DumpChannel(), 0x50204, 0);

        JsonElement timings = Parse(UmcTimings.Decode(Channels(DumpChannel(), bad), DramGeneration.Ddr5));

        Assert.Equal(36, timings.GetProperty("cl").GetInt32());
    }

    [Fact]
    public void Ddr4LayoutDecodes()
    {
        var regs = new Dictionary<uint, uint>
        {
            [0x50200] = 54, // ratio 54 / 3 = 18 -> MCLK 1800, 1T, GDM off
            [0x50204] = (18u << 24) | (18u << 16) | (36u << 8) | 16,
            [0x50208] = (18u << 16) | 54,
            [0x50210] = 28,
            [0x50260] = 560 | (350u << 11) | (280u << 22),
        };

        JsonElement t = Parse(UmcTimings.Decode(Channels(regs), DramGeneration.Ddr4));

        Assert.Equal("ddr4", t.GetProperty("generation").GetString());
        Assert.Equal(1800, t.GetProperty("mclkMhz").GetInt32());
        Assert.Equal(3600, t.GetProperty("dataRate").GetInt32());
        Assert.False(t.GetProperty("cmd2t").GetBoolean());
        Assert.Equal(16, t.GetProperty("cl").GetInt32());
        Assert.Equal(18, t.GetProperty("trcdRd").GetInt32());
        Assert.Equal(18, t.GetProperty("trcdWr").GetInt32());
        Assert.Equal(18, t.GetProperty("trp").GetInt32());
        Assert.Equal(36, t.GetProperty("tras").GetInt32());
        Assert.Equal(54, t.GetProperty("trc").GetInt32());
        Assert.Equal(28, t.GetProperty("tfaw").GetInt32());
        Assert.Equal("normal", t.GetProperty("refreshMode").GetString());
        Assert.Equal(560, t.GetProperty("trfc").GetInt32());
        Assert.Equal(350, t.GetProperty("trfc2").GetInt32());
        Assert.False(t.TryGetProperty("trfcSb", out _));
        Assert.Equal(311.111, t.GetProperty("trfcNs").GetDouble());
    }

    [Fact]
    public void Ddr4OmitsTrfc2WhenZero()
    {
        var regs = new Dictionary<uint, uint> { [0x50200] = 54, [0x50204] = (18u << 24) | (18u << 16) | (36u << 8) | 16, [0x50260] = 560 };

        JsonElement t = Parse(UmcTimings.Decode(Channels(regs), DramGeneration.Ddr4));

        Assert.False(t.TryGetProperty("trfc2", out _));
    }

    [Theory]
    [InlineData("AMD", 0x19, 0x60, DramGeneration.Ddr5)]
    [InlineData("AMD", 0x19, 0x7F, DramGeneration.Ddr5)]
    [InlineData("AMD", 0x1A, 0x40, DramGeneration.Ddr5)]
    [InlineData("AMD", 0x1A, 0x4F, DramGeneration.Ddr5)]
    [InlineData("AMD", 0x17, 0x71, DramGeneration.Ddr4)]
    [InlineData("AMD", 0x19, 0x21, DramGeneration.Ddr4)]
    public void GenerationFollowsTheCpuTable(string vendor, int family, int model, DramGeneration expected) =>
        Assert.Equal(expected, UmcTimings.Generation(new CpuInfo(vendor, "x", family, model, null)));

    [Theory]
    [InlineData("AMD", 0x19, 0x50)]
    [InlineData("AMD", 0x19, 0x80)]
    [InlineData("AMD", 0x1A, 0x50)]
    [InlineData("AMD", 0x17, 0x01)]
    [InlineData("Intel", 0x19, 0x61)]
    [InlineData("Intel", 6, 0xB7)]
    public void GenerationIsNullForOtherCpus(string vendor, int family, int model) =>
        Assert.Null(UmcTimings.Generation(new CpuInfo(vendor, "x", family, model, null)));

    private static FakeTree TreeWith(params int[] channels)
    {
        var tree = new FakeTree();
        for (uint c = 0; c < 12; c++)
        {
            tree.Smn[(c << 20) | 0x50DF0] = 1u << 19; // disabled unless the dump below replaces it
        }

        foreach (int channel in channels)
        {
            foreach ((uint address, uint value) in DumpChannel())
            {
                tree.Smn[((uint)channel << 20) | address] = value;
            }
        }

        return tree;
    }

    [Fact]
    public void ReadSkipsAbsentChannels()
    {
        FakeTree tree = TreeWith(0);

        JsonElement timings = Parse(UmcTimings.Read(tree.ReadSmn, Zen4));

        Assert.Equal(36, timings.GetProperty("cl").GetInt32());
        Assert.False(timings.TryGetProperty("channels", out _));
        // Channels 1-11 cost one read (0x50DF0) each; channel 0 costs its three presence registers plus 17 timing registers.
        Assert.Equal(11 + 3 + 17, tree.SmnReads);
    }

    [Fact]
    public void ReadSkipsDisabledChannelsAndChannelsWithoutDimms()
    {
        FakeTree tree = TreeWith(0, 1);
        tree.Smn[(1u << 20) | 0x50008] = 0; // channel 1: no DIMM
        tree.Smn[0x50DF0] = 1u << 19; // channel 0: disabled

        Assert.Null(UmcTimings.Read(tree.ReadSmn, Zen4));
    }

    [Fact]
    public void AMissingTimingRegisterMakesTheReadIncomplete()
    {
        FakeTree tree = TreeWith(0);
        tree.Smn.TryRemove(0x50214, out _);

        bool complete = UmcTimings.TryRead(tree.ReadSmn, Zen4, out string? json, null);

        Assert.False(complete);
        Assert.Null(json);
        Assert.Null(UmcTimings.Read(tree.ReadSmn, Zen4));
    }

    [Fact]
    public void AMissingPresenceRegisterMakesTheReadIncomplete()
    {
        FakeTree tree = TreeWith(0);
        tree.Smn.TryRemove(5u << 20 | 0x50DF0, out _);

        Assert.False(UmcTimings.TryRead(tree.ReadSmn, Zen4, out string? json, null));
        Assert.Null(json);
    }

    [Fact]
    public void AnEmptyMachineIsAFinalAnswer()
    {
        FakeTree tree = TreeWith();

        Assert.True(UmcTimings.TryRead(tree.ReadSmn, Zen4, out string? json, null));
        Assert.Null(json);
    }

    [Fact]
    public void AnIncompleteReadIsRetriedByTheNextSchemaBuildAndThenPublished()
    {
        FakeTree tree = TreeWith(0);
        tree.Smn.TryRemove(0x50214, out _);
        tree.Initial.Add(CpuRoot());
        tree.Initial.Add(RamRoot());
        var time = new FakeTimeProvider();
        using var hub = new SensorHub(tree, new FakeDisks(), new FakeActivity(), () => true, time, new ListLogger<SensorHub>()) { StartWorkers = false };
        var updates = new List<FeedUpdate>();
        hub.Subscribe(1000, updates.Add);
        hub.TickOnce();
        Assert.False(updates.Last(u => u.Schema is not null).Schema!.Devices.Single(d => d.Kind == "memory").Properties.ContainsKey("memoryTimings"));

        tree.Smn[0x50214] = DumpChannel()[0x50214];
        tree.Replace(CpuRoot(), RamRoot()); // raises HardwareChanged: the schema is built again
        time.Advance(TimeSpan.FromSeconds(1));
        hub.TickOnce();

        Assert.Equal(Zen4Expected, updates.Last(u => u.Schema is not null).Schema!.Devices.Single(d => d.Kind == "memory").Properties["memoryTimings"]);
    }

    [Fact]
    public void ReadDoesNothingForAnUnsupportedCpu()
    {
        FakeTree tree = TreeWith(0);

        Assert.Null(UmcTimings.Read(tree.ReadSmn, new CpuInfo("Intel", "x", 6, 0xB7, null)));
        Assert.Equal(0, tree.SmnReads);
    }

    private static HardwareNode CpuRoot() => new(
        "/amdcpu/0",
        HardwareType.Cpu,
        "AMD Ryzen 7 7800X3D",
        [new SensorNode("/amdcpu/0/load/0", SensorType.Load, "CPU Total", 0)],
        [],
        Cpu: Zen4);

    private static HardwareNode RamRoot() =>
        new("/ram", HardwareType.Memory, "Total Memory", [new SensorNode("/ram/data/0", SensorType.Data, "Memory Used", 0)], []);

    private static IReadOnlyDictionary<string, string> MemoryProperties(bool pawnIo, FakeTree tree)
    {
        tree.Initial.Add(CpuRoot());
        tree.Initial.Add(RamRoot());
        using var hub = new SensorHub(tree, new FakeDisks(), new FakeActivity(), () => pawnIo, new FakeTimeProvider(), new ListLogger<SensorHub>()) { StartWorkers = false };
        var updates = new List<FeedUpdate>();
        hub.Subscribe(1000, updates.Add);
        hub.TickOnce();
        SchemaMessage schema = updates.Last(u => u.Schema is not null).Schema!;
        return schema.Devices.Single(d => d.Kind == "memory").Properties;
    }

    [Fact]
    public void WithoutPawnIoNoUmcRead()
    {
        FakeTree tree = TreeWith(0);

        IReadOnlyDictionary<string, string> properties = MemoryProperties(pawnIo: false, tree);

        Assert.Equal(0, tree.SmnReads);
        Assert.False(properties.ContainsKey("memoryTimings"));
    }

    [Fact]
    public void WithPawnIoTheSchemaCarriesTheTimings()
    {
        FakeTree tree = TreeWith(0, 1);

        IReadOnlyDictionary<string, string> properties = MemoryProperties(pawnIo: true, tree);

        Assert.Equal(Zen4Expected, properties["memoryTimings"]);
    }
}
