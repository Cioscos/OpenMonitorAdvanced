using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// Drives <see cref="SensorHub"/> deterministically: workers are never started
/// (<c>StartWorkers = false</c>), time is a <see cref="FakeTimeProvider"/>, and each test calls
/// <c>TickOnce</c> (one sampling tick), <c>RunDue</c> (one wake of the sampler loop),
/// <c>StorageOnce</c> (one storage round) or <c>RunStorageDue</c> (one wake of the storage loop)
/// itself. The fake tree only proves the hub's own rules; that the real LHM open does not wake a
/// sleeping disk is the Task 15 hardware gate.
/// </summary>
public sealed class SensorHubTests
{
    private const string CpuTotal = "/amdcpu/0/load/0";
    private const string CpuTctl = "/amdcpu/0/temperature/2";
    private const string CpuCoreMax = "/amdcpu/0/load/1";
    private const string RamUsed = "/ram/data/0";
    private const string HddTemp = "/hdd/0/temperature/0";
    private const double TwoPow30 = 1024d * 1024d * 1024d;

    private static HardwareNode Cpu(params SensorNode[] extra) => new(
        "/amdcpu/0",
        HardwareType.Cpu,
        "AMD Ryzen 7 7800X3D",
        [
            new SensorNode(CpuTotal, SensorType.Load, "CPU Total", 0),
            new SensorNode(CpuTctl, SensorType.Temperature, "Core (Tctl/Tdie)", 2),
            .. extra,
        ],
        []);

    private static HardwareNode Ram() =>
        new("/ram", HardwareType.Memory, "Total Memory", [new SensorNode(RamUsed, SensorType.Data, "Memory Used", 0)], []);

    private static HardwareNode Hdd() => new(
        "/hdd/0",
        HardwareType.Storage,
        "ST2000DM008-2FR102",
        [new SensorNode(HddTemp, SensorType.Temperature, "Temperature", 0)],
        [],
        new StorageInfo(0, "ST2000DM008-2FR102", "SERIAL", "SERIAL", Rotational: true));

    private sealed class Harness : IDisposable
    {
        public Harness(IDiskPowerProbe? disks = null)
        {
            Hub = new SensorHub(Tree, disks ?? Disks, () => true, Time, Log) { StartWorkers = false };
        }

        public FakeTree Tree { get; } = new();

        public FakeDisks Disks { get; } = new();

        public FakeTimeProvider Time { get; } = new(DateTimeOffset.FromUnixTimeMilliseconds(1_790_000_000_000));

        public ListLogger<SensorHub> Log { get; } = new();

        public SensorHub Hub { get; }

        public List<FeedUpdate> Subscribe(uint intervalMs) => Subscribe(Requests.Of(intervalMs), out _);

        public List<FeedUpdate> Subscribe(uint intervalMs, out IFeedSubscription subscription) => Subscribe(Requests.Of(intervalMs), out subscription);

        public List<FeedUpdate> Subscribe(FeedRequest request, out IFeedSubscription subscription)
        {
            var received = new List<FeedUpdate>();
            subscription = Hub.Subscribe(request, u =>
            {
                lock (received)
                {
                    received.Add(u);
                }
            });
            return received;
        }

        public void Advance(int ms) => Time.Advance(TimeSpan.FromMilliseconds(ms));

        public void Dispose() => Hub.Dispose();
    }

    /// <summary>The value of the (kind, name) sensor in <paramref name="update"/>, resolved against the latest schema the subscriber saw.</summary>
    private static double? ValueOf(IReadOnlyList<FeedUpdate> updates, int at, string kind, string name)
    {
        SchemaMessage? schema = null;
        for (int i = 0; i <= at; i++)
        {
            schema = updates[i].Schema ?? schema;
        }

        Assert.NotNull(schema);
        int index = schema.Sensors.ToList().FindIndex(s => s.Kind == kind && s.Name == name);
        Assert.True(index >= 0, $"{kind}/{name} is not in the schema");
        return updates[at].Snapshot.Values[index];
    }

    private static SchemaMessage LatestSchema(IReadOnlyList<FeedUpdate> updates) =>
        updates.Last(u => u.Schema is not null).Schema!;

    [Fact]
    public void FirstSubscriberOpensTheTreeOnce()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Subscribe(1000);
        h.Subscribe(1000);
        Assert.Equal(0, h.Tree.OpenCount);

