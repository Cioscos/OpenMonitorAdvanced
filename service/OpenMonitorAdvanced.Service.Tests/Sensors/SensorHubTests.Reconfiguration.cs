using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

public sealed partial class SensorHubTests
{
    // ---- Task 12: applying module and per-disk SMART requests on the owning threads ----

    private const string WdcTemp = "/hdd/1/temperature/0";

    /// <summary>The drive key of <see cref="Hdd"/> as <see cref="FakeDisks"/> describes it by default.</summary>
    private static readonly string HddKey = DriveKey.Compute("ST2000DM008-2FR102", "DESCRIPTOR-SERIAL")!;

    private static readonly string WdcKey = DriveKey.Compute("WDC WD40EFRX-68N32N0", "WD-WCC7K0000001")!;

    private static HardwareNode Wdc() => new(
        "/hdd/1",
        HardwareType.Storage,
        "WDC WD40EFRX-68N32N0",
        [new SensorNode(WdcTemp, SensorType.Temperature, "Temperature", 0)],
        [],
        new StorageInfo(1, null, null, "WD-WCC7K0000001", Rotational: true));

    private static DriveFacts WdcFacts() =>
        new(1, DriveAvailability.Present, "WDC WD40EFRX-68N32N0", "WD-WCC7K0000001", BusType: 0x0B, SeekPenalty: true);

    private static bool HasMemory(SchemaMessage schema) => schema.Devices.Any(d => d.Kind == "memory");

    private static bool HasStorage(SchemaMessage schema) => schema.Devices.Any(d => d.Kind == "storage");

    private static int ReconfigurationWarnings(Harness h) =>
        h.Log.Entries.Count(e => e.Level == LogLevel.Warning && e.Message.Contains("reconfiguration", StringComparison.OrdinalIgnoreCase));

