using LibreHardwareMonitor.Hardware;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

public sealed partial class SensorHubTests
{
    private const string SataId = "/hdd/0";

    private static HardwareNode SataHdd() => Hdd() with { Storage = Hdd().Storage! with { HasAtaHealth = true, AtaSectorAttributes = 7 } };

    private static IReadOnlyList<AtaAttribute> SataAttributes(uint reallocated, byte current = 100) =>
    [
        new AtaAttribute(0x05, current, 36, reallocated),
        new AtaAttribute(0xC5, 100, 0, 0),
        new AtaAttribute(0xC6, 100, 0, 0),
    ];

    private static Harness SataHarness(out List<FeedUpdate> updates, out IFeedSubscription subscription)
    {
        var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(SataHdd());
        h.Tree.Values[HddTemp] = 34;
        h.Tree.AtaAttributes[SataId] = SataAttributes(8);
        updates = h.Subscribe(1000, out subscription);
        return h;
    }

    [Fact]
    public void SataHealthIsReadInTheSmartPass()
    {
        using Harness h = SataHarness(out List<FeedUpdate> a, out _);
        int readsAtUpdate = -1;
        h.Tree.BeforeUpdate = root => readsAtUpdate = root.Identifier == SataId ? h.Tree.AtaAttributeReads(SataId) : readsAtUpdate;
        h.Hub.TickOnce();
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(0, readsAtUpdate); // read after the update, never before
        Assert.Equal(0, ValueOf(a, a.Count - 1, "flag", "critical-warning"));
        Assert.Equal(1, ValueOf(a, a.Count - 1, "flag", "sector-warning"));
        Assert.Equal(8, ValueOf(a, a.Count - 1, "counter", "reallocated-sectors"));
        Assert.Equal(0, ValueOf(a, a.Count - 1, "counter", "pending-sectors"));
        Assert.Equal(0, ValueOf(a, a.Count - 1, "counter", "uncorrectable-sectors"));
        Assert.Equal(1, h.Tree.AtaAttributeReads(SataId));
        h.Advance(1000);
        h.Hub.TickOnce(); // the sampler reads the cache, not the tree
        Assert.Equal(1, h.Tree.AtaAttributeReads(SataId));
    }

    [Fact]
    public void RestingSataDiskKeepsHealthValues()
    {
        using Harness h = SataHarness(out List<FeedUpdate> a, out _);
        h.Hub.TickOnce();
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.False(HeldOf(a, a.Count - 1, "flag", "sector-warning"));

        // Asleep: neither updated nor read (the tree would now answer a failing disk); the last values stay, held.
        h.Disks.SpunDown[0] = true;
        h.Tree.AtaAttributes[SataId] = SataAttributes(0, current: 30);
        int reads = h.Tree.AtaAttributeReads(SataId);
        for (int round = 0; round < 3; round++)
        {
            h.Advance(30_000);
            h.WorkingRound();
            h.Advance(1000);
            h.Hub.TickOnce();
            Assert.Equal(0, ValueOf(a, a.Count - 1, "flag", "critical-warning"));
            Assert.Equal(1, ValueOf(a, a.Count - 1, "flag", "sector-warning"));
            Assert.Equal(8, ValueOf(a, a.Count - 1, "counter", "reallocated-sectors"));
            Assert.True(HeldOf(a, a.Count - 1, "flag", "sector-warning"));
            Assert.True(HeldOf(a, a.Count - 1, "counter", "reallocated-sectors"));
        }

        Assert.Equal(reads, h.Tree.AtaAttributeReads(SataId));
    }

    [Fact]
    public void SataHealthAbsentWhenSmartIsOff()
    {
        using Harness h = SataHarness(out List<FeedUpdate> a, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal(1, ValueOf(a, a.Count - 1, "flag", "sector-warning"));

        sub.Update(Requests.Of(1000, ServiceModules.None, HddKey));
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.DoesNotContain(LatestSchema(a).Sensors, s => s.Kind is "flag" or "counter");
        int reads = h.Tree.AtaAttributeReads(SataId);

        h.Advance(30_000);
        h.Hub.RunStorageDue();
        Assert.Equal(reads, h.Tree.AtaAttributeReads(SataId)); // SMART off: no update, no read
    }
}
