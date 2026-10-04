using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

public sealed partial class SensorHubTests
{
    // ---- M6b Task 8: the drive list and the disks whose SMART is off by default ----

    private const string UsbTemp = "/hdd/4/temperature/0";

    private static readonly string UsbKey = DriveKey.Compute("SanDisk Extreme", "4C530001")!;

    /// <summary>The stick of the M6b spike: USB bus, seek penalty not answered, so it needs a power check.</summary>
    private static DriveFacts UsbStick() =>
        new(4, DriveAvailability.Present, "SanDisk Extreme", "4C530001", DriveFacts.BusTypeUsb, SeekPenalty: null);

    private static HardwareNode UsbRoot() => new(
        "/hdd/4",
        HardwareType.Storage,
        "SanDisk Extreme",
        [new SensorNode(UsbTemp, SensorType.Temperature, "Temperature", 0)],
        [],
        new StorageInfo(4, null, null, "4C530001", Rotational: true));

    private static WireDrive HddDrive(string state, bool blocksSmart = false) => new(0, HddKey, "ST2000DM008-2FR102", state, blocksSmart);

    private static WireDrive WdcDrive(string state, bool blocksSmart = false) => new(1, WdcKey, "WDC WD40EFRX-68N32N0", state, blocksSmart);

    private static WireDrive UsbDrive(string state, bool blocksSmart = false) => new(4, UsbKey, "SanDisk Extreme", state, blocksSmart);

    private static IReadOnlyList<WireDrive> Drives(IReadOnlyList<FeedUpdate> updates) => LatestSchema(updates).Service.Drives;

    /// <summary>The temperature of that physical drive in the latest update, resolved against the latest schema.</summary>
    private static double? DriveTemperature(IReadOnlyList<FeedUpdate> updates, uint drive)
    {
        SchemaMessage schema = LatestSchema(updates);
        WireDevice disk = Assert.Single(schema.Devices, d => d.Hint is StorageHint hint && hint.PhysicalDrive == drive);
        int index = schema.Sensors.ToList().FindIndex(s => s.DeviceId == disk.Id && s.Kind == "temperature");
        Assert.True(index >= 0, $"drive {drive} has no temperature in the schema");
        return updates[^1].Snapshot.Values[index];
    }

    /// <summary>The <c>held</c> flag of that physical drive's temperature in the latest update.</summary>
    private static bool DriveHeld(IReadOnlyList<FeedUpdate> updates, uint drive)
    {
        SchemaMessage schema = LatestSchema(updates);
        WireDevice disk = Assert.Single(schema.Devices, d => d.Hint is StorageHint hint && hint.PhysicalDrive == drive);
        int index = schema.Sensors.ToList().FindIndex(s => s.DeviceId == disk.Id && s.Kind == "temperature");
        Assert.True(index >= 0, $"drive {drive} has no temperature in the schema");
        return updates[^1].Snapshot.Held[index];
    }

    /// <summary>A harness with the HDD (drive 0) and the USB stick (drive 4) both in LHM's storage group, and one subscriber.</summary>
    private static Harness UsbHarness(FeedRequest request, out List<FeedUpdate> updates, out IFeedSubscription subscription)
    {
        var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(UsbRoot());
        h.Tree.Values[HddTemp] = 40;
        h.Tree.Values[UsbTemp] = 30;
        h.Disks.Facts[4] = UsbStick();
        updates = h.Subscribe(request, out subscription);
        h.Hub.TickOnce();
        return h;
    }

    /// <summary>
    /// One storage round (when due) and the sampling tick that publishes it. A disk whose
    /// counters grow at every read (the fake's default) counts as working since the round
    /// before, and one with scripted counters as quiet.
    /// </summary>
    private static void RoundAndTick(Harness h)
    {
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
    }

