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

        public List<FeedUpdate> Subscribe(uint intervalMs) => Subscribe(intervalMs, out _);

        public List<FeedUpdate> Subscribe(uint intervalMs, out IDisposable subscription)
        {
            var received = new List<FeedUpdate>();
            subscription = Hub.Subscribe(intervalMs, u =>
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
        List<FeedUpdate> a = h.Subscribe(1000, out IDisposable subA);
        List<FeedUpdate> b = h.Subscribe(1000, out IDisposable subB);

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
        h.Subscribe(1000, out IDisposable sub);
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
        h.Subscribe(1000, out IDisposable sub);
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
        h.Subscribe(1000, out IDisposable first);
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
}
