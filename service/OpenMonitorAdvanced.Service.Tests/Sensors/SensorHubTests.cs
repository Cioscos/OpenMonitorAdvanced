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
        /// One storage round with its baseline taken right before it, so a disk whose counters
        /// grow at every read (<see cref="FakeActivity"/>'s default) counts as working.
        /// </summary>
        public void WorkingRound()
        {
            Hub.BaselineOnce();
            Hub.StorageOnce();
        }

        /// <summary>Like <see cref="WorkingRound"/>, through one wake of the storage loop.</summary>
        public TimeSpan WorkingRoundDue()
        {
            Hub.BaselineOnce();
            return Hub.RunStorageDue();
        }

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

    /// <summary>The <c>held</c> flag of the (kind, name) sensor in <paramref name="updates"/>[<paramref name="at"/>], resolved like <see cref="ValueOf"/>.</summary>
    private static bool HeldOf(IReadOnlyList<FeedUpdate> updates, int at, string kind, string name)
    {
        SchemaMessage? schema = null;
        for (int i = 0; i <= at; i++)
        {
            schema = updates[i].Schema ?? schema;
        }

        Assert.NotNull(schema);
        int index = schema.Sensors.ToList().FindIndex(s => s.Kind == kind && s.Name == name);
        Assert.True(index >= 0, $"{kind}/{name} is not in the schema");
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
            RoundAndTick(h); // after a change of the drive list or of the SMART selection no baseline counts: idle
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
        RoundAndTick(h); // back, with no baseline: idle, and nothing to keep
        Assert.Equal([HddDrive("idle")], Drives(a));
        Assert.Null(DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));
        h.Advance(30_000);
        RoundAndTick(h);
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
        RoundAndTick(h); // the first round of a storage episode asks it: standby, and nothing to keep
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

        // The first round sees a changed drive list, so no baseline counts and the disk is not
        // asked (idle); the second one asks it (standby). Neither state keeps the other disk's value.
        for (int round = 0; round < 2; round++)
        {
            h.Advance(30_000);
            RoundAndTick(h);
            Assert.Equal(DriveKey.Compute("ST2000DM008-2FR102", "OTHER-SERIAL"), Assert.Single(Drives(a)).Key);
            Assert.Equal(round == 0 ? "idle" : "standby", Assert.Single(Drives(a)).State);
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
        // without a baseline (none is taken while nobody is subscribed).
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
    /// One storage round (when due) and the sampling tick that publishes it. The baseline is
    /// taken right before the round, so a disk whose counters grow at every read (the fake's
    /// default) counts as working, and one with scripted counters as quiet.
    /// </summary>
    private static void RoundAndTick(Harness h)
    {
        h.Hub.BaselineOnce();
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

        h.Disks.SpunDown[1] = false;
        h.Disks.SpunDown[3] = false;
        h.Advance(300_000); // the dock has no key, so no counters: it is asked again only after five minutes
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

    // ---- M6b Task 10: power checks and SMART only after recent disk activity ----

    /// <summary>
    /// A harness with the HDD (drive 0, 40 degrees) in LHM's storage group, one subscriber and
    /// the gate open: the opening round asked the disk once and updated it once. Its counters
    /// stand still unless the test makes it work, and an update would now read 50 degrees.
    /// </summary>
    private static Harness QuietHddHarness(out List<FeedUpdate> updates, out IFeedSubscription subscription)
    {
        var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        h.Activity.Counters[0] = new DiskCounters(100, 100);
        updates = h.Subscribe(Requests.Of(1000), out subscription);
        h.Hub.TickOnce();
        RoundAndTick(h);
        Assert.Equal((1, 1, 1), (h.Tree.EnableStorageCount, h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        h.Tree.Values[HddTemp] = 50;
        return h;
    }

    /// <summary>
    /// A harness with two HDDs whose counters stand still (drive 1 answers
    /// <paramref name="wdcSpunDown"/>), one subscriber and the first gate round done.
    /// </summary>
    private static Harness GateHarness(bool? wdcSpunDown, out List<FeedUpdate> updates)
    {
        var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        h.Disks.Facts[1] = WdcFacts();
        h.Disks.SpunDown[1] = wdcSpunDown;
        h.Activity.Counters[0] = new DiskCounters(100, 100);
        h.Activity.Counters[1] = new DiskCounters(200, 200);
        updates = h.Subscribe(1000);
        h.Hub.TickOnce();
        RoundAndTick(h);
        return h;
    }

    /// <summary>How many times each of the two HDDs was asked for its power mode.</summary>
    private static (int Hdd, int Wdc) Asked(Harness h) => (h.Disks.SpunDownQueriesOf(0), h.Disks.SpunDownQueriesOf(1));

    /// <summary>
    /// The next storage round as the worker runs it: the wake for the baseline ten seconds
    /// before, <paramref name="meanwhile"/> in between, the round, then a sampling tick.
    /// </summary>
    private static void TimedRound(Harness h, Action? meanwhile = null)
    {
        h.Time.Advance(h.Hub.RunStorageDue()); // nothing is due: the worker sleeps until the baseline
        Assert.Equal(DiskActivity.Window, h.Hub.RunStorageDue()); // the baseline
        h.Time.Advance(DiskActivity.Window);
        meanwhile?.Invoke();
        h.Hub.RunStorageDue(); // the round
        h.Advance(1000);
        h.Hub.RunDue();
    }

    [Fact]
    public void ADiskThatWindowsTurnedOffIsStandbyWithoutAnyCommand()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);
        Assert.Equal(40, DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        // Windows comes first: not even counters that grew make the hub ask a disk it turned
        // off (the question would power it up).
        h.Activity.Powered[0] = false;
        for (int round = 0; round < 2; round++)
        {
            TimedRound(h, () => h.Activity.Work(0));

            Assert.Equal([HddDrive("standby")], Drives(a));
            Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
            Assert.Equal(40, DriveTemperature(a, 0));
            Assert.True(DriveHeld(a, 0));
        }
    }

    [Fact]
    public void AnIdleHddIsNeitherAskedNorUpdatedAndKeepsHeldValues()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);

        for (int round = 0; round < 3; round++)
        {
            TimedRound(h);

            Assert.Equal([HddDrive("idle")], Drives(a));
            Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
            Assert.Equal(40, DriveTemperature(a, 0));
            Assert.True(DriveHeld(a, 0));
        }

        Assert.Equal(1, h.Tree.Reads(HddTemp));
    }

    [Fact]
    public void AnHddWithRecentIoIsAskedAndUpdated()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);

        TimedRound(h, () => h.Activity.Work(0));

        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal(50, DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        // Asked because it worked, it may still answer standby: then it is left alone as before.
        h.Disks.SpunDown[0] = true;
        h.Tree.Values[HddTemp] = 60;
        TimedRound(h, () => h.Activity.Work(0));

        Assert.Equal([HddDrive("standby")], Drives(a));
        Assert.Equal((3, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal(50, DriveTemperature(a, 0));
        Assert.True(DriveHeld(a, 0));
    }

    [Fact]
    public void SolidStateDisksAreUpdatedEveryRound()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd()); // LHM's node of drive 0, here a SATA SSD
        h.Tree.Values[HddTemp] = 40;
        h.Disks.Facts[0] = new DriveFacts(0, DriveAvailability.Present, "Samsung SSD 870 EVO", "S5Y1NX0R", BusType: 0x0B, SeekPenalty: false);
        h.Disks.Facts[2] = new DriveFacts(2, DriveAvailability.Present, "Fanxiang S880 2TB", "NVME-SERIAL", DriveFacts.BusTypeNvme, SeekPenalty: false);
        h.Disks.Facts[3] = new DriveFacts(3, DriveAvailability.Present, "Virtual Disk", "VHD", DriveFacts.BusTypeFileBackedVirtual, SeekPenalty: null);
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();

        for (int round = 1; round <= 3; round++)
        {
            // Thirty seconds to the next round, with no wake for a baseline: nothing to watch.
            Assert.Equal(SensorHub.StorageInterval, h.Hub.RunStorageDue());
            h.Advance(1000);
            h.Hub.RunDue();

            Assert.Equal(round, h.Tree.Updates("/hdd/0"));
            Assert.All(Drives(a), d => Assert.Equal("active", d.State));
            Assert.Equal(40, DriveTemperature(a, 0));
            Assert.False(DriveHeld(a, 0));
            h.Advance(29_000);
        }

        Assert.Equal((0, 0, 0), (h.Disks.SpunDownQueries, h.Activity.Reads, h.Activity.PowerQueries));
    }

    [Fact]
    public void TheWorkerWakesTenSecondsBeforeARound()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd()); // a busy disk: its counters grow at every read
        h.Subscribe(1000);
        h.Hub.TickOnce();

        // The first round; the next wake is for its successor's baseline.
        Assert.Equal(TimeSpan.FromSeconds(20), h.Hub.RunStorageDue());
        Assert.Equal((1, 1), (h.Activity.ReadsOf(0), h.Disks.SpunDownQueries));

        h.Advance(20_000);
        Assert.Equal(TimeSpan.FromSeconds(10), h.Hub.RunStorageDue());
        Assert.Equal(2, h.Activity.ReadsOf(0));
        Assert.Equal((1, 1), (h.Disks.EnumerateCalls, h.Disks.SpunDownQueries)); // the baseline reads the counters and nothing else

        // Woken again before the round (a request, say): the baseline stays the one it took.
        h.Advance(4_000);
        Assert.Equal(TimeSpan.FromSeconds(6), h.Hub.RunStorageDue());
        Assert.Equal(2, h.Activity.ReadsOf(0));

        // The round samples again, before asking anything: the counters grew, so the disk is asked.
        h.Advance(6_000);
        Assert.Equal(TimeSpan.FromSeconds(20), h.Hub.RunStorageDue());
        Assert.Equal(3, h.Activity.ReadsOf(0));
        Assert.Equal((2, 2), (h.Disks.EnumerateCalls, h.Disks.SpunDownQueries));
    }

    [Fact]
    public void ABaselineFromAnotherIdentityOrBeforeSuspendIsNotUsed()
    {
        using Harness h = QuietHddHarness(out _, out _);

        // Another disk took the drive number after the baseline: the counters are not its own.
        TimedRound(h, () =>
        {
            h.Disks.Facts[0] = new DriveFacts(0, DriveAvailability.Present, "ST2000DM008-2FR102", "ANOTHER-SERIAL", BusType: 0x0B, SeekPenalty: true);
            h.Activity.Work(0);
        });
        Assert.Equal(1, h.Disks.SpunDownQueries);

        // A disk plugged in elsewhere renumbers the drives: no baseline from before counts.
        TimedRound(h, () =>
        {
            h.Disks.Facts[1] = WdcFacts();
            h.Activity.Work(0);
        });
        Assert.Equal(1, h.Disks.SpunDownQueries);

        // The system slept between the baseline and the round.
        TimedRound(h, () =>
        {
            h.Advance(600_000);
            h.Activity.Work(0);
        });
        Assert.Equal(1, h.Disks.SpunDownQueries);
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));

        // The same disks, on time: used.
        TimedRound(h, () => h.Activity.Work(0));
        Assert.Equal(2, h.Disks.SpunDownQueriesOf(0));
    }

    [Fact]
    public void ALateBaselineDoesNotAuthorizeSmart()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);

        // The round comes later after the baseline than the window and its tolerance: the
        // growth cannot be placed in the last ten seconds.
        TimedRound(h, () =>
        {
            h.Time.Advance(DiskActivity.Tolerance + TimeSpan.FromMilliseconds(1));
            h.Activity.Work(0);
        });
        Assert.Equal([HddDrive("idle")], Drives(a));
        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal(40, DriveTemperature(a, 0));
        Assert.True(DriveHeld(a, 0));

        // Within the tolerance (a timer is never exactly on time) it is used.
        TimedRound(h, () =>
        {
            h.Time.Advance(DiskActivity.Tolerance);
            h.Activity.Work(0);
        });
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
    }

    [Fact]
    public void TheFirstRoundAfterTheGateOpensDoesNotAskAgain()
    {
        using Harness h = GateHarness(wdcSpunDown: true, out List<FeedUpdate> a);
        h.Tree.Values[HddTemp] = 40;
        h.Tree.Values[WdcTemp] = 35;

        // The round that opens the gate: the blocker's answer and the full check of the other
        // disk. The round goes on with those answers: no disk is asked again, each is updated.
        h.Disks.SpunDown[1] = false;
        TimedRound(h, () => h.Activity.Work(1));
        Assert.Equal((2, 2), Asked(h));
        Assert.Equal((1, 1, 1), (h.Tree.EnableStorageCount, h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));
        Assert.Equal(40, DriveTemperature(a, 0));
        Assert.Equal(35, DriveTemperature(a, 1));

        // The round after it is an ordinary one: no activity, no question.
        TimedRound(h);
        Assert.Equal((2, 2), Asked(h));
        Assert.Equal((1, 1), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));
        Assert.Equal([HddDrive("idle"), WdcDrive("idle")], Drives(a));
    }

    [Fact]
    public void TheRoundThatOpensTheGateUpdatesEveryActiveDiskOnce()
    {
        // Two spinning disks whose counters stand still: the gate's first round is also its
        // full check, and the storage episode's first round.
        using Harness h = GateHarness(wdcSpunDown: false, out List<FeedUpdate> a);

        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal((1, 1), Asked(h));
        Assert.Equal((1, 1), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));
        Assert.Equal(1, h.Disks.EnumerateCalls);
    }

    [Fact]
    public void TheSecondRoundWithoutActivityIsIdleAndKeepsTheFirstValues()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);
        Assert.Equal(40, DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        TimedRound(h);

        Assert.Equal([HddDrive("idle")], Drives(a));
        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal(40, DriveTemperature(a, 0));
        Assert.True(DriveHeld(a, 0));
    }

    /// <summary>Storage switched off for every client, then on again; the worker takes each request, and the round at once is published.</summary>
    private static void SwitchStorageOffAndOn(Harness h, IFeedSubscription subscription)
    {
        subscription.Update(Requests.Of(1000, ServiceModules.Storage));
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        subscription.Update(Requests.Of(1000));
        h.Hub.RunDue();
        h.Hub.RunStorageDue(); // a round at once
        h.Advance(1000);
        h.Hub.RunDue();
    }

    [Fact]
    public void AReenabledStorageReadsAnActiveHddOnce()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out IFeedSubscription sub);

        SwitchStorageOffAndOn(h, sub);

        // A storage episode starts: the quiet disk is asked once and read, although nothing
        // shows that it works.
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal(50, DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        // Once: from the second round on it takes activity.
        TimedRound(h);
        TimedRound(h);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("idle")], Drives(a));
        Assert.Equal(50, DriveTemperature(a, 0));
        Assert.True(DriveHeld(a, 0));
    }

    [Fact]
    public void ANewSubscriberAfterAnIdleHubReadsAnActiveHddOnce()
    {
        using Harness h = QuietHddHarness(out _, out IFeedSubscription sub);
        sub.Dispose();
        h.Advance(5_000);

        List<FeedUpdate> b = h.Subscribe(1000);
        h.Hub.RunDue();
        h.Hub.RunStorageDue(); // a round at once
        h.Advance(1000);
        h.Hub.RunDue();

        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("active")], Drives(b));
        Assert.Equal(50, DriveTemperature(b, 0));
        Assert.False(DriveHeld(b, 0));

        TimedRound(h);
        TimedRound(h);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("idle")], Drives(b));
        Assert.Equal(50, DriveTemperature(b, 0));
        Assert.True(DriveHeld(b, 0));

        // A disk that answers standby in the first round has no earlier round to keep values from.
        using Harness asleep = QuietHddHarness(out _, out IFeedSubscription only);
        only.Dispose();
        asleep.Disks.SpunDown[0] = true;
        List<FeedUpdate> c = asleep.Subscribe(1000);
        asleep.Hub.RunDue();
        asleep.Hub.RunStorageDue();
        asleep.Advance(1000);
        asleep.Hub.RunDue();
        Assert.Equal((2, 1), (asleep.Disks.SpunDownQueries, asleep.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("standby")], Drives(c));
        Assert.Null(DriveTemperature(c, 0));
        Assert.False(DriveHeld(c, 0));
    }

    [Fact]
    public void TheFirstRoundNeverAsksADiskThatWindowsTurnedOff()
    {
        // Storage re-enabled.
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out IFeedSubscription sub);
        h.Activity.Powered[0] = false;
        SwitchStorageOffAndOn(h, sub);
        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("standby")], Drives(a));
        Assert.Null(DriveTemperature(a, 0)); // the values went when storage was switched off

        // A subscriber after an idle hub.
        sub.Dispose();
        h.Advance(5_000);
        List<FeedUpdate> b = h.Subscribe(1000);
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("standby")], Drives(b));

        // Turned on again, it is an ordinary round by then: asked only once it works.
        h.Activity.Powered[0] = true;
        TimedRound(h);
        Assert.Equal(1, h.Disks.SpunDownQueries);
        Assert.Equal([HddDrive("idle")], Drives(b));
    }

    [Fact]
    public void AFailingEnableStorageDoesNotRepeatTheFullCheckEveryRound()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Storage.Add(Wdc());
        h.Disks.Facts[1] = WdcFacts();
        h.Activity.Counters[0] = new DiskCounters(100, 100);
        h.Activity.Counters[1] = new DiskCounters(200, 200);
        h.Tree.FailEnableStorage = true;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();

        RoundAndTick(h);
        Assert.Equal((1, 1), Asked(h));
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a)); // nothing blocks; the group is just not there

        // A round every thirty seconds: the answers are kept, and the full check with a new
        // attempt comes only every five minutes.
        for (int round = 1; round <= 19; round++)
        {
            TimedRound(h);
            Assert.Equal((1 + (round / 10), 1 + (round / 10)), Asked(h));
            Assert.Equal(1 + (round / 10), h.Tree.EnableStorageCount);
        }

        h.Tree.FailEnableStorage = false;
        TimedRound(h); // the twentieth: five minutes after the second attempt
        Assert.Equal((3, 3), Asked(h));
        Assert.Equal(3, h.Tree.EnableStorageCount);
        Assert.Equal((1, 1), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));
    }

    [Fact]
    public void AnIdleDiskAfterAStallKeepsNothing()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);
        TimedRound(h);
        Assert.Equal(40, DriveTemperature(a, 0));
        Assert.True(DriveHeld(a, 0));

        // No round for more than two intervals: what the last one kept has expired, and an idle
        // round does not bring it back.
        h.Advance(61_000);
        for (int round = 0; round < 2; round++)
        {
            h.Hub.RunStorageDue();
            h.Advance(1000);
            h.Hub.RunDue();

            Assert.Equal([HddDrive("idle")], Drives(a));
            Assert.Null(DriveTemperature(a, 0));
            Assert.False(DriveHeld(a, 0));
            h.Advance(29_000);
        }

        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
    }

    [Fact]
    public void StorageOffAndNoSubscribersPerformNoActivityIo()
    {
        using Harness h = QuietHddHarness(out _, out IFeedSubscription sub);
        var before = (h.Activity.Reads, h.Activity.PowerQueries);

        // Storage off: the worker keeps waking for its schedule, and reads nothing.
        sub.Update(Requests.Of(1000, ServiceModules.Storage));
        for (int wake = 0; wake < 8; wake++)
        {
            h.Hub.RunDue();
            h.Hub.RunStorageDue();
            h.Advance(10_000);
        }

        Assert.Equal(before, (h.Activity.Reads, h.Activity.PowerQueries));

        // Switched on again: a round at once, and the worker is back on its schedule with a
        // baseline to take in twenty seconds.
        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();
        h.Time.Advance(h.Hub.RunStorageDue());
        Assert.Equal((before.Reads + 1, before.PowerQueries + 1), (h.Activity.Reads, h.Activity.PowerQueries));

        // Nobody subscribed: the worker sleeps, although that baseline is due.
        before = (h.Activity.Reads, h.Activity.PowerQueries);
        sub.Dispose();
        for (int wake = 0; wake < 8; wake++)
        {
            Assert.Equal(Timeout.InfiniteTimeSpan, h.Hub.RunStorageDue());
            h.Advance(10_000);
        }

        Assert.Equal(before, (h.Activity.Reads, h.Activity.PowerQueries));
    }

    [Fact]
    public void TheClosedGateAsksEachDriveOnceThenOnlyBlockersWithActivity()
    {
        using Harness h = GateHarness(wdcSpunDown: true, out List<FeedUpdate> a);
        IReadOnlyList<WireDrive> closed = [HddDrive("active"), WdcDrive("standby", blocksSmart: true)];
        Assert.Equal(closed, Drives(a));
        Assert.Equal((1, 1), Asked(h));

        // Nothing happens on either disk: nobody is asked again, and the answers stay.
        TimedRound(h);
        TimedRound(h);
        Assert.Equal((1, 1), Asked(h));
        Assert.Equal(closed, Drives(a));

        // The disk that answered "active" works: its answer is not renewed.
        TimedRound(h, () => h.Activity.Work(0));
        Assert.Equal((1, 1), Asked(h));

        // The blocker's counters grew: it alone is asked, and it still answers standby.
        TimedRound(h, () => h.Activity.Work(1));
        Assert.Equal((1, 2), Asked(h));
        Assert.Equal(closed, Drives(a));
        Assert.Equal(0, h.Tree.EnableStorageCount);
    }

    [Theory]
    [InlineData(false)] // its counters cannot be read
    [InlineData(true)] // no serial, so no key: nothing proves whose counters they are, and they are not read
    public void ABlockerWithoutCountersIsRetriedEveryFiveMinutes(bool keyless)
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Disks.Facts[1] = keyless ? WdcFacts() with { Serial = null } : WdcFacts();
        h.Disks.SpunDown[1] = true;
        h.Activity.Counters[0] = new DiskCounters(100, 100);
        if (!keyless)
        {
            h.Activity.Counters[1] = null;
        }

        h.Subscribe(1000);
        h.Hub.TickOnce();
        RoundAndTick(h);
        Assert.Equal((1, 1), Asked(h));

        // A round every thirty seconds: the blocker is asked again at the tenth and at the twentieth.
        for (int round = 1; round <= 20; round++)
        {
            TimedRound(h);
            Assert.Equal((1, 1 + (round / 10)), Asked(h));
        }

        Assert.Equal(0, h.Tree.EnableStorageCount);
        if (keyless)
        {
            Assert.Equal(0, h.Activity.ReadsOf(1));
        }
    }

    [Fact]
    public void ADriveThatWindowsTurnedOffBlocksTheGateWithoutACommand()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        h.Activity.Counters[0] = new DiskCounters(100, 100);
        h.Activity.Powered[0] = false;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();

        RoundAndTick(h);
        Assert.Equal([HddDrive("standby", blocksSmart: true)], Drives(a));
        Assert.Equal((0, 0), (h.Disks.SpunDownQueries, h.Tree.EnableStorageCount));

        TimedRound(h, () => h.Activity.Work(0)); // still off, whatever its counters say
        Assert.Equal([HddDrive("standby", blocksSmart: true)], Drives(a));
        Assert.Equal(0, h.Disks.SpunDownQueries);

        // Windows turned it on again, but nothing shows that it works: still not asked.
        h.Activity.Powered[0] = true;
        TimedRound(h);
        Assert.Equal([HddDrive("idle", blocksSmart: true)], Drives(a));
        Assert.Equal((0, 0), (h.Disks.SpunDownQueries, h.Tree.EnableStorageCount));

        TimedRound(h, () => h.Activity.Work(0));
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.EnableStorageCount));
        Assert.Equal(40, DriveTemperature(a, 0));
    }

    [Fact]
    public void TheGateRunsOneFullCheckBeforeOpening()
    {
        using Harness h = GateHarness(wdcSpunDown: true, out List<FeedUpdate> a);
        TimedRound(h);
        Assert.Equal((1, 1), Asked(h));

        // The blocker wakes up. Its answer is this round's; the other disk's is a minute old,
        // so that disk is asked once more before LHM touches every disk.
        h.Disks.SpunDown[1] = false;
        TimedRound(h, () => h.Activity.Work(1));

        Assert.Equal((2, 2), Asked(h));
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));
        Assert.Equal((1, 1), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));
    }

    [Fact]
    public void AStandbyFoundByTheFinalCheckKeepsTheGateClosed()
    {
        using Harness h = GateHarness(wdcSpunDown: true, out List<FeedUpdate> a);

        h.Disks.SpunDown[1] = false;
        h.Disks.SpunDown[0] = true; // it fell asleep after it answered "active"
        TimedRound(h, () => h.Activity.Work(1));

        Assert.Equal((2, 2), Asked(h));
        Assert.Equal(0, h.Tree.EnableStorageCount);
        Assert.Equal([HddDrive("standby", blocksSmart: true), WdcDrive("active")], Drives(a));

        // The episode goes on: the new blocker is asked only when it works, the other disk not at all.
        TimedRound(h);
        TimedRound(h, () => h.Activity.Work(1));
        Assert.Equal((2, 2), Asked(h));

        h.Disks.SpunDown[0] = false;
        TimedRound(h, () => h.Activity.Work(0));
        Assert.Equal((3, 3), Asked(h)); // its answer, then the full check once more
        Assert.Equal(1, h.Tree.EnableStorageCount);
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));
    }

    [Fact]
    public void AFailedPowerStateCallCountsAsOn()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        h.Activity.Counters[0] = new DiskCounters(100, 100);
        h.Activity.Powered[0] = null;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();

        RoundAndTick(h); // the gate asks it like a disk that is on
        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.EnableStorageCount));
        Assert.Equal([HddDrive("active")], Drives(a));

        TimedRound(h); // and afterwards the activity rule applies to it: idle, not standby
        Assert.Equal([HddDrive("idle")], Drives(a));

        TimedRound(h, () => h.Activity.Work(0));
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
    }
}
