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
public sealed partial class SensorHubTests
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
            Hub = new SensorHub(Tree, disks ?? Disks, Activity, () => true, Time, Log) { StartWorkers = false };
        }

        public FakeActivity Activity { get; } = new();

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

        /// <summary>
        /// One storage round on a working disk: a disk whose counters grow at every read
        /// (<see cref="FakeActivity"/>'s default) has worked since the reference the round
        /// before took at its end, as long as that round is at most one interval old.
        /// </summary>
        public void WorkingRound() => Hub.StorageOnce();

        /// <summary>Like <see cref="WorkingRound"/>, through one wake of the storage loop.</summary>
        public TimeSpan WorkingRoundDue() => Hub.RunStorageDue();

        public void Dispose() => Hub.Dispose();
    }

    /// <summary>The index of the (kind, name) sensor in the latest schema the subscriber saw up to <paramref name="updates"/>[<paramref name="at"/>].</summary>
    private static int SensorIndex(IReadOnlyList<FeedUpdate> updates, int at, string kind, string name)
    {
        SchemaMessage? schema = null;
        for (int i = 0; i <= at; i++)
        {
            schema = updates[i].Schema ?? schema;
        }

        Assert.NotNull(schema);
        int index = schema.Sensors.ToList().FindIndex(s => s.Kind == kind && s.Name == name);
        Assert.True(index >= 0, $"{kind}/{name} is not in the schema");
        return index;
    }

    /// <summary>The value of the (kind, name) sensor in <paramref name="updates"/>[<paramref name="at"/>], resolved against the latest schema the subscriber saw.</summary>
    private static double? ValueOf(IReadOnlyList<FeedUpdate> updates, int at, string kind, string name) =>
        updates[at].Snapshot.Values[SensorIndex(updates, at, kind, name)];

    /// <summary>The <c>held</c> flag of the (kind, name) sensor in <paramref name="updates"/>[<paramref name="at"/>], resolved like <see cref="ValueOf"/>.</summary>
    private static bool HeldOf(IReadOnlyList<FeedUpdate> updates, int at, string kind, string name)
    {
        int index = SensorIndex(updates, at, kind, name);
        Assert.Equal(updates[at].Snapshot.Values.Count, updates[at].Snapshot.Held.Count);
        return updates[at].Snapshot.Held[index];
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
    public void AStandbyDiskKeepsItsLastValuesAsHeld()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.False(HeldOf(a, a.Count - 1, "temperature", "drive"));

        // Asleep: not read again (the tree would now answer 50), its last measurement stays,
        // round after round, held from the first publication on: nothing was measured.
        h.Disks.SpunDown[0] = true;
        h.Tree.Values[HddTemp] = 50;
        int reads = h.Tree.Reads(HddTemp);
        for (int round = 0; round < 3; round++)
        {
            h.Advance(30_000);
            h.WorkingRound();
            h.Advance(1000);
            h.Hub.TickOnce();
            Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));
            Assert.True(HeldOf(a, a.Count - 1, "temperature", "drive"));
        }

        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
        Assert.Equal(reads, h.Tree.Reads(HddTemp));

        // Awake again: a measurement, published fresh.
        h.Disks.SpunDown[0] = false;
        h.Advance(30_000);
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal(50, ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.False(HeldOf(a, a.Count - 1, "temperature", "drive"));
        Assert.Equal(2, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void ADiskAsleepFromTheStartHasNoValues()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        h.Tree.Values[HddTemp] = 34;
        h.Tree.Values[WdcTemp] = 35;
        h.Disks.Facts[1] = WdcFacts();

        // The gate opens with both disks active; the second one's SMART is off, so it is never read.
        List<FeedUpdate> a = h.Subscribe(Requests.Of(1000, ServiceModules.None, WdcKey), out IFeedSubscription sub);
        h.Hub.TickOnce();
        RoundAndTick(h);

        // Its SMART is switched on once it is asleep: there is no measurement to keep.
        h.Disks.SpunDown[1] = true;
        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();
        for (int round = 0; round < 2; round++)
        {
            RoundAndTick(h);
            h.Advance(1000);
            h.Hub.TickOnce();
            Assert.Equal([HddDrive("active"), WdcDrive("standby")], Drives(a));
            Assert.Null(DriveTemperature(a, 1));
            Assert.False(DriveHeld(a, 1));
            h.Advance(30_000);
        }

        Assert.Equal(0, h.Tree.Updates("/hdd/1"));
        Assert.Equal(34, DriveTemperature(a, 0));
    }

    [Fact]
    public void AnUnknownPowerStateDoesNotKeepValues()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));

        h.Disks.SpunDown[0] = null;
        h.Advance(30_000);
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Null(ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.False(HeldOf(a, a.Count - 1, "temperature", "drive"));

        // A standby confirmed afterwards has nothing to keep: the measurement did not survive.
        h.Disks.SpunDown[0] = true;
        h.Advance(30_000);
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Null(ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.False(HeldOf(a, a.Count - 1, "temperature", "drive"));
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void StorageValuesAreHeldAfterTheirFirstPublication()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound();

        h.Advance(1000);
        h.Hub.TickOnce(); // the first snapshot with this round's measurement
        Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.False(HeldOf(a, a.Count - 1, "temperature", "drive"));

        for (int tick = 0; tick < 3; tick++)
        {
            h.Advance(1000);
            h.Hub.TickOnce(); // a new seq, the same measurement
            Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));
            Assert.True(HeldOf(a, a.Count - 1, "temperature", "drive"));
        }
    }

    [Fact]
    public void ANewRoundPublishesFreshValuesAgain()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.True(HeldOf(a, a.Count - 1, "temperature", "drive"));

        // The next round measures the very same value: it is a new measurement all the same.
        h.Advance(28_000);
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal(2, h.Tree.Updates("/hdd/0"));
        Assert.Null(a[^1].Schema);
        Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.False(HeldOf(a, a.Count - 1, "temperature", "drive"));

        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.True(HeldOf(a, a.Count - 1, "temperature", "drive"));
    }

    [Fact]
    public void NonStorageValuesAreNeverHeld()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Initial.Add(Ram());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[CpuTotal] = 12;
        h.Tree.Values[RamUsed] = 8;
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound();
        h.Disks.SpunDown[0] = true;
        for (int tick = 0; tick < 3; tick++)
        {
            h.Advance(1000);
            h.Hub.TickOnce(); // the same CPU and memory values again and again
        }

        h.Advance(30_000);
        h.WorkingRound(); // and a standby round, with a kept disk value
        for (int tick = 0; tick < 2; tick++)
        {
            h.Advance(1000);
            h.Hub.TickOnce(); // its first publication, then a republication
        }

        // Whatever the storage round a snapshot carries (new, republished, with kept values),
        // only the disk's value is ever flagged.
        SchemaMessage schema = LatestSchema(a);
        Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));
        for (int i = 0; i < schema.Sensors.Count; i++)
        {
            bool storage = schema.Devices.Single(d => d.Id == schema.Sensors[i].DeviceId).Kind == "storage";
            Assert.Equal(storage, a[^1].Snapshot.Held[i]);
        }

        for (int at = 0; at < a.Count; at++)
        {
            Assert.Equal(12, ValueOf(a, at, "load", "total"));
            Assert.False(HeldOf(a, at, "load", "total"));
            Assert.Equal(8 * TwoPow30, ValueOf(a, at, "data", "used"));
            Assert.False(HeldOf(a, at, "data", "used"));
        }
    }

    [Fact]
    public void KeptValuesDoNotSurviveAnIdleHub()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription first);
        h.Hub.TickOnce();
        h.WorkingRoundDue();
        h.Disks.SpunDown[0] = true;
        h.Advance(30_000);
        h.WorkingRoundDue();
        h.Hub.TickOnce();
        Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.True(HeldOf(a, a.Count - 1, "temperature", "drive"));
        first.Dispose();

        // A client comes back while the disk still sleeps: the round that runs at once has no
        // earlier measurement to keep, and none comes back in the rounds after it.
        h.Advance(5_000);
        List<FeedUpdate> b = h.Subscribe(1000);
        h.Hub.TickOnce(); // before any new storage round
        Assert.Null(ValueOf(b, b.Count - 1, "temperature", "drive"));
        Assert.False(HeldOf(b, b.Count - 1, "temperature", "drive"));

        for (int round = 0; round < 2; round++)
        {
            h.WorkingRoundDue();
            h.Advance(1000);
            h.Hub.TickOnce();
            Assert.Null(ValueOf(b, b.Count - 1, "temperature", "drive"));
            Assert.False(HeldOf(b, b.Count - 1, "temperature", "drive"));
            h.Advance(30_000);
        }

        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void UnknownNoMediaDisabledAndReadErrorsPublishMissingNotHeld()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();

        void Measure()
        {
            h.Disks.SpunDown[0] = false;
            h.Disks.Facts.TryRemove(0, out _);
            h.Tree.Failing.Clear();
            h.Advance(30_000);
            RoundAndTick(h);
            Assert.Equal(34, DriveTemperature(a, 0));
            Assert.False(DriveHeld(a, 0));
        }

        void RoundPublishesMissing()
        {
            h.Advance(30_000);
            RoundAndTick(h);
            h.Advance(1000);
            h.Hub.TickOnce(); // a republication too
            Assert.Null(DriveTemperature(a, 0));
            Assert.False(DriveHeld(a, 0));
        }

        // Unknown power state.
        Measure();
        h.Disks.SpunDown[0] = null;
        RoundPublishesMissing();

        // A read error on an active disk.
        Measure();
        h.Tree.Failing["/hdd/0"] = true;
        RoundPublishesMissing();

        // "No media" in the round's enumeration, whatever a power check would answer.
        Measure();
        h.Disks.Facts[0] = new DriveFacts(0, DriveAvailability.NoMedia, null, null, null, null);
        h.Disks.SpunDown[0] = true;
        RoundPublishesMissing();
        Assert.Equal("noMedia", Assert.Single(Drives(a)).State);

        // SMART switched off for the disk while it sleeps: out of the schema, and nothing kept
        // for when it comes back.
        Measure();
        h.Disks.SpunDown[0] = true;
        sub.Update(Requests.Of(1000, ServiceModules.None, HddKey));
        h.Hub.RunDue();
        RoundAndTick(h);
        Assert.DoesNotContain(LatestSchema(a).Devices, d => d.Kind == "storage");
        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();
        RoundAndTick(h); // watched anew, so asked: standby, and nothing to keep
        Assert.Equal([HddDrive("standby")], Drives(a));
        Assert.Null(DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        // Storage switched off as a whole, then on again with the disk asleep.
        Measure();
        h.Disks.SpunDown[0] = true;
        sub.Update(Requests.Of(1000, ServiceModules.Storage));
        h.Hub.RunDue();
        RoundAndTick(h);
        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();
        RoundAndTick(h); // watched anew, so asked: standby, and nothing to keep
        Assert.Equal([HddDrive("standby")], Drives(a));
        Assert.Null(DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void AReplacementDiskDoesNotInheritHeldValues(bool noticedByLhm)
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        RoundAndTick(h);
        Assert.Equal(34, DriveTemperature(a, 0));

        // Another disk of the same model at the same PhysicalDrive number and under the same LHM
        // identifier, asleep. Whether or not LHM's own serial followed, the drive's descriptor
        // (read with access 0 in every round) is not the measured disk's any more.
        h.Disks.Facts[0] = new DriveFacts(0, DriveAvailability.Present, "ST2000DM008-2FR102", "OTHER-SERIAL", BusType: 0x0B, SeekPenalty: true);
        h.Disks.SpunDown[0] = true;
        if (noticedByLhm)
        {
            h.Tree.Replace(Cpu(), Hdd() with { Storage = Hdd().Storage! with { DriveSerial = "OTHER" } });
        }

        // A new identity is asked once when it is first listed, and afterwards when it works:
        // standby both times, and the other disk's value is never kept for it.
        for (int round = 0; round < 2; round++)
        {
            h.Advance(30_000);
            RoundAndTick(h);
            Assert.Equal(DriveKey.Compute("ST2000DM008-2FR102", "OTHER-SERIAL"), Assert.Single(Drives(a)).Key);
            Assert.Equal("standby", Assert.Single(Drives(a)).State);
            Assert.Null(DriveTemperature(a, 0));
            Assert.False(DriveHeld(a, 0));
        }

        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void AStalledStorageWorkerExpiresEvenPreviouslyKeptValues()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound();
        h.Disks.SpunDown[0] = true;

        // Standby rounds that end renew the kept measurement, well past the 60 s of one round.
        for (int round = 0; round < 4; round++)
        {
            h.Advance(30_000);
            h.WorkingRound();
            h.Hub.TickOnce();
            Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));
            Assert.True(HeldOf(a, a.Count - 1, "temperature", "drive"));
        }

        h.Advance(59_000);
        h.Hub.TickOnce();
        Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.True(HeldOf(a, a.Count - 1, "temperature", "drive"));

        h.Advance(2_000); // the storage worker is stuck: no round has ended for more than 60 s
        h.Hub.TickOnce();
        Assert.Null(ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.False(HeldOf(a, a.Count - 1, "temperature", "drive"));
    }

    [Fact]
    public void AStandbyRoundAfterAStallDoesNotBringExpiredValuesBack()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound();
        h.Disks.SpunDown[0] = true;
        h.Advance(30_000);
        h.WorkingRound();
        h.Hub.TickOnce();
        Assert.Equal(34, ValueOf(a, a.Count - 1, "temperature", "drive"));
        Assert.True(HeldOf(a, a.Count - 1, "temperature", "drive"));

        // No round for more than 60 s: the measurement expired, and the standby rounds that end
        // afterwards have nothing current to keep. It stays absent until the disk is read again.
        h.Advance(61_000);
        for (int round = 0; round < 2; round++)
        {
            h.WorkingRound();
            h.Hub.TickOnce();
            Assert.Equal([HddDrive("standby")], Drives(a));
            Assert.Null(ValueOf(a, a.Count - 1, "temperature", "drive"));
            Assert.False(HeldOf(a, a.Count - 1, "temperature", "drive"));
            h.Advance(30_000);
        }

        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void AStandbyRoundThatTookTooLongIsNotKeptFromByTheNextOne()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        h.Tree.Values[HddTemp] = 34;
        h.Tree.Values[WdcTemp] = 35;
        h.Disks.Facts[1] = WdcFacts();
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound();

        // The round that keeps the sleeping disk's value is stuck in the other disk's update for
        // more than 60 s: it is published already expired (a round's time is its start).
        h.Disks.SpunDown[0] = true;
        h.Advance(30_000);
        h.Tree.BeforeUpdate = root =>
        {
            if (root.Identifier == "/hdd/1")
            {
                h.Advance(61_000);
            }
        };
        h.WorkingRound();
        h.Tree.BeforeUpdate = null;
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal([HddDrive("standby"), WdcDrive("active")], Drives(a));
        Assert.Null(DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        // The next round does not take the expired value over as if it were current.
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal([HddDrive("standby"), WdcDrive("active")], Drives(a));
        Assert.Null(DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));
        Assert.Equal(35, DriveTemperature(a, 1));
        Assert.False(DriveHeld(a, 1));
    }

    [Fact]
    public void ARoundInFlightWhenTheHubGoesIdleKeepsNothing()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        h.Tree.Values[HddTemp] = 34;
        h.Tree.Values[WdcTemp] = 35;
        h.Disks.Facts[1] = WdcFacts();
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription first);
        h.Hub.TickOnce();
        h.WorkingRoundDue();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal(34, DriveTemperature(a, 0));

        // The last client leaves while a round is running, after that round took the sleeping
        // disk's value over from the round before.
        h.Disks.SpunDown[0] = true;
        h.Advance(30_000);
        h.Tree.BeforeUpdate = root =>
        {
            if (root.Identifier == "/hdd/1")
            {
                first.Dispose();
            }
        };
        h.WorkingRoundDue();
        h.Tree.BeforeUpdate = null;
        Assert.Equal(2, h.Tree.Updates("/hdd/1")); // the round did run to its end

        // A client comes back at once, the disk still asleep: neither the round that was in
        // flight nor the one that runs now has a value from before the idle period.
        h.Advance(5_000);
        List<FeedUpdate> b = h.Subscribe(1000);
        h.Hub.TickOnce();
        Assert.Null(DriveTemperature(b, 0));
        Assert.False(DriveHeld(b, 0));

        for (int round = 0; round < 2; round++)
        {
            h.WorkingRoundDue();
            h.Advance(1000);
            h.Hub.TickOnce();
            Assert.Equal([HddDrive("standby"), WdcDrive("active")], Drives(b));
            Assert.Null(DriveTemperature(b, 0));
            Assert.False(DriveHeld(b, 0));
            h.Advance(30_000);
        }

        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
    }

    [Fact]
    public void AKeptValueArrivesTogetherWithItsStandbyState()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 34;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound(); // round 1: active, measured, not published yet

        // The sampler is stopped inside its tick, after it took round 1.
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

        // Meanwhile round 2 is published: the disk sleeps, its value is kept.
        h.Disks.SpunDown[0] = true;
        h.WorkingRound();
        release.Set();
        Assert.True(sampler.Join(TimeSpan.FromSeconds(10)));
        h.Tree.BeforeUpdate = null;

        // That tick is round 1 throughout: an active disk and a measurement.
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal(34, DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        // The next one is round 2 throughout: standby and the held flag, in its first publication.
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal([HddDrive("standby")], a[^1].Schema!.Service.Drives);
        Assert.Equal(34, DriveTemperature(a, 0));
        Assert.True(DriveHeld(a, 0));
    }

    [Fact]
    public void AnAppliedStorageRequestArrivesTogetherWithItsDriveList()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        h.Disks.Facts[1] = WdcFacts();
        List<FeedUpdate> a = h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        RoundAndTick(h);
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));

        // A sampling tick in the middle of the round that takes the request (the storage worker
        // holds no lock there): the request is not reported applied ahead of its drive list.
        sub.Update(Requests.Of(1000, ServiceModules.None, WdcKey));
        h.Hub.RunDue();
        ServiceStateBlock? during = null;
        h.Tree.BeforeUpdate = root =>
        {
            if (root.Type == HardwareType.Storage)
            {
                h.Advance(1000);
                h.Hub.TickOnce();
                during = LatestSchema(a).Service;
            }
        };
        h.WorkingRoundDue();
        h.Tree.BeforeUpdate = null;

        Assert.NotNull(during);
        Assert.Equal("pending", during.Reconfiguration);
        Assert.Empty(during.SmartDisabledDrives);
        Assert.Equal([HddDrive("active"), WdcDrive("active")], during.Drives);

        h.Advance(1000);
        h.Hub.TickOnce();
        ServiceStateBlock after = LatestSchema(a).Service;
        Assert.Equal("applied", after.Reconfiguration);
        Assert.Equal([WdcKey], after.SmartDisabledDrives);
        Assert.Equal([HddDrive("active"), WdcDrive("smartOff")], after.Drives);
    }

    [Fact]
    public void UnknownPowerStateSkipsHdd()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        h.WorkingRound(); // active: the gate opens and the disk is identified
        int reads = h.Tree.Reads(HddTemp);

        h.Disks.SpunDown[0] = null;
        h.Advance(30_000);
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
        Assert.Equal(reads, h.Tree.Reads(HddTemp));
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
        h.Disks.SpunDown[0] = true;
        h.Subscribe(1000);

        for (int i = 0; i < 3; i++)
        {
            h.Hub.TickOnce();
            h.WorkingRound();
            h.Advance(1000);
        }

        Assert.Equal(1, h.Tree.OpenCount);
        Assert.Equal(0, h.Tree.EnableStorageCount);
        Assert.Equal(0, h.Tree.Updates("/hdd/0"));
        Assert.Equal(0, h.Tree.Reads(HddTemp));
        Assert.Equal(0, h.Disks.DescribeCalls);
        Assert.Equal(3, h.Disks.EnumerateCalls); // only the gate: the drives listed once per round,
        Assert.Equal(3, h.Disks.SpunDownQueries); // and its power check of a blocker whose counters grow
    }

    [Fact]
    public void StorageIsEnabledOnlyWhenEveryRotationalDiskIsActive()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.SpunDown[0] = true;
        h.Subscribe(1000);
        h.Hub.TickOnce();

        h.WorkingRound();
        Assert.Equal(0, h.Tree.EnableStorageCount);
        Assert.Equal(0, h.Tree.Updates("/hdd/0"));

        h.Disks.SpunDown[0] = false;
        h.Advance(30_000);
        h.WorkingRound();
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));

        // Once enabled it stays enabled: the gate is not asked again, the per-disk check takes over.
        h.Advance(30_000);
        h.WorkingRound();
        Assert.Equal(1, h.Tree.EnableStorageCount);
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

        h.WorkingRound();
        h.Advance(30_000);
        h.WorkingRound();
        Assert.Equal(0, h.Tree.EnableStorageCount);

        spunDown = false;
        h.Advance(300_000); // no serial, so no key and no counters: asked again only after five minutes
        h.WorkingRound();
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

        var storage = new Thread(() => h.WorkingRound()) { IsBackground = true, Name = "test-storage" };
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
        var hub = new SensorHub(tree, new FakeDisks(), new FakeActivity(), () => true, new FakeTimeProvider(), new ListLogger<SensorHub>());
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
            var hub = new SensorHub(tree, new FakeDisks(), new FakeActivity(), () => true, new FakeTimeProvider(), new ListLogger<SensorHub>());
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
        var hub = new SensorHub(tree, new FakeDisks(), new FakeActivity(), () => true, new FakeTimeProvider(), log) { WorkerJoinTimeout = TimeSpan.FromMilliseconds(200) };
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

        var storage = new Thread(() => h.WorkingRound()) { Name = "oma-storage" };
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

        var storage = new Thread(() => h.WorkingRound()) { IsBackground = true };
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
        h.WorkingRound();
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
        h.WorkingRound();
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
        h.WorkingRound();
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

        h.WorkingRound();
        Assert.Equal(0, h.Tree.Updates("/hdd/0"));

        h.Disks.Facts[0] = new DriveFacts(0, DriveAvailability.Present, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL", BusType: 0x0B, SeekPenalty: true);
        h.Advance(30_000);
        h.WorkingRound();
        Assert.Equal(0, h.Tree.Updates("/hdd/0")); // re-described, and its standby is honoured
        Assert.Equal(2, h.Disks.DescribeCalls);

        h.Disks.SpunDown[0] = false;
        h.Advance(30_000);
        h.WorkingRound();
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
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        Assert.Equal(new StorageHint(0, null, null), Assert.Single(LatestSchema(a).Devices, d => d.Kind == "storage").Hint);

        h.Disks.Facts.TryRemove(0, out _);
        h.Advance(30_000);
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();

        Assert.NotNull(a[^1].Schema);
        Assert.Equal(new StorageHint(0, "ST2000DM008-2FR102", "DESCRIPTOR-SERIAL"), Assert.Single(LatestSchema(a).Devices, d => d.Kind == "storage").Hint);

        h.Advance(30_000);
        h.WorkingRound();
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
        h.WorkingRoundDue();
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));
        first.Dispose();

        h.Advance(5_000);
        h.Subscribe(1000);
        h.Hub.RunStorageDue();

        // The round ran at once, and as the first of a storage episode it read the disk
        // without a reference (the one of the round before is dropped when the hub goes idle).
        Assert.Equal(2, h.Disks.EnumerateCalls);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
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
            h.WorkingRound();
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
        h.WorkingRound();
        h.Advance(1000);
        h.Hub.TickOnce();
        string firstId = Assert.Single(LatestSchema(a).Devices, d => d.Kind == "storage").Id;

        h.Disks.Facts.TryRemove(1, out _);
        h.Advance(30_000);
        h.WorkingRound();
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
            h.WorkingRound();
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
            h.Disks.EnumerateCalls + h.Activity.Reads + h.Activity.PowerQueries,
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
        Assert.Empty(pending.Drives);
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
}