    [Fact]
    public void AnActiveUsbDiskDoesNotBlockTheGateAndIsSmartOff()
    {
        using Harness h = UsbHarness(Requests.Of(1000), out List<FeedUpdate> a, out _);
        Assert.Empty(Drives(a)); // nothing is known before the first storage round

        RoundAndTick(h);

        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal([HddDrive("active"), UsbDrive("smartOff")], Drives(a));
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
        Assert.Equal(0, h.Tree.Updates("/hdd/4"));
        Assert.Equal(0, h.Tree.Reads(UsbTemp));

        // Out of the devices like a disk whose SMART a client switched off, but not listed as one.
        SchemaMessage schema = LatestSchema(a);
        Assert.Equal(new StorageHint(0, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL"), Assert.Single(schema.Devices, d => d.Kind == "storage").Hint);
        Assert.Empty(schema.Service.SmartDisabledDrives);
        Assert.Equal(40, ValueOf(a, a.Count - 1, "temperature", "drive"));
    }

    [Theory]
    [InlineData(true)]
    [InlineData(null)]
    public void AUsbDiskInStandbyBlocksTheGateEvenWhenSmartOff(bool? spunDown)
    {
        using Harness h = UsbHarness(Requests.Of(1000), out List<FeedUpdate> a, out _);
        h.Disks.SpunDown[4] = spunDown;

        RoundAndTick(h);

        // The first identification would touch it whatever its switch says: its state stays
        // "smartOff", and only the flag tells that it is what keeps SMART off for every disk.
        Assert.Equal(0, h.Tree.EnableStorageCount);
        Assert.Equal([HddDrive("active"), UsbDrive("smartOff", blocksSmart: true)], Drives(a));
        Assert.Equal(0, h.Tree.Updates("/hdd/0"));

        h.Disks.SpunDown[4] = false;
        h.Advance(30_000);
        RoundAndTick(h);
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal([HddDrive("active"), UsbDrive("smartOff")], Drives(a));
    }

    [Fact]
    public void AnEnabledUsbDiskIsPowerCheckedAndUpdated()
    {
        using Harness h = UsbHarness(Requests.Of(1000).WithSmartOn(UsbKey), out List<FeedUpdate> a, out _);

        for (int round = 1; round <= 2; round++)
        {
            RoundAndTick(h);
            Assert.Equal(round, h.Disks.SpunDownQueriesOf(4));
            Assert.Equal(round, h.Tree.Updates("/hdd/4"));
            Assert.Equal([HddDrive("active"), UsbDrive("active")], Drives(a));
            h.Advance(30_000);
        }

        Assert.Equal(30, DriveTemperature(a, 4));

        // Enabled, it is treated like any disk that needs a power check: asleep, it is left alone.
        h.Disks.SpunDown[4] = true;
        RoundAndTick(h);
        Assert.Equal(3, h.Disks.SpunDownQueriesOf(4));
        Assert.Equal(2, h.Tree.Updates("/hdd/4"));
        Assert.Equal([HddDrive("active"), UsbDrive("standby")], Drives(a));
    }

    [Fact]
    public void AnEnabledUsbDiskIsSwitchedOffAgainWhenItsLastClientLeavesIt()
    {
        using Harness h = UsbHarness(Requests.Of(1000).WithSmartOn(UsbKey), out List<FeedUpdate> a, out IFeedSubscription sub);
        RoundAndTick(h);
        Assert.Equal([HddDrive("active"), UsbDrive("active")], Drives(a));
        Assert.Equal(2, LatestSchema(a).Devices.Count(d => d.Kind == "storage"));

        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();
        Assert.Equal(new StorageHint(0, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL"), Assert.Single(LatestSchema(a).Devices, d => d.Kind == "storage").Hint); // at once

        RoundAndTick(h); // the storage worker takes the request: a round at once
        Assert.Equal([HddDrive("active"), UsbDrive("smartOff")], Drives(a));
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
        Assert.Equal(1, h.Disks.SpunDownQueriesOf(4));
        Assert.Equal(1, h.Tree.Updates("/hdd/4"));
    }

    [Fact]
    public void TheDriveListCoversDisksThatLhmDoesNotExpose()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd()); // LHM identified drive 0 only
        h.Disks.Facts[1] = WdcFacts();
        h.Disks.Facts[2] = new DriveFacts(2, DriveAvailability.Present, "Fanxiang S880 2TB", "NVME-SERIAL", DriveFacts.BusTypeNvme, SeekPenalty: false);
        h.Disks.Facts[3] = new DriveFacts(3, DriveAvailability.Present, "USB Bridge", null, DriveFacts.BusTypeUsb, SeekPenalty: null); // no serial: no key
        h.Disks.Facts[6] = new DriveFacts(6, DriveAvailability.NoMedia, null, null, null, null); // an empty card reader
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();

        RoundAndTick(h);

        Assert.Equal(
            [
                HddDrive("active"),
                WdcDrive("active"),
                new WireDrive(2, NvmeKey, "Fanxiang S880 2TB", "active", false),
                new WireDrive(3, null, "USB Bridge", "smartOff", false),
                new WireDrive(6, null, null, "noMedia", false),
            ],
            Drives(a));
        Assert.Single(LatestSchema(a).Devices, d => d.Kind == "storage");
        Assert.Equal(0, h.Disks.SpunDownQueriesOf(2)); // NVMe: never asked
        Assert.Equal(0, h.Disks.SpunDownQueriesOf(6));
    }

    [Fact]
    public void EveryDriveIsListedWhileTheGateIsClosed()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.Facts[1] = WdcFacts();
        h.Disks.Facts[3] = new DriveFacts(3, DriveAvailability.Present, "SATA Dock", null, BusType: 0x0B, SeekPenalty: null); // no serial: no key, but still listed
        h.Disks.SpunDown[1] = true;
        h.Disks.SpunDown[3] = null;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        Assert.Empty(Drives(a));

        RoundAndTick(h);
        Assert.Equal(
            [
                HddDrive("active"),
                WdcDrive("standby", blocksSmart: true),
                new WireDrive(3, null, "SATA Dock", "unknown", true),
            ],
            Drives(a));
        Assert.Equal(0, h.Tree.EnableStorageCount);
        int revision = h.Hub.Revision;

        // The dock has no key, so no counters: it is asked again only after five minutes. The
        // other blocker is asked when it worked since the round before.
        h.Disks.SpunDown[1] = false;
        h.Disks.SpunDown[3] = false;
        h.Advance(270_000);
        RoundAndTick(h);
        Assert.Equal(0, h.Tree.EnableStorageCount);
        h.Advance(29_000);
        RoundAndTick(h);
        Assert.Equal([HddDrive("active"), WdcDrive("active"), new WireDrive(3, null, "SATA Dock", "active", false)], Drives(a));
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.True(h.Hub.Revision > revision);
    }

    [Fact]
    public void AfterTheGateOpensNoDriveBlocksSmart()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.SpunDown[0] = true;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        RoundAndTick(h);
        Assert.Equal([HddDrive("standby", blocksSmart: true)], Drives(a));

        h.Disks.SpunDown[0] = false;
        h.Advance(30_000);
        RoundAndTick(h);
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal(1, h.Tree.EnableStorageCount);

        // Asleep again after the identification: only its own updates stop, nothing is blocked.
        h.Disks.SpunDown[0] = true;
        h.Advance(30_000);
        RoundAndTick(h);
        Assert.Equal([HddDrive("standby")], Drives(a));
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void StorageOffKeepsTheLastDriveListWithoutDiskIo()
    {
        using Harness h = UsbHarness(Requests.Of(1000), out List<FeedUpdate> a, out IFeedSubscription sub);
        h.Disks.SpunDown[4] = null; // the stick keeps the gate closed
        RoundAndTick(h);
        Assert.Equal([HddDrive("active"), UsbDrive("smartOff", blocksSmart: true)], Drives(a));
        var before = (h.Disks.DescribeCalls, h.Disks.SpunDownQueries, h.Disks.EnumerateCalls, h.Tree.EnableStorageCount, h.Activity.Reads, h.Activity.PowerQueries);

        sub.Update(Requests.Of(1000, ServiceModules.Storage));
        h.Disks.Facts[5] = WdcFacts() with { DriveNumber = 5 }; // plugged in afterwards: nobody looks
        for (int round = 0; round < 3; round++)
        {
            h.Hub.RunDue();
            h.Hub.RunStorageDue();
            h.Advance(30_000);
        }

        h.Hub.RunDue();
        // The last known drives, none of them queried and none waiting at a gate.
        Assert.Equal([HddDrive("smartOff"), UsbDrive("smartOff")], Drives(a));
        Assert.DoesNotContain("storage", LatestSchema(a).Service.ActiveModules);
        Assert.Equal(before, (h.Disks.DescribeCalls, h.Disks.SpunDownQueries, h.Disks.EnumerateCalls, h.Tree.EnableStorageCount, h.Activity.Reads, h.Activity.PowerQueries));
    }

    [Fact]
    public void StorageNeverEnabledPublishesNoDrives()
    {
        using Harness h = UsbHarness(Requests.Of(1000, ServiceModules.Storage), out List<FeedUpdate> a, out _);

        for (int round = 0; round < 3; round++)
        {
            RoundAndTick(h);
            h.Advance(30_000);
        }

        Assert.Empty(Drives(a));
        Assert.All(a, u => Assert.Empty((u.Schema?.Service ?? ServiceStateBlock.AllActive).Drives));
        Assert.Equal((0, 0, 0, 0, 0), (h.Disks.EnumerateCalls, h.Disks.DescribeCalls, h.Disks.SpunDownQueries, h.Activity.Reads, h.Activity.PowerQueries));
    }

    [Fact]
    public void ADriveStateChangeBumpsTheSchemaRevision()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        RoundAndTick(h);
        SchemaMessage active = LatestSchema(a);
        Assert.Equal([HddDrive("active")], active.Service.Drives);
        int revision = h.Hub.Revision;

        // The same state again is no change at all.
        h.Advance(30_000);
        RoundAndTick(h);
        Assert.Null(a[^1].Schema);
        Assert.Equal(revision, h.Hub.Revision);

        h.Disks.SpunDown[0] = true;
        h.Advance(30_000);
        RoundAndTick(h);

        // No device and no sensor changed: the drive list alone is a new revision, sent with its snapshot.
        SchemaMessage? standby = a[^1].Schema;
        Assert.NotNull(standby);
        Assert.Equal(revision + 1, h.Hub.Revision);
        Assert.Equal([HddDrive("standby")], standby.Service.Drives);
        Assert.Equal(active.Sensors, standby.Sensors);
        Assert.Equal(active.Devices.Select(d => d.Id), standby.Devices.Select(d => d.Id));
        Assert.Equal(standby.Sensors.Count, a[^1].Snapshot.Values.Count);
    }