    [Fact]
    public void ModulesDisabledBeforeTheFirstTickAreNeverOpened()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Ram());
        List<FeedUpdate> a = h.Subscribe(Requests.Of(1000, ServiceModules.Memory), out _);

        h.Hub.TickOnce();

        // Storage stays out of the open as well (D6): only the storage worker's gate enables it.
        Assert.Equal(ServiceModules.Cpu | ServiceModules.Motherboard | ServiceModules.Controller | ServiceModules.Psu, h.Tree.OpenedModules);
        Assert.Equal(0, h.Tree.Updates("/ram"));
        Assert.False(HasMemory(LatestSchema(a)));
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
        Assert.Equal(0, h.Tree.SetModulesCalls);
    }

    [Fact]
    public void DisablingAModuleDropsItsDevicesInTheSnapshotsRevision()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Ram());
        h.Tree.Values[CpuTotal] = 10;
        h.Tree.Values[RamUsed] = 2;
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        Assert.True(HasMemory(LatestSchema(a)));
        int ramUpdates = h.Tree.Updates("/ram");

        sub.Update(Requests.Of(1000, ServiceModules.Memory));
        h.Hub.RunDue();

        // The same update carries the new schema and a snapshot built with its bindings.
        FeedUpdate answer = a[^1];
        Assert.NotNull(answer.Schema);
        Assert.False(HasMemory(answer.Schema));
        Assert.Equal(answer.Schema.Sensors.Count, answer.Snapshot.Values.Count);
        Assert.Equal(10, ValueOf(a, a.Count - 1, "load", "total"));
        Assert.Equal("pending", answer.Schema.Service.Reconfiguration);
        Assert.Equal(2, h.Hub.Revision); // devices and service block change in one revision

        // From that very tick the group is no longer updated, although it is still loaded.
        Assert.Equal(ramUpdates, h.Tree.Updates("/ram"));
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal(ramUpdates, h.Tree.Updates("/ram"));
        Assert.Equal(0, h.Tree.SetModulesCalls);
    }

    [Fact]
    public void SettersWaitForTheStorageWorkerToPark()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Ram());
        h.Tree.Storage.Add(Hdd());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.WorkingRoundDue();
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));

        sub.Update(Requests.Of(1000, ServiceModules.Memory));
        h.Hub.RunDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal(0, h.Tree.SetModulesCalls); // the storage worker has not parked yet

        // The storage worker parks at the boundary of its loop, and stays parked even when a round is due.
        Assert.Equal(Timeout.InfiniteTimeSpan, h.WorkingRoundDue());
        Assert.True(h.Hub.IsStorageParked);
        h.Advance(30_000);
        Assert.Equal(Timeout.InfiniteTimeSpan, h.WorkingRoundDue());
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
        Assert.Equal(0, h.Tree.SetModulesCalls);

        h.Hub.RunDue();
        (ServiceModules modules, _) = Assert.Single(h.Tree.SetModulesLog);
        Assert.Equal(ServiceModules.Cpu | ServiceModules.Motherboard | ServiceModules.Controller | ServiceModules.Psu, modules);
        Assert.False(h.Hub.IsStorageParked);

        h.WorkingRoundDue(); // released: the due round runs
        Assert.Equal(2, h.Tree.Updates("/hdd/0"));

        h.Advance(1000);
        h.Hub.RunDue();
        ServiceStateBlock state = LatestSchema(a).Service;
        Assert.Equal("applied", state.Reconfiguration);
        Assert.Equal(["cpu", "motherboard", "storage", "controller", "psu"], state.ActiveModules);
    }

    [Fact]
    public void ABlockedStorageWorkerLeadsToFailedWithoutApplying()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Ram());
        h.Tree.Storage.Add(Hdd());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();

        CancellationToken ct = TestContext.Current.CancellationToken;
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        h.Tree.BeforeUpdate = root =>
        {
            if (root.Type == HardwareType.Storage)
            {
                entered.Set();
                release.Wait(TimeSpan.FromSeconds(30), ct);
            }
        };
        var storage = new Thread(() => h.Hub.RunStorageDue()) { IsBackground = true, Name = "test-storage" };
        storage.Start();
        Assert.True(entered.Wait(TimeSpan.FromSeconds(10), ct), "the storage round never reached the disk");

        sub.Update(Requests.Of(1000, ServiceModules.Memory));
        h.Hub.RunDue();
        Assert.Equal("pending", LatestSchema(a).Service.Reconfiguration);
        int cpuUpdates = h.Tree.Updates("/amdcpu/0");
        for (int i = 0; i < 20; i++)
        {
            h.Advance(1000);
            h.Hub.RunDue();
        }

        // Past the timeout: failed, one warning, CPU sampling going on, no setter under the busy worker.
        Assert.Equal("failed", LatestSchema(a).Service.Reconfiguration);
        Assert.Equal(1, ReconfigurationWarnings(h));
        Assert.Equal(cpuUpdates + 20, h.Tree.Updates("/amdcpu/0"));
        Assert.Equal(0, h.Tree.SetModulesCalls);
        Assert.False(HasMemory(LatestSchema(a))); // the schema already reflects the request

        release.Set();
        Assert.True(storage.Join(TimeSpan.FromSeconds(10)));
        h.Tree.BeforeUpdate = null;
        h.Hub.RunStorageDue(); // the next loop boundary: parks
        h.Hub.RunDue();
        Assert.Equal(1, h.Tree.SetModulesCalls);
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
        Assert.Equal(1, ReconfigurationWarnings(h));
    }

    [Fact]
    public void AThrowingSetterIsRetriedOnlyAfterTheFailureDelay()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Ram());
        h.Tree.Storage.Add(Hdd());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        h.Tree.FailNextSetModules(1);

        sub.Update(Requests.Of(1000, ServiceModules.Memory));
        h.Hub.RunDue(); // asks for the park
        h.Hub.RunStorageDue(); // parks
        h.Hub.RunDue(); // the setter throws: the worker is released anyway
        Assert.Equal(1, h.Tree.SetModulesCalls);
        Assert.False(h.Hub.IsStorageParked);

        // For 30 s no new park and no setter (a throwing setter can cost seconds per call), while
        // the subscriber keeps getting its samples with the request marked failed.
        int updates = a.Count;
        for (int second = 1; second < 30; second++)
        {
            h.Advance(1000);
            h.Hub.RunDue();
            h.Hub.RunStorageDue();
            Assert.False(h.Hub.IsStorageParked);
            Assert.Equal(1, h.Tree.SetModulesCalls);
            Assert.Equal("failed", LatestSchema(a).Service.Reconfiguration);
        }

        Assert.Equal(updates + 29, a.Count);
        Assert.Equal(1, ReconfigurationWarnings(h));

        // Then it is retried with a new park, and this time it works.
        h.Advance(1000);
        h.Hub.RunDue(); // asks for the park
        h.Hub.RunStorageDue(); // parks
        h.Hub.RunDue();
        Assert.Equal(2, h.Tree.SetModulesCalls);
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
        Assert.Equal(1, ReconfigurationWarnings(h));
    }

    [Fact]
    public void ANewRequestIsTriedAtOnceDespiteAThrowingSetterBefore()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Ram());
        h.Tree.Storage.Add(Hdd());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        h.Tree.FailNextSetModules(1);

        sub.Update(Requests.Of(1000, ServiceModules.Memory));
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Hub.RunDue(); // throws
        Assert.Equal(1, h.Tree.SetModulesCalls);

        // The delay belongs to the request that failed: another one is not held back by it.
        h.Advance(1000);
        sub.Update(Requests.Of(1000, ServiceModules.Memory | ServiceModules.Psu));
        h.Hub.RunDue(); // asks for the park
        h.Hub.RunStorageDue(); // parks
        h.Hub.RunDue();
        Assert.Equal(2, h.Tree.SetModulesCalls);
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
    }

    [Fact]
    public void APersistentlyFailingRebuildStillAnswersEveryRequest()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Ram());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Tree.ThrowOnRoots = true;

        sub.Update(Requests.Of(1000, ServiceModules.Memory));
        h.Hub.RunDue();

        // The requester is answered with the current state, not starved waiting for a rebuild.
        FeedUpdate answer = a[^1];
        Assert.NotNull(answer.Schema);
        Assert.Equal("failed", answer.Schema.Service.Reconfiguration);
        Assert.Equal(answer.Schema.Sensors.Count, answer.Snapshot.Values.Count);

        // A client that subscribes meanwhile gets its schema too.
        List<FeedUpdate> b = h.Subscribe(Requests.Of(1000, ServiceModules.Memory), out _);
        h.Hub.RunDue();
        Assert.Equal("failed", Assert.Single(b).Schema?.Service.Reconfiguration);

        // Retried on every tick; the rebuild error is rate-limited, not one warning per tick.
        for (int i = 0; i < 3; i++)
        {
            h.Advance(1000);
            h.Hub.RunDue();
        }

        Assert.Single(h.Log.Entries, e => e.Level == LogLevel.Warning && e.Exception is InvalidOperationException { Message: "roots unavailable" });

        h.Tree.ThrowOnRoots = false;
        h.Advance(1000);
        h.Hub.RunDue();
        SchemaMessage rebuilt = LatestSchema(a);
        Assert.False(HasMemory(rebuilt));
        Assert.Equal("pending", rebuilt.Service.Reconfiguration); // the group waits for the storage worker's park
    }

    [Fact]
    public void DisablingStorageClearsItsCacheAndStopsDiskIo()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal(40, ValueOf(a, a.Count - 1, "temperature", "drive"));
        var before = (h.Tree.Updates("/hdd/0"), h.Tree.Reads(HddTemp), h.Disks.SpunDownQueries, h.Disks.DescribeCalls, h.Disks.EnumerateCalls, h.Activity.Reads, h.Activity.PowerQueries);

        sub.Update(Requests.Of(1000, ServiceModules.Storage));
        h.Hub.RunDue();
        Assert.False(HasStorage(LatestSchema(a))); // at once, before the storage worker ran
        for (int round = 0; round < 3; round++)
        {
            h.Hub.RunStorageDue();
            h.Advance(30_000);
            h.Hub.RunDue();
        }

        Assert.Equal(before, (h.Tree.Updates("/hdd/0"), h.Tree.Reads(HddTemp), h.Disks.SpunDownQueries, h.Disks.DescribeCalls, h.Disks.EnumerateCalls, h.Activity.Reads, h.Activity.PowerQueries));
        ServiceStateBlock state = LatestSchema(a).Service;
        Assert.False(HasStorage(LatestSchema(a)));
        Assert.Equal("applied", state.Reconfiguration);
        Assert.DoesNotContain("storage", state.ActiveModules);
        Assert.Equal(0, h.Tree.SetModulesCalls); // soft: the LHM group stays open
    }

    [Fact]
    public void AStorageRequestIsAppliedOnlyOnceTheStorageWorkerTookIt()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();

        sub.Update(Requests.Of(1000, ServiceModules.None, HddKey));
        h.Hub.RunDue();
        h.Advance(1000);
        h.Hub.RunDue();
        ServiceStateBlock waiting = LatestSchema(a).Service;
        Assert.Equal("pending", waiting.Reconfiguration);
        Assert.Empty(waiting.SmartDisabledDrives); // what the storage worker applies, not what was asked

        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
        Assert.Equal([HddKey], LatestSchema(a).Service.SmartDisabledDrives);
    }

    private const string NvmeId = "/nvme/2";
    private const string NvmeTemp = "/nvme/2/temperature/0";

    private static readonly string NvmeKey = DriveKey.Compute("Fanxiang S880 2TB", "NVME-SERIAL")!;

    private static HardwareNode Nvme() => new(
        NvmeId,
        HardwareType.Storage,
        "Fanxiang S880 2TB",
        [new SensorNode(NvmeTemp, SensorType.Temperature, "Composite Temperature", 0)],
        [],
        new StorageInfo(2, null, null, "NVME-SERIAL", Rotational: true, IsNvme: true, HasCriticalWarning: true));

    /// <summary>A harness with the NVMe disk resolved (bus type NVMe, no seek penalty) and one subscriber.</summary>
    private static Harness NvmeHarness(out List<FeedUpdate> updates, out IFeedSubscription subscription)
    {
        var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Nvme());
        h.Tree.Values[NvmeTemp] = 41;
        h.Disks.Facts[2] = new DriveFacts(2, DriveAvailability.Present, "Fanxiang S880 2TB", "NVME-SERIAL", BusType: DriveFacts.BusTypeNvme, SeekPenalty: false);
        updates = h.Subscribe(1000, out subscription);
        return h;
    }

    [Theory]
    [InlineData((byte)0x00, 0)]
    [InlineData((byte)0x02, 0)] // the transient temperature bit alone
    [InlineData((byte)0xC0, 0)] // reserved bits
    [InlineData((byte)0x01, 1)]
    [InlineData((byte)0x04, 1)]
    [InlineData((byte)0x06, 1)] // reliability degraded, with the temperature bit
    [InlineData((byte)0x3D, 1)]
    public void CriticalWarningMasksTheTemperatureBit(byte raw, int expected)
    {
        using Harness h = NvmeHarness(out List<FeedUpdate> a, out _);
        h.Tree.NvmeCriticalWarnings[NvmeId] = raw;
        h.Hub.TickOnce();
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(expected, ValueOf(a, a.Count - 1, "flag", "critical-warning"));
        Assert.Equal(41, ValueOf(a, a.Count - 1, "temperature", "drive"));
    }

    [Fact]
    public void CriticalWarningIsAbsentWhenTheAttributeCannotBeRead()
    {
        using Harness h = NvmeHarness(out List<FeedUpdate> a, out _);
        h.Hub.TickOnce();
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Null(ValueOf(a, a.Count - 1, "flag", "critical-warning"));
    }

    [Fact]
    public void CriticalWarningIsReadOnlyAfterTheDiskUpdate()
    {
        using Harness h = NvmeHarness(out _, out _);
        int readsAtUpdate = -1;
        h.Tree.BeforeUpdate = root =>
        {
            if (root.Identifier == NvmeId)
            {
                readsAtUpdate = h.Tree.CriticalWarningReads(NvmeId);
            }
        };
        h.Tree.NvmeCriticalWarnings[NvmeId] = 0x04;
        h.Hub.TickOnce();
        h.WorkingRound();

        Assert.Equal(0, readsAtUpdate); // the attribute is read after the update, never before
        Assert.Equal(1, h.Tree.Updates(NvmeId));
        Assert.Equal(1, h.Tree.CriticalWarningReads(NvmeId));
        h.Advance(1000);
        h.Hub.TickOnce(); // the sampler reads the cached value, not the tree
        Assert.Equal(1, h.Tree.CriticalWarningReads(NvmeId));
    }

    [Fact]
    public void CriticalWarningFollowsTheSmartSwitch()
    {
        using Harness h = NvmeHarness(out List<FeedUpdate> a, out IFeedSubscription sub);
        h.Tree.NvmeCriticalWarnings[NvmeId] = 0x04;
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal(1, ValueOf(a, a.Count - 1, "flag", "critical-warning"));

        sub.Update(Requests.Of(1000, ServiceModules.None, NvmeKey));
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.DoesNotContain(LatestSchema(a).Sensors, s => s.Kind == "flag");
        int reads = h.Tree.CriticalWarningReads(NvmeId);

        h.Advance(30_000);
        h.Hub.RunStorageDue();
        Assert.Equal(reads, h.Tree.CriticalWarningReads(NvmeId)); // SMART off: no update, no read

        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal(1, ValueOf(a, a.Count - 1, "flag", "critical-warning"));
    }

    [Fact]
    public void CriticalWarningAppearsOnceTheFirstUpdateExposesTheAttribute()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        HardwareNode before = Nvme() with { Storage = Nvme().Storage! with { HasCriticalWarning = false } };
        h.Tree.Storage.Add(before);
        h.Tree.Values[NvmeTemp] = 41;
        h.Disks.Facts[2] = new DriveFacts(2, DriveAvailability.Present, "Fanxiang S880 2TB", "NVME-SERIAL", BusType: DriveFacts.BusTypeNvme, SeekPenalty: false);
        // As LhmTree does: the first Update() rebuilds the node, now with the attribute.
        h.Tree.BeforeUpdate = _ =>
        {
            h.Tree.NvmeCriticalWarnings[NvmeId] = 0x04;
            h.Tree.Replace(h.Tree.Roots.Select(r => r.Identifier == NvmeId ? Nvme() : r).ToArray());
        };
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound();
        Assert.DoesNotContain(LatestSchema(a).Sensors, s => s.Kind == "flag");

        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal(1, ValueOf(a, a.Count - 1, "flag", "critical-warning"));
    }

    [Fact]
    public void ReEnablingStorageDoesNotReloadTheGroup()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        sub.Update(Requests.Of(1000, ServiceModules.Storage));
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        int powerChecks = h.Disks.SpunDownQueries;

        h.Advance(1000);
        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();
        Assert.False(HasStorage(LatestSchema(a))); // the resolved disks went with the cache

        h.Hub.RunStorageDue(); // a round at once, without a new gate or a new group
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal(powerChecks + 1, h.Disks.SpunDownQueries);
        Assert.Equal(2, h.Tree.Updates("/hdd/0"));

        h.Advance(1000);
        h.Hub.RunDue();
        Assert.True(HasStorage(LatestSchema(a)));
        Assert.Equal(40, ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
    }

    [Fact]
    public void StorageEnabledForTheFirstTimeLaterStillGoesThroughTheD6Gate()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.SpunDown[0] = true;
        h.Subscribe(Requests.Of(1000, ServiceModules.Storage), out IFeedSubscription sub);
        h.Hub.TickOnce();
        for (int round = 0; round < 3; round++)
        {
            h.WorkingRoundDue();
            h.Advance(30_000);
        }

        Assert.Equal((0, 0, 0, 0, 0, 0), (h.Disks.EnumerateCalls, h.Disks.SpunDownQueries, h.Disks.DescribeCalls, h.Tree.EnableStorageCount, h.Activity.Reads, h.Activity.PowerQueries));

        sub.Update(Requests.Of(1000));
        h.WorkingRoundDue();
        Assert.Equal((1, 1), (h.Disks.EnumerateCalls, h.Disks.SpunDownQueries));
        Assert.Equal(0, h.Tree.EnableStorageCount);

        h.Disks.SpunDown[0] = false;
        h.Advance(30_000);
        h.WorkingRoundDue();
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void DisposeReleasesAParkedStorageWorker()
    {
        var tree = new FakeTree();
        tree.Initial.Add(Cpu());
        tree.Initial.Add(Ram());
        tree.Storage.Add(Hdd());
        var log = new ListLogger<SensorHub>();
        var hub = new SensorHub(tree, new FakeDisks(), new FakeActivity(), () => true, new FakeTimeProvider(), log);
        CancellationToken ct = TestContext.Current.CancellationToken;
        using var storageUpdated = new ManualResetEventSlim();
        using var disposed = new ManualResetEventSlim();
        tree.BeforeUpdate = root =>
        {
            if (root.Type == HardwareType.Storage)
            {
                storageUpdated.Set();
            }
        };
        bool parkedAtDispose = false;
        string? setterThread = null;
        tree.BeforeSetModules = _ =>
        {
            // The storage worker acknowledged the park and waits for its release: stop the hub now.
            setterThread = Thread.CurrentThread.Name;
            parkedAtDispose = hub.IsStorageParked;
            hub.Dispose();
            disposed.Set();
        };

        IFeedSubscription sub = hub.Subscribe(Requests.Of(1000), _ => { });
        Assert.True(storageUpdated.Wait(TimeSpan.FromSeconds(10), ct), "the storage thread never ran its first round");
        sub.Update(Requests.Of(1000, ServiceModules.Memory));

        Assert.True(disposed.Wait(TimeSpan.FromSeconds(30), ct), "the setter never ran");
        Assert.Equal("oma-sampler", setterThread);
        Assert.True(parkedAtDispose);
        Assert.Equal(1, tree.CloseCount); // the parked worker stopped at once, so the tree could be closed
        Assert.DoesNotContain(log.Entries, e => e.Message.Contains("did not stop", StringComparison.Ordinal));
        Assert.Equal(0, tree.UpdatesAfterClose);
    }

    [Fact]
    public void ASmartDisabledDiskIsNeitherPowerCheckedNorUpdated()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        h.Disks.Facts[1] = WdcFacts();
        h.Subscribe(Requests.Of(1000, ServiceModules.None, HddKey), out _);
        h.Hub.TickOnce();

        for (int round = 1; round <= 2; round++)
        {
            h.WorkingRoundDue();
            h.Advance(30_000);
            Assert.Equal(1, h.Disks.SpunDownQueriesOf(0)); // the gate's check before the first identification, and never again
            Assert.Equal(0, h.Tree.Updates("/hdd/0"));
            Assert.Equal(round, h.Disks.SpunDownQueriesOf(1));
            Assert.Equal(round, h.Tree.Updates("/hdd/1"));
        }

        // Described once (access 0, never wakes it) to learn its key, then remembered.
        Assert.Equal(1, h.Disks.DescribeCallsOf(0));
    }

    [Fact]
    public void ASmartDisabledDiskLeavesTheSchemaAndReturnsWithItsId()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        h.Disks.Facts[1] = WdcFacts();
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        string hddId = Assert.Single(LatestSchema(a).Devices, d => d.Hint is StorageHint { PhysicalDrive: 0 }).Id;
        int revision = h.Hub.Revision;

        sub.Update(Requests.Of(1000, ServiceModules.None, HddKey));
        h.Hub.RunDue();
        SchemaMessage without = LatestSchema(a);
        Assert.DoesNotContain(without.Devices, d => d.Hint is StorageHint { PhysicalDrive: 0 });
        Assert.Contains(without.Devices, d => d.Hint is StorageHint { PhysicalDrive: 1 });
        Assert.Equal(revision + 1, h.Hub.Revision);

        // Applied once the storage worker took the request.
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal([HddKey], LatestSchema(a).Service.SmartDisabledDrives);
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);

        h.Advance(1000);
        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();
        Assert.Equal(hddId, Assert.Single(LatestSchema(a).Devices, d => d.Hint is StorageHint { PhysicalDrive: 0 }).Id);
    }

    [Fact]
    public void ASmartDisabledDiskStillHoldsTheD6Gate()
    {
        // Verdict (c): LHM cannot leave one disk out of the discovery, so switching its SMART off
        // does not take it out of the gate either.
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.SpunDown[0] = true;
        h.Activity.Counters[0] = new DiskCounters(100, 100); // nothing works on it
        List<FeedUpdate> a = h.Subscribe(Requests.Of(1000, ServiceModules.None, HddKey), out _);
        h.Hub.TickOnce();
        for (int round = 0; round < 2; round++)
        {
            h.Hub.RunStorageDue();
            h.Advance(30_000);
        }

        h.Hub.RunDue();
        Assert.Equal(2, h.Disks.EnumerateCalls);
        Assert.Equal(1, h.Disks.SpunDownQueriesOf(0)); // asked by the first gate round only: nothing shows that it works
        Assert.Equal(0, h.Tree.EnableStorageCount);
        Assert.Equal([new WireDrive(0, HddKey, "ST2000DM008-2FR102", "smartOff", true)], Drives(a));
    }
}