        h.Hub.TickOnce();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(1, h.Tree.OpenCount);
    }

    [Fact]
    public void TheFirstUpdateOfEachSubscriberCarriesTheSchema()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.Advance(1000);
        List<FeedUpdate> b = h.Subscribe(1000);
        h.Hub.TickOnce();

        Assert.Equal(2, a.Count);
        Assert.NotNull(a[0].Schema);
        Assert.Null(a[1].Schema);
        FeedUpdate first = Assert.Single(b);
        Assert.NotNull(first.Schema);
        Assert.Equal(first.Schema.Sensors.Count, first.Snapshot.Values.Count);
    }

    [Fact]
    public void EachSubscriberGetsItsOwnInterval()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        List<FeedUpdate> a = h.Subscribe(1000);
        List<FeedUpdate> b = h.Subscribe(2000);

        for (int i = 0; i < 4; i++)
        {
            h.Advance(1000);
            h.Hub.TickOnce();
        }

        Assert.Equal(4, a.Count);
        Assert.Equal(2, b.Count);
    }

    [Fact]
    public void NonMultipleIntervalsAreNotSentEarly()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        long start = h.Time.GetUtcNow().ToUnixTimeMilliseconds();
        var aTimes = new List<long>();
        var bTimes = new List<long>();
        h.Hub.Subscribe(1000, _ => aTimes.Add(h.Time.GetUtcNow().ToUnixTimeMilliseconds() - start));
        h.Hub.Subscribe(1500, _ => bTimes.Add(h.Time.GetUtcNow().ToUnixTimeMilliseconds() - start));

        for (int guard = 0; h.Time.GetUtcNow().ToUnixTimeMilliseconds() - start <= 9000; guard++)
        {
            Assert.True(guard < 100, "the sampler loop did not make progress");
            TimeSpan delay = h.Hub.RunDue();
            Assert.True(delay > TimeSpan.Zero && delay != Timeout.InfiniteTimeSpan, $"unexpected delay {delay}");
            h.Time.Advance(delay);
        }

        Assert.Equal([0L, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000], aTimes);
        Assert.Equal([0L, 1500, 3000, 4500, 6000, 7500, 9000], bTimes);

        // B's deliveries at 1500/4500/7500 reuse the latest sample: LHM ran once per second only.
        Assert.Equal(10, h.Tree.Updates("/amdcpu/0"));
    }

    [Fact]
    public void SamplingContinuesWhileAnyClientRemains()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription subA);
        List<FeedUpdate> b = h.Subscribe(1000, out IFeedSubscription subB);

        h.Hub.TickOnce();
        subA.Dispose();
        h.Advance(1000);
        Assert.Equal(TimeSpan.FromSeconds(1), h.Hub.RunDue());
        h.Advance(1000);
        h.Hub.RunDue();

        Assert.Single(a);
        Assert.Equal(3, b.Count);
        Assert.Equal(3, h.Tree.Updates("/amdcpu/0"));

        subB.Dispose();
        h.Advance(1000);
        Assert.Equal(Timeout.InfiniteTimeSpan, h.Hub.RunDue());
        Assert.Equal(3, h.Tree.Updates("/amdcpu/0"));
    }

    [Fact]
    public void TheTreeIsClosedOnlyOnDispose()
    {
        var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        sub.Dispose();
        h.Advance(5000);
        h.Hub.RunDue();
        h.Hub.RunStorageDue();

        Assert.Equal(0, h.Tree.CloseCount);

        h.Hub.Dispose();
        h.Hub.Dispose();

        Assert.Equal(1, h.Tree.CloseCount);
    }

    [Fact]
    public void StorageIsUpdatedEveryThirtySeconds()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Subscribe(1000);
        h.Hub.TickOnce();

        for (int s = 0; s <= 61; s++)
        {
            h.Hub.RunStorageDue();
            h.Advance(1000);
        }

        Assert.Equal(3, h.Tree.Updates("/hdd/0"));
        Assert.Equal(1, h.Tree.EnableStorageCount);
    }

    [Fact]
    public void ASpunDownHddIsSkippedAndReadsAsMissing()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.Hub.StorageOnce();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal(40, ValueOf(a, a.Count - 1, "temperature", "drive"));

        h.Disks.SpunDown[0] = true;
        h.Advance(30_000);
        h.Hub.StorageOnce();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Null(ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void UnknownPowerStateSkipsHdd()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        h.Disks.SpunDown[0] = null;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.Hub.StorageOnce();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(0, h.Tree.Updates("/hdd/0"));
        Assert.Equal(0, h.Tree.Reads(HddTemp));
        Assert.Null(ValueOf(a, a.Count - 1, "temperature", "drive"));
    }

    [Fact]
    public void NoUnconditionalStorageReadDuringOpen()
    {
        using var h = new Harness();
        // Even a tree that exposed a disk straight from Open() must not have it touched until
        // the D6 gate opens: the sampler never updates or reads storage hardware.
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        h.Disks.AllActive = false;
        h.Subscribe(1000);

        for (int i = 0; i < 3; i++)
        {
            h.Hub.TickOnce();
            h.Hub.StorageOnce();
            h.Advance(1000);
        }

        Assert.Equal(1, h.Tree.OpenCount);
        Assert.Equal(0, h.Tree.EnableStorageCount);
        Assert.Equal(0, h.Tree.Updates("/hdd/0"));
        Assert.Equal(0, h.Tree.Reads(HddTemp));
        Assert.Equal(0, h.Disks.SpunDownQueries);
    }

    [Fact]
    public void StorageIsEnabledOnlyWhenEveryRotationalDiskIsActive()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.AllActive = false;
        h.Subscribe(1000);
        h.Hub.TickOnce();

        h.Hub.StorageOnce();
        Assert.Equal(0, h.Tree.EnableStorageCount);
        Assert.Equal(0, h.Tree.Updates("/hdd/0"));

        h.Disks.AllActive = true;
        h.Advance(30_000);
        h.Hub.StorageOnce();
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));

        // Once enabled it stays enabled; the per-disk check takes over.
        h.Disks.AllActive = false;
        h.Advance(30_000);
        h.Hub.StorageOnce();
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal(2, h.Disks.AllActiveQueries);
        Assert.Equal(2, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void UnknownRotationalStateKeepsStorageDisabled()
    {
        bool? spunDown = null;
        var probe = new DiskPowerProbe(
            enumerateDrives: () => [new DriveFacts(0, DriveAvailability.Present, "Disk", null, BusType: 0x0B, SeekPenalty: null)], // unknown => treated as rotational
            isSpunDown: _ => spunDown);
        using var h = new Harness(probe);
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Subscribe(1000);
        h.Hub.TickOnce();

        h.Hub.StorageOnce();
        h.Advance(30_000);
        h.Hub.StorageOnce();
        Assert.Equal(0, h.Tree.EnableStorageCount);

        spunDown = false;
        h.Advance(30_000);
        h.Hub.StorageOnce();
        Assert.Equal(1, h.Tree.EnableStorageCount);
    }

    [Fact]
    public void StorageStopsWithNoSubscribers()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));

        sub.Dispose();
        for (int i = 0; i < 4; i++)
        {
            h.Advance(30_000);
            Assert.Equal(Timeout.InfiniteTimeSpan, h.Hub.RunStorageDue());
        }

        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void SlowStorageDoesNotBlockCpuSampling()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        List<FeedUpdate> a = h.Subscribe(1000);
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

        var storage = new Thread(() => h.Hub.StorageOnce()) { IsBackground = true, Name = "test-storage" };
        storage.Start();
        Assert.True(entered.Wait(TimeSpan.FromSeconds(10), ct), "the storage round never reached the disk");

        var ticks = new Thread(() =>
        {
            for (int i = 0; i < 3; i++)
            {
                h.Advance(1000);
                h.Hub.TickOnce();
            }
        })
        { IsBackground = true, Name = "test-sampler" };
        ticks.Start();
        bool finished = ticks.Join(TimeSpan.FromSeconds(10));
        release.Set();

        Assert.True(finished, "CPU sampling was blocked by a slow disk");
        Assert.Equal(4, a.Count);
        Assert.Equal(4, h.Tree.Updates("/amdcpu/0"));
        Assert.True(storage.Join(TimeSpan.FromSeconds(10)));
    }

    [Fact]
    public void SchemaAndBindingsChangeAtomically()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Values[CpuTotal] = 10;
        h.Tree.Values[RamUsed] = 2;
        List<FeedUpdate> a = h.Subscribe(1000);

        h.Hub.TickOnce();
        h.Tree.Replace(Ram(), Cpu());
        h.Advance(1000);
        h.Hub.TickOnce();
        h.Tree.Replace(Cpu());
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(3, a.Count(u => u.Schema is not null));
        SchemaMessage? current = null;
        foreach (FeedUpdate u in a)
        {
            current = u.Schema ?? current;
            Assert.NotNull(current);
            Assert.Equal(current.Sensors.Count, u.Snapshot.Values.Count);
        }

        Assert.Equal(10, ValueOf(a, 1, "load", "total"));
        Assert.Equal(2 * TwoPow30, ValueOf(a, 1, "data", "used"));
        Assert.Equal(10, ValueOf(a, 2, "load", "total"));
    }

    [Fact]
    public void LateActivatedSensorAppearsInSchema()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Values[CpuCoreMax] = 55;
        bool activated = false;
        h.Tree.BeforeUpdate = root =>
        {
            if (!activated && root.Identifier == "/amdcpu/0")
            {
                activated = true;
                h.Tree.Replace(Cpu(new SensorNode(CpuCoreMax, SensorType.Load, "CPU Core Max", 1)));
            }
        };
        List<FeedUpdate> a = h.Subscribe(1000);

        h.Hub.TickOnce();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Contains(LatestSchema(a).Sensors, s => s.Kind == "load" && s.Name == "core-max");
        Assert.Equal(55, ValueOf(a, a.Count - 1, "load", "core-max"));
    }

    [Fact]
    public void HardwareAddedSendsANewSchema()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        List<FeedUpdate> a = h.Subscribe(1000);
        List<FeedUpdate> b = h.Subscribe(1000);
        h.Hub.TickOnce();
        Assert.Equal(1, h.Hub.Revision);

        h.Tree.Replace(Cpu(), Ram());
        h.Advance(1000);
        h.Hub.TickOnce();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(2, h.Hub.Revision);
        foreach (List<FeedUpdate> updates in new[] { a, b })
        {
            Assert.NotNull(updates[1].Schema);
            Assert.Contains(updates[1].Schema!.Sensors, s => s.Kind == "data" && s.Name == "used");
            Assert.Null(updates[2].Schema);
        }
    }

    [Fact]
    public void AnUnchangedStructureDoesNotResendTheSchema()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();

        // Equal content, new record and list instances: not a revision change.
        h.Tree.Replace(Cpu());
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(1, h.Hub.Revision);
        Assert.Null(a[1].Schema);
    }

    [Fact]
    public void AFailingHardwareBlanksOnlyItsOwnSensors()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Ram());
        h.Tree.Values[CpuTotal] = 10;
        h.Tree.Values[RamUsed] = 1;
        h.Tree.Failing["/amdcpu/0"] = true;
        List<FeedUpdate> a = h.Subscribe(1000);

        for (int i = 0; i < 3; i++)
        {
            h.Hub.TickOnce();
            h.Advance(1000);
        }

        Assert.Null(ValueOf(a, 2, "load", "total"));
        Assert.Equal(TwoPow30, ValueOf(a, 2, "data", "used"));
        Assert.Single(h.Log.Entries, e => e.Level >= LogLevel.Warning && e.Message.Contains("/amdcpu/0", StringComparison.Ordinal));

        h.Tree.Failing.Clear();
        h.Hub.TickOnce();
        Assert.Equal(10, ValueOf(a, 3, "load", "total"));

        // At most one line per minute per root.
        h.Tree.Failing["/amdcpu/0"] = true;
        h.Advance(61_000);
        h.Hub.TickOnce();
        Assert.Equal(2, h.Log.Entries.Count(e => e.Level >= LogLevel.Warning && e.Message.Contains("/amdcpu/0", StringComparison.Ordinal)));
    }

    [Fact]
    public void ValuesAreScaledAndNonFiniteBecomesNull()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Ram());
        h.Tree.Values[RamUsed] = 2.5;
        h.Tree.Values[CpuTotal] = double.NaN;
        h.Tree.Values[CpuTctl] = double.PositiveInfinity;
        List<FeedUpdate> a = h.Subscribe(1000);

        h.Hub.TickOnce();

        Assert.Equal(2.5 * TwoPow30, ValueOf(a, 0, "data", "used"));
        Assert.Null(ValueOf(a, 0, "load", "total"));
        Assert.Null(ValueOf(a, 0, "temperature", "tctl"));
    }

    [Fact]
    public void SeqRisesByOnePerTickAndTimestampIsTheTickTime()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        List<FeedUpdate> a = h.Subscribe(1000);

        h.Hub.TickOnce();
        long firstTick = h.Time.GetUtcNow().ToUnixTimeMilliseconds();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(a[0].Snapshot.Seq + 1, a[1].Snapshot.Seq);
        Assert.Equal((ulong)firstTick, a[0].Snapshot.TimestampMs);
        Assert.Equal((ulong)firstTick + 1000, a[1].Snapshot.TimestampMs);
    }

    [Fact]
    public void WorkersOpenOnTheSamplerThreadAndNeverUpdateAfterClose()
    {
        var tree = new FakeTree();
        tree.Initial.Add(Cpu());
        tree.Storage.Add(Hdd());
        var hub = new SensorHub(tree, new FakeDisks(), () => true, new FakeTimeProvider(), new ListLogger<SensorHub>());
        CancellationToken ct = TestContext.Current.CancellationToken;
        using var delivered = new ManualResetEventSlim();
        using var storageUpdated = new ManualResetEventSlim();
        tree.BeforeUpdate = root =>
        {
            if (root.Type == HardwareType.Storage)
            {
                storageUpdated.Set();
            }
        };

        hub.Subscribe(1000, _ => delivered.Set());

        Assert.True(delivered.Wait(TimeSpan.FromSeconds(10), ct), "the sampler thread never delivered");
        Assert.True(storageUpdated.Wait(TimeSpan.FromSeconds(10), ct), "the storage thread never ran its first round");
        hub.Dispose();

        Assert.Equal("oma-sampler", tree.OpenThreadName);
        Assert.Equal(1, tree.OpenCount);
        Assert.Equal(1, tree.CloseCount);
        Assert.Equal(0, tree.UpdatesAfterClose);
    }

    [Fact]
    public void SubscribeRacingDisposeIsSafe()
    {
        for (int round = 0; round < 50; round++)
        {
            var tree = new FakeTree();
            tree.Initial.Add(Cpu());
            var hub = new SensorHub(tree, new FakeDisks(), () => true, new FakeTimeProvider(), new ListLogger<SensorHub>());
            var errors = new System.Collections.Concurrent.ConcurrentQueue<Exception>();
            CancellationToken ct = TestContext.Current.CancellationToken;
            using var start = new Barrier(2);
            var subscriber = new Thread(() =>
            {
                start.SignalAndWait(ct);
                for (int i = 0; i < 20; i++)
                {
                    try
                    {
                        hub.Subscribe(1000, _ => { }).Dispose();
                        hub.Subscribe(1000, _ => { });
                    }
                    catch (Exception e)
                    {
                        errors.Enqueue(e);
                    }
                }
            })
            { IsBackground = true };
            subscriber.Start();
            start.SignalAndWait(ct);
            hub.Dispose();
            Assert.True(subscriber.Join(TimeSpan.FromSeconds(10)));

            // After Dispose a subscription is a no-op: no exception, no worker thread.
            hub.Subscribe(1000, _ => { }).Dispose();

            Assert.Empty(errors);
            Assert.False(hub.AnyWorkerAlive);
            Assert.True(tree.CloseCount <= 1);
            Assert.Equal(0, tree.UpdatesAfterClose);
        }
    }

    [Fact]
    public void DisposeGivesUpOnAStuckWorkerWithoutClosingTheTree()
    {
        var tree = new FakeTree();
        tree.Initial.Add(Cpu());
        tree.Storage.Add(Hdd());
        var log = new ListLogger<SensorHub>();
        var hub = new SensorHub(tree, new FakeDisks(), () => true, new FakeTimeProvider(), log) { WorkerJoinTimeout = TimeSpan.FromMilliseconds(200) };
        CancellationToken ct = TestContext.Current.CancellationToken;
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        tree.BeforeUpdate = root =>
        {
            if (root.Type == HardwareType.Storage)
            {
                entered.Set();
                release.Wait(TimeSpan.FromSeconds(30), ct);
            }
        };
        hub.Subscribe(1000, _ => { });
        Assert.True(entered.Wait(TimeSpan.FromSeconds(10), ct), "the storage worker never reached the disk");

        hub.Dispose();

        // A worker may still be inside the tree: it is left open (the process is exiting).
        Assert.Equal(0, tree.CloseCount);
        Assert.Contains(log.Entries, e => e.Level >= LogLevel.Warning && e.Message.Contains("oma-storage", StringComparison.Ordinal));
        release.Set();
    }

    [Fact]
    public void DiskIdentityIsResolvedOnlyByTheStorageWorker()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        List<FeedUpdate> a = h.Subscribe(1000);
        for (int i = 0; i < 3; i++)
        {
            h.Hub.TickOnce();
            h.Advance(1000);
        }

        Assert.Equal(0, h.Disks.DescribeCalls);
        Assert.DoesNotContain(LatestSchema(a).Devices, d => d.Kind == "storage");

        var storage = new Thread(() => h.Hub.StorageOnce()) { Name = "oma-storage" };
        storage.Start();
        Assert.True(storage.Join(TimeSpan.FromSeconds(10)));
        h.Hub.TickOnce();

        Assert.NotEmpty(h.Disks.DescribeThreads);
        Assert.All(h.Disks.DescribeThreads, name => Assert.Equal("oma-storage", name));
        WireDevice disk = Assert.Single(LatestSchema(a).Devices, d => d.Kind == "storage");
        Assert.Equal(new StorageHint(0, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL"), disk.Hint);
    }

    [Fact]
    public void ABlockingDiskIdentityQueryDoesNotBlockSampling()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();

        CancellationToken ct = TestContext.Current.CancellationToken;
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        h.Disks.BeforeDescribe = _ =>
        {
            entered.Set();
            release.Wait(TimeSpan.FromSeconds(30), ct);
        };

        var storage = new Thread(() => h.Hub.StorageOnce()) { IsBackground = true };
        storage.Start();
        Assert.True(entered.Wait(TimeSpan.FromSeconds(10), ct), "the storage round never asked for the disk identity");

        var ticks = new Thread(() =>
        {
            for (int i = 0; i < 3; i++)
            {
                h.Advance(1000);
                h.Hub.TickOnce();
            }
        })
        { IsBackground = true };
        ticks.Start();
        bool finished = ticks.Join(TimeSpan.FromSeconds(10));
        release.Set();

        Assert.True(finished, "CPU sampling was blocked by a disk identity query");
        Assert.Equal(4, a.Count);
        Assert.True(storage.Join(TimeSpan.FromSeconds(10)));
    }

    [Fact]
    public void OneSamplingAndOneWakePerIntervalDespiteSlowTicks()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        int ticks = 0;
        h.Tree.BeforeUpdate = _ => h.Advance(ticks++ % 2 == 0 ? 50 : 10); // LHM updates take 50 or 10 ms of fake time
        long start = h.Time.GetUtcNow().ToUnixTimeMilliseconds();
        List<FeedUpdate> a = h.Subscribe(1000);

        int wakes = 0;
        while (h.Time.GetUtcNow().ToUnixTimeMilliseconds() - start < 60_000)
        {
            Assert.True(wakes < 1000, "the sampler loop did not make progress");
            TimeSpan delay = h.Hub.RunDue();
            wakes++;
            h.Time.Advance(delay + TimeSpan.FromMilliseconds(3)); // the OS timer fires a little late
        }

        Assert.Equal(60, h.Tree.Updates("/amdcpu/0"));
        Assert.Equal(60, a.Count);
        Assert.Equal(60, wakes);
    }

    [Fact]
    public void ADiskSensorActivatedByItsUpdateIsReadInTheSameRound()
    {
        const string HddLife = "/hdd/0/level/20";
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddLife] = 97;
        bool activated = false;
        h.Tree.BeforeUpdate = root =>
        {
            if (!activated && root.Type == HardwareType.Storage)
            {
                activated = true;
                h.Tree.Replace(Cpu(), Hdd() with { Sensors = [.. Hdd().Sensors, new SensorNode(HddLife, SensorType.Level, "Life", 20)] });
            }
        };
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.Hub.StorageOnce();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(97, ValueOf(a, a.Count - 1, "percent", "life"));
    }

    [Fact]
    public void AFailedSchemaRebuildIsRetriedOnTheNextTick()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Values[RamUsed] = 1;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();

        h.Tree.Replace(Cpu(), Ram());
        h.Tree.ThrowOnNextRoots();
        h.Advance(1000);
        h.Hub.TickOnce(); // the rebuild fails: the old schema keeps being served
        h.Advance(1000);
        h.Hub.TickOnce(); // retried

        Assert.Equal(3, a.Count);
        Assert.Null(a[1].Schema);
        Assert.Equal(a[0].Schema!.Sensors.Count, a[1].Snapshot.Values.Count);
        Assert.NotNull(a[2].Schema);
        Assert.Equal(TwoPow30, ValueOf(a, 2, "data", "used"));
        Assert.Contains(h.Log.Entries, e => e.Level >= LogLevel.Warning && e.Exception is InvalidOperationException { Message: "roots unavailable" });
    }

    [Fact]
    public void StorageValuesDoNotSurviveAnIdlePeriod()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        h.Subscribe(1000, out IFeedSubscription first);
        h.Hub.TickOnce();
        h.Hub.StorageOnce();
        first.Dispose();

        h.Advance(5_000);
        List<FeedUpdate> b = h.Subscribe(1000);
        h.Hub.TickOnce(); // before any new storage round

        Assert.Null(ValueOf(b, 0, "temperature", "drive"));
    }

    [Fact]
    public void StorageValuesOlderThanTwoRoundsAreAbsent()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.Hub.StorageOnce();
        h.Advance(59_000);
        h.Hub.TickOnce();
        Assert.Equal(40, ValueOf(a, a.Count - 1, "temperature", "drive"));

        h.Advance(2_000); // the storage worker is stuck: no round for more than 60 s
        h.Hub.TickOnce();
        Assert.Null(ValueOf(a, a.Count - 1, "temperature", "drive"));
    }

    [Fact]
    public void ANoMediaAnswerIsRetriedAndNeverBypassesThePowerCheck()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        // A USB bridge answering NOT_READY while its disk sleeps.
        h.Disks.Facts[0] = new DriveFacts(0, DriveAvailability.NoMedia, null, null, null, null);
        h.Disks.SpunDown[0] = true;
        h.Subscribe(1000);
        h.Hub.TickOnce();

        h.Hub.StorageOnce();
        Assert.Equal(0, h.Tree.Updates("/hdd/0"));

        h.Disks.Facts[0] = new DriveFacts(0, DriveAvailability.Present, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL", BusType: 0x0B, SeekPenalty: true);
        h.Advance(30_000);
        h.Hub.StorageOnce();
        Assert.Equal(0, h.Tree.Updates("/hdd/0")); // re-described, and its standby is honoured
        Assert.Equal(2, h.Disks.DescribeCalls);

        h.Disks.SpunDown[0] = false;
        h.Advance(30_000);
        h.Hub.StorageOnce();
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
        Assert.Equal(2, h.Disks.DescribeCalls); // a complete description is cached
    }

    [Fact]
    public void AnIncompleteDescriptionIsRetriedAndThenUpdatesTheHint()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.Facts[0] = new DriveFacts(0, DriveAvailability.Present, null, null, BusType: null, SeekPenalty: true); // descriptor query failed
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.Hub.StorageOnce();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal(new StorageHint(0, null, null), Assert.Single(LatestSchema(a).Devices, d => d.Kind == "storage").Hint);

        h.Disks.Facts.TryRemove(0, out _);
        h.Advance(30_000);
        h.Hub.StorageOnce();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.NotNull(a[^1].Schema);
        Assert.Equal(new StorageHint(0, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL"), Assert.Single(LatestSchema(a).Devices, d => d.Kind == "storage").Hint);

        h.Advance(30_000);
        h.Hub.StorageOnce();
        Assert.Equal(2, h.Disks.DescribeCalls);
    }

    [Fact]
    public void AComeBackAfterIdleRunsAStorageRoundAtOnce()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Subscribe(1000, out IFeedSubscription first);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
        first.Dispose();

        h.Advance(5_000);
        h.Subscribe(1000);
        h.Hub.RunStorageDue();

        Assert.Equal(2, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void AnUndescribableDiskIsLoggedOnce()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.Facts[0] = null;
        h.Subscribe(1000);
        h.Hub.TickOnce();

        for (int i = 0; i < 3; i++)
        {
            h.Hub.StorageOnce();
            h.Advance(30_000);
        }

        Assert.Single(h.Log.Entries, e => e.Message.Contains("PhysicalDrive0", StringComparison.Ordinal) && e.Message.Contains("cannot be described", StringComparison.Ordinal));
        Assert.Equal(3, h.Disks.DescribeCalls);
    }

    [Fact]
    public void IdenticalDisksResolvedInDifferentRoundsKeepTheirPublishedIds()
    {
        var twin = new HardwareNode(
            "/hdd/1",
            HardwareType.Storage,
            "ST2000DM008-2FR102",
            [new SensorNode("/hdd/1/temperature/0", SensorType.Temperature, "Temperature", 0)],
            [],
            new StorageInfo(1, null, null, "SERIAL", Rotational: true)); // same IDENTIFY serial as Hdd()
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(twin);
        h.Disks.Facts[1] = null; // the twin cannot be described yet
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.Hub.StorageOnce();
        h.Advance(1000);
        h.Hub.TickOnce();
        string firstId = Assert.Single(LatestSchema(a).Devices, d => d.Kind == "storage").Id;

        h.Disks.Facts.TryRemove(1, out _);
        h.Advance(30_000);
        h.Hub.StorageOnce();
        h.Advance(1000);
        h.Hub.TickOnce();

        List<WireDevice> disks = LatestSchema(a).Devices.Where(d => d.Kind == "storage").ToList();
        Assert.Equal(2, disks.Count);
        Assert.Equal(firstId, Assert.Single(disks, d => d.Hint is StorageHint { PhysicalDrive: 0 }).Id);
        Assert.NotEqual(firstId, Assert.Single(disks, d => d.Hint is StorageHint { PhysicalDrive: 1 }).Id);
    }

    [Fact]
    public void StorageRootsSharingAnIdentifierAreSkippedAndLoggedOnce()
    {
        // Two disks LHM enumerates under the same identifier (e.g. two "/hdd/-1"; here with
        // drive numbers, so the hub would otherwise resolve both): neither is described, updated
        // or published, and the rest of the schema is unaffected.
        HardwareNode Clash(int drive) => new(
            "/hdd/7",
            HardwareType.Storage,
            "Clashing disk",
            [new SensorNode("/hdd/7/temperature/0", SensorType.Temperature, "Temperature", 0)],
            [],
            new StorageInfo(drive, null, null, null, Rotational: true));
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Clash(3));
        h.Tree.Storage.Add(Clash(4));
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        for (int round = 0; round < 3; round++)
        {
            h.Hub.StorageOnce();
            h.Advance(1000);
            h.Hub.TickOnce();
            h.Advance(30_000);
        }

        SchemaMessage schema = LatestSchema(a);
        SchemaBuilderTests.AssertWireInvariants(new BuiltSchema(schema, [.. schema.Sensors.Select(_ => new SensorBinding("x", 1))]));
        WireDevice disk = Assert.Single(schema.Devices, d => d.Kind == "storage");
        Assert.Equal(new StorageHint(0, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL"), disk.Hint);
        Assert.Contains(schema.Devices, d => d.Kind == "cpu");
        Assert.Equal(0, h.Tree.Updates("/hdd/7"));
        Assert.Single(h.Log.Entries, e => e.Level == LogLevel.Warning && e.Message.Contains("/hdd/7", StringComparison.Ordinal));
    }

    [Fact]
    public void AThrowingSubscriberIsLoggedAndKeptWithoutStarvingOthers()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        int calls = 0;
        h.Hub.Subscribe(1000, _ =>
        {
            calls++;
            throw new InvalidOperationException("broken client");
        });
        List<FeedUpdate> b = h.Subscribe(1000);

        h.Hub.TickOnce();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(2, calls);
        Assert.Equal(2, b.Count);
        Assert.Contains(h.Log.Entries, e => e.Exception is InvalidOperationException { Message: "broken client" });
    }

    [Fact]
    public void PipeCallbacksNeverTouchTheTree()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Subscribe(1000, out IFeedSubscription first);
        Assert.Equal(0, h.Tree.OpenCount);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        (int, int, int, int, int, int, int, int, int) Touches() => (
            h.Tree.OpenCount,
            h.Tree.SetModulesCalls,
            h.Tree.EnableStorageCount,
            h.Tree.Updates("/amdcpu/0"),
            h.Tree.Updates("/hdd/0"),
            h.Tree.Reads(CpuTotal),
            h.Disks.DescribeCalls,
            h.Disks.AllActiveQueries,
            h.Disks.SpunDownQueries);
        var before = Touches();

        // Everything a pipe session does: subscribe, replace a request, unsubscribe.
        first.Update(Requests.Of(500, ServiceModules.Storage | ServiceModules.Cpu, "0000000000000000000000000000000000000000000000000000000000000001"));
        h.Subscribe(Requests.Of(2000, ServiceModules.Memory), out IFeedSubscription second);
        second.Update(Requests.Of(2000));
        second.Dispose();
        first.Update(Requests.Of(1000));

        Assert.Equal(before, Touches());
    }

    [Fact]
    public void ResubscribeDoesNotDropTheStorageCache()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Hub.RunStorageDue();
        h.Advance(1000);

        sub.Update(Requests.Of(2000)); // the only client changes its interval
        h.Hub.TickOnce(); // before the next storage round

        Assert.Equal(40, ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.NotEqual(TimeSpan.Zero, h.Hub.RunStorageDue()); // not an idle -> active comeback
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void ResubscribeChangesTheSubscribersInterval()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        long start = h.Time.GetUtcNow().ToUnixTimeMilliseconds();
        var times = new List<long>();
        IFeedSubscription sub = h.Hub.Subscribe(Requests.Of(1000), _ => times.Add(h.Time.GetUtcNow().ToUnixTimeMilliseconds() - start));
        h.Hub.RunDue();
        h.Advance(500);

        sub.Update(Requests.Of(2000));
        for (int guard = 0; h.Time.GetUtcNow().ToUnixTimeMilliseconds() - start <= 6500; guard++)
        {
            Assert.True(guard < 100, "the sampler loop did not make progress");
            TimeSpan delay = h.Hub.RunDue();
            Assert.True(delay > TimeSpan.Zero && delay != Timeout.InfiniteTimeSpan, $"unexpected delay {delay}");
            h.Time.Advance(delay);
        }

        // Delivered at once on the replaced request, then every 2 s from the sample it carried.
        Assert.Equal([0L, 500, 2000, 4000, 6000], times);
    }

    [Fact]
    public void EveryAcceptedSubscribeIsFollowedByASchema()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.NotNull(a[0].Schema);
        Assert.Null(a[1].Schema);
        int revision = h.Hub.Revision;

        sub.Update(Requests.Of(1000)); // the very same request again
        h.Hub.RunDue();
        Assert.Equal(3, a.Count);
        Assert.NotNull(a[2].Schema);
        Assert.Equal(a[2].Schema!.Sensors.Count, a[2].Snapshot.Values.Count);

        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Null(a[^1].Schema);

        sub.Update(Requests.Of(250));
        h.Hub.RunDue();
        Assert.NotNull(a[^1].Schema);
        Assert.Equal(revision, h.Hub.Revision); // forced for this client, not a new revision
    }

    [Fact]
    public void ReplacingARequestIsAtomic()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Subscribe(Requests.Of(1000, ServiceModules.Storage), out _);
        h.Subscribe(Requests.Of(1000), out IFeedSubscription b);
        SensorHub.DesiredConfig? desired = h.Hub.Desired;
        Assert.NotNull(desired);
        Assert.Equal(ServiceModules.All, desired.Config.Enabled);

        // Dropping B's request first would leave only A's (storage off) for an instant.
        b.Update(Requests.Of(2000));
        Assert.Same(desired, h.Hub.Desired);

        h.Advance(1234);
        b.Update(Requests.Of(2000, ServiceModules.Storage));
        SensorHub.DesiredConfig? replaced = h.Hub.Desired;
        Assert.NotNull(replaced);
        Assert.Equal(desired.Version + 1, replaced.Version);
        Assert.Equal(ServiceModules.All & ~ServiceModules.Storage, replaced.Config.Enabled);
        Assert.Equal(h.Time.GetTimestamp(), replaced.RequestedAt);
    }

    [Fact]
    public void LastSubscriberLeavingKeepsTheEffectiveConfiguration()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        Assert.Null(h.Hub.Desired);
        h.Subscribe(Requests.Of(1000, ServiceModules.Memory), out IFeedSubscription sub);
        SensorHub.DesiredConfig? desired = h.Hub.Desired;
        Assert.NotNull(desired);
        Assert.Equal(1, desired.Version);
        Assert.Equal(ServiceModules.All & ~ServiceModules.Memory, desired.Config.Enabled);

        sub.Dispose();
        Assert.Same(desired, h.Hub.Desired);

        // The same request on reconnection publishes nothing new.
        h.Subscribe(Requests.Of(1000, ServiceModules.Memory), out _);
        Assert.Same(desired, h.Hub.Desired);
    }

    [Fact]
    public void ARequestThatDiffersFromTheAppliedConfigurationIsPending()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
        Assert.Equal(ProtocolConstants.Modules, LatestSchema(a).Service.ActiveModules);
        int revision = h.Hub.Revision;

        sub.Update(Requests.Of(1000, ServiceModules.Psu));
        h.Hub.RunDue();

        // The PSU group is switched only once the storage worker parks: it stays active meanwhile.
        ServiceStateBlock pending = LatestSchema(a).Service;
        Assert.Equal("pending", pending.Reconfiguration);
        Assert.Equal(ProtocolConstants.Modules, pending.ActiveModules);
        Assert.Empty(pending.SmartDisabledDrives);
        Assert.Empty(pending.SmartBlockedBy);
        Assert.Equal(revision + 1, h.Hub.Revision);

        sub.Update(Requests.Of(1000));
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
        Assert.Equal(revision + 2, h.Hub.Revision);
    }

    [Fact]
    public void AFirstRequestWithADisabledModuleIsAppliedByTheOpen()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        List<FeedUpdate> a = h.Subscribe(Requests.Of(1000, ServiceModules.Controller), out _);

        h.Hub.TickOnce();

        ServiceStateBlock state = LatestSchema(a).Service;
        Assert.Equal("applied", state.Reconfiguration);
        Assert.Equal(["cpu", "motherboard", "memory", "storage", "psu"], state.ActiveModules);
        Assert.Equal(1, h.Hub.Revision);
        Assert.Equal(0, h.Tree.SetModulesCalls);
    }

    [Fact]
    public void AResubscribeDuringATickWaitsForTheStateAfterItsRequest()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        Assert.Equal("applied", LatestSchema(a).Service.Reconfiguration);
        h.Advance(1000);

        // The request lands while this tick is updating LHM, after it read the desired configuration.
        bool asked = false;
        h.Tree.BeforeUpdate = _ =>
        {
            if (!asked)
            {
                asked = true;
                sub.Update(Requests.Of(1000, ServiceModules.Psu));
            }
        };
        h.Hub.TickOnce();
        Assert.True(asked);

        // Nothing answers the request with the state from before it.
        Assert.Single(a);

        // The next wake samples at once, answers with the new state, and keeps the sampling phase.
        TimeSpan next = h.Hub.RunDue();
        FeedUpdate answer = a[^1];
        Assert.NotNull(answer.Schema);
        Assert.Equal("pending", answer.Schema.Service.Reconfiguration);
        Assert.Equal(TimeSpan.FromMilliseconds(1000), next);
    }

    [Fact]
    public void AConfigurationChangeKeepsTheSamplingPhaseOfOtherSubscribers()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        long start = h.Time.GetUtcNow().ToUnixTimeMilliseconds();
        var deliveries = new List<(long At, long SampledAt)>();
        h.Hub.Subscribe(Requests.Of(1000, ServiceModules.Psu), u =>
            deliveries.Add((h.Time.GetUtcNow().ToUnixTimeMilliseconds() - start, (long)u.Snapshot.TimestampMs - start)));
        List<FeedUpdate> b = h.Subscribe(1000, out IFeedSubscription other);
        h.Hub.RunDue();
        h.Advance(300);

        other.Update(Requests.Of(1000, ServiceModules.Psu)); // now nobody wants the PSU: the configuration changes
        for (int guard = 0; h.Time.GetUtcNow().ToUnixTimeMilliseconds() - start <= 5000; guard++)
        {
            Assert.True(guard < 100, "the sampler loop did not make progress");
            TimeSpan delay = h.Hub.RunDue();
            Assert.True(delay > TimeSpan.Zero && delay != Timeout.InfiniteTimeSpan, $"unexpected delay {delay}");
            h.Time.Advance(delay);
        }

        Assert.Equal("pending", b[1].Schema?.Service.Reconfiguration);
        // The first client still gets a fresh sample every second on its original phase.
        Assert.Equal([(0L, 0L), (1000, 1000), (2000, 2000), (3000, 3000), (4000, 4000), (5000, 5000)], deliveries);
        Assert.Equal(7, h.Tree.Updates("/amdcpu/0")); // six on schedule, one for the new request
    }

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
        h.Hub.RunStorageDue();
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));

        sub.Update(Requests.Of(1000, ServiceModules.Memory));
        h.Hub.RunDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal(0, h.Tree.SetModulesCalls); // the storage worker has not parked yet

        // The storage worker parks at the boundary of its loop, and stays parked even when a round is due.
        Assert.Equal(Timeout.InfiniteTimeSpan, h.Hub.RunStorageDue());
        Assert.True(h.Hub.IsStorageParked);
        h.Advance(30_000);
        Assert.Equal(Timeout.InfiniteTimeSpan, h.Hub.RunStorageDue());
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
        Assert.Equal(0, h.Tree.SetModulesCalls);

        h.Hub.RunDue();
        (ServiceModules modules, _) = Assert.Single(h.Tree.SetModulesLog);
        Assert.Equal(ServiceModules.Cpu | ServiceModules.Motherboard | ServiceModules.Controller | ServiceModules.Psu, modules);
        Assert.False(h.Hub.IsStorageParked);

        h.Hub.RunStorageDue(); // released: the due round runs
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
        var before = (h.Tree.Updates("/hdd/0"), h.Tree.Reads(HddTemp), h.Disks.SpunDownQueries, h.Disks.DescribeCalls, h.Disks.AllActiveQueries);

        sub.Update(Requests.Of(1000, ServiceModules.Storage));
        h.Hub.RunDue();
        Assert.False(HasStorage(LatestSchema(a))); // at once, before the storage worker ran
        for (int round = 0; round < 3; round++)
        {
            h.Hub.RunStorageDue();
            h.Advance(30_000);
            h.Hub.RunDue();
        }

        Assert.Equal(before, (h.Tree.Updates("/hdd/0"), h.Tree.Reads(HddTemp), h.Disks.SpunDownQueries, h.Disks.DescribeCalls, h.Disks.AllActiveQueries));
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
        h.Hub.StorageOnce();
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
        h.Hub.StorageOnce();
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
        h.Hub.StorageOnce();

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
        h.Hub.StorageOnce();
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
        Assert.Equal(1, h.Disks.AllActiveQueries);
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
        h.Disks.AllActive = false;
        h.Subscribe(Requests.Of(1000, ServiceModules.Storage), out IFeedSubscription sub);
        h.Hub.TickOnce();
        for (int round = 0; round < 3; round++)
        {
            h.Hub.RunStorageDue();
            h.Advance(30_000);
        }

        Assert.Equal((0, 0, 0, 0), (h.Disks.AllActiveQueries, h.Disks.SpunDownQueries, h.Disks.DescribeCalls, h.Tree.EnableStorageCount));

        sub.Update(Requests.Of(1000));
        h.Hub.RunStorageDue();
        Assert.Equal(1, h.Disks.AllActiveQueries);
        Assert.Equal(0, h.Tree.EnableStorageCount);

        h.Disks.AllActive = true;
        h.Advance(30_000);
        h.Hub.RunStorageDue();
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
        var hub = new SensorHub(tree, new FakeDisks(), () => true, new FakeTimeProvider(), log);
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
            h.Hub.RunStorageDue();
            h.Advance(30_000);
            Assert.Equal(0, h.Disks.SpunDownQueriesOf(0));
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
        h.Disks.AllActive = false;
        h.Disks.Blockers.Add(new DriveBlocker(new DriveFacts(0, DriveAvailability.Present, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL", BusType: 0x0B, SeekPenalty: true), SpunDown: true));
        List<FeedUpdate> a = h.Subscribe(Requests.Of(1000, ServiceModules.None, HddKey), out _);
        h.Hub.TickOnce();
        for (int round = 0; round < 2; round++)
        {
            h.Hub.RunStorageDue();
            h.Advance(30_000);
        }

        h.Hub.RunDue();
        Assert.Equal(2, h.Disks.AllActiveQueries);
        Assert.Equal(0, h.Tree.EnableStorageCount);
        Assert.Equal([HddKey], LatestSchema(a).Service.SmartBlockedBy);
    }

    [Fact]
    public void GateBlockersAreReportedAsDriveKeys()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.AllActive = false;
        h.Disks.Blockers.Add(new DriveBlocker(WdcFacts(), SpunDown: true));
        h.Disks.Blockers.Add(new DriveBlocker(new DriveFacts(3, DriveAvailability.Present, "USB Bridge", null, BusType: 0x07, SeekPenalty: null), SpunDown: null)); // no serial: no key
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        Assert.Empty(LatestSchema(a).Service.SmartBlockedBy);

        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal([WdcKey], LatestSchema(a).Service.SmartBlockedBy);
        int revision = h.Hub.Revision;

        h.Disks.AllActive = true;
        h.Advance(30_000);
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Empty(LatestSchema(a).Service.SmartBlockedBy);
        Assert.True(h.Hub.Revision > revision);
    }
}