    [Fact]
    public void GateChecksAreNotRepeatedForTheDriveList()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        h.Disks.Facts[1] = WdcFacts();
        h.Disks.SpunDown[1] = true;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();

        for (int round = 1; round <= 2; round++)
        {
            RoundAndTick(h);
            Assert.Equal([HddDrive("active"), WdcDrive("standby", blocksSmart: true)], Drives(a));
            Assert.Equal((1, round), (h.Disks.SpunDownQueriesOf(0), h.Disks.SpunDownQueriesOf(1))); // only the working blocker is asked again
            h.Advance(30_000);
        }

        // The round that opens the gate: the blocker's answer, one more of the other disk, and
        // those answers also decide the updates and the list.
        h.Disks.SpunDown[1] = false;
        RoundAndTick(h);
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));
        Assert.Equal((2, 3), (h.Disks.SpunDownQueriesOf(0), h.Disks.SpunDownQueriesOf(1)));
        Assert.Equal((1, 1), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));
        Assert.Equal(3, h.Disks.EnumerateCalls); // one enumeration per round: the opening round does not list the drives again

        // Afterwards one check per working drive and round.
        h.Advance(30_000);
        RoundAndTick(h);
        Assert.Equal((3, 4), (h.Disks.SpunDownQueriesOf(0), h.Disks.SpunDownQueriesOf(1)));
        Assert.Equal(4, h.Disks.EnumerateCalls);
    }

    [Fact]
    public void AnOffUsbDiskReceivesNoPeriodicPowerChecksOrUpdates()
    {
        using Harness h = UsbHarness(Requests.Of(1000), out List<FeedUpdate> a, out _);

        for (int round = 1; round <= 3; round++)
        {
            RoundAndTick(h);
            Assert.Equal(1, h.Disks.SpunDownQueriesOf(4)); // the gate, before the first identification
            Assert.Equal(round, h.Disks.SpunDownQueriesOf(0));
            Assert.Equal(0, h.Tree.Updates("/hdd/4"));
            Assert.Equal(round, h.Tree.Updates("/hdd/0"));
            h.Advance(30_000);
        }

        Assert.Equal(0, h.Tree.Reads(UsbTemp));
        Assert.Equal([HddDrive("active"), UsbDrive("smartOff")], Drives(a));
    }

    [Fact]
    public void AnEnabledDiskAbsentFromLhmStillHasACurrentPowerState()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.Facts[1] = WdcFacts(); // LHM could not identify it
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        RoundAndTick(h);
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));

        h.Disks.SpunDown[1] = true;
        h.Advance(30_000);
        RoundAndTick(h);
        Assert.Equal([HddDrive("active"), WdcDrive("standby")], Drives(a));
        Assert.Equal(2, h.Disks.SpunDownQueriesOf(1));

        h.Disks.SpunDown[1] = null;
        h.Advance(30_000);
        RoundAndTick(h);
        Assert.Equal([HddDrive("active"), WdcDrive("unknown")], Drives(a));
        Assert.Equal(3, h.Disks.SpunDownQueriesOf(1));

        // Switched off by its key, it is not asked any more.
        sub.Update(Requests.Of(1000, ServiceModules.None, WdcKey));
        h.Hub.RunDue();
        RoundAndTick(h);
        h.Advance(30_000);
        RoundAndTick(h);
        Assert.Equal([HddDrive("active"), WdcDrive("smartOff")], Drives(a));
        Assert.Equal(3, h.Disks.SpunDownQueriesOf(1));
    }

    [Fact]
    public void AFailingPowerCheckIsAnUnknownStateForThatDriveOnly()
    {
        var disks = new ThrowingDisks { Facts = { [1] = WdcFacts() } };
        using var h = new Harness(disks);
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        RoundAndTick(h);
        Assert.Equal((1, 1), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));

        disks.ThrowFor = 0;
        h.Advance(30_000);
        RoundAndTick(h);

        Assert.Equal([HddDrive("unknown"), WdcDrive("active")], Drives(a));
        Assert.Equal((1, 2), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));
        Assert.Contains(h.Log.Entries, e => e.Level == LogLevel.Warning && e.Exception is InvalidOperationException { Message: "power check failed" });
    }

    [Fact]
    public void AStorageRequestTakenBeforeTheTreeIsOpenIsAppliedInTheFirstSchema()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        List<FeedUpdate> a = h.Subscribe(Requests.Of(1000, ServiceModules.None, HddKey), out _);

        h.Hub.RunStorageDue(); // the storage worker wakes first: no tree, no round, the request is taken
        h.Hub.TickOnce();

        ServiceStateBlock state = Assert.Single(a).Schema!.Service;
        Assert.Equal("applied", state.Reconfiguration);
        Assert.Equal([HddKey], state.SmartDisabledDrives);
        Assert.Empty(state.Drives);
    }

    [Fact]
    public void AStorageRequestIsAppliedEvenWhenItsRoundCannotListTheDrives()
    {
        var disks = new ThrowingDisks { Facts = { [1] = WdcFacts() } };
        using var h = new Harness(disks);
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        RoundAndTick(h);
        Assert.Equal((1, 1), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));

        // No disk is touched in such a round, so the request is honoured; without this a client
        // would wait on "pending" until the drives can be listed again.
        disks.ThrowOnEnumerate = true;
        sub.Update(Requests.Of(1000, ServiceModules.None, WdcKey));
        h.Hub.RunDue();
        Assert.Equal("pending", LatestSchema(a).Service.Reconfiguration);
        RoundAndTick(h);

        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
        Assert.Equal([WdcKey], LatestSchema(a).Service.SmartDisabledDrives);
        Assert.Equal((1, 1), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));
    }

    /// <summary><see cref="FakeDisks"/> whose power check throws for one drive.</summary>
    private sealed class ThrowingDisks : IDiskPowerProbe
    {
        private readonly FakeDisks _inner = new();

        public System.Collections.Concurrent.ConcurrentDictionary<int, DriveFacts?> Facts => _inner.Facts;

        public int? ThrowFor { get; set; }

        /// <summary>The drives cannot be listed while set.</summary>
        public bool ThrowOnEnumerate { get; set; }

        public bool? IsSpunDown(int driveNumber, string? model, string? serial) =>
            driveNumber == ThrowFor ? throw new InvalidOperationException("power check failed") : _inner.IsSpunDown(driveNumber, model, serial);

        public IReadOnlyList<DriveFacts> Enumerate() =>
            ThrowOnEnumerate ? throw new InvalidOperationException("enumeration failed") : _inner.Enumerate();

        public DriveFacts? Describe(int driveNumber) => _inner.Describe(driveNumber);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void AConcurrentRoundCannotMixDriveStateAndSnapshotValues(bool failingRebuild)
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        h.Tree.Values[HddTemp] = 40;
        h.Tree.Values[WdcTemp] = 35;
        h.Disks.Facts[1] = WdcFacts();
        bool describable = false; // the second disk is listed, but LHM's node of it cannot be described yet
        h.Disks.BeforeDescribe = drive =>
        {
            if (drive == 1 && !describable)
            {
                throw new InvalidOperationException("not yet");
            }
        };
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound(); // round 1: one disk in the schema, active, 40 degrees
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));
        Assert.Equal(40, DriveTemperature(a, 0));
        int revision = h.Hub.Revision;

        // The sampler is stopped inside its tick, after it took the round it works with.
        CancellationToken ct = TestContext.Current.CancellationToken;
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        h.Tree.BeforeUpdate = root =>
        {
            if (root.Type == HardwareType.Cpu)
            {
                entered.Set();
                release.Wait(TimeSpan.FromSeconds(30), ct);
            }
        };
        h.Advance(1000);
        var sampler = new Thread(() => h.Hub.TickOnce()) { IsBackground = true, Name = "test-sampler" };
        sampler.Start();
        Assert.True(entered.Wait(TimeSpan.FromSeconds(10), ct), "the tick never reached the CPU update");

        // Meanwhile the storage worker publishes round 2: the first disk's state is unknown (no
        // value: only a confirmed standby keeps one), the second one is resolved and read. The tick is then made to rebuild its
        // schema after its updates, successfully or not: either way with the round it took.
        h.Disks.SpunDown[0] = null;
        describable = true;
        h.WorkingRound();
        h.Tree.Replace([.. h.Tree.Roots]);
        if (failingRebuild)
        {
            h.Tree.ThrowOnNextRoots();
        }

        int delivered = a.Count;
        release.Set();
        Assert.True(sampler.Join(TimeSpan.FromSeconds(10)));
        h.Tree.BeforeUpdate = null;

        // That tick is round 1 throughout: its drive list, its disks and its values.
        Assert.Equal(delivered + 1, a.Count);
        Assert.Null(a[^1].Schema);
        Assert.Equal(revision, h.Hub.Revision);
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));
        Assert.Equal(40, DriveTemperature(a, 0));

        // The next one is round 2 throughout: the unknown state arrives together with the absent value,
        // the new disk together with its entry in the list and its value.
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.NotNull(a[^1].Schema);
        Assert.Equal(revision + 1, h.Hub.Revision);
        Assert.Equal([HddDrive("unknown"), WdcDrive("active")], a[^1].Schema!.Service.Drives);
        Assert.Null(DriveTemperature(a, 0));
        Assert.Equal(35, DriveTemperature(a, 1));
    }
}
