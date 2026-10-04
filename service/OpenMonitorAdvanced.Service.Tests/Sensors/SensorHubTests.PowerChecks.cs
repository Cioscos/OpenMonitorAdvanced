using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

public sealed partial class SensorHubTests
{
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
    /// The next storage round as the worker runs it: the one sleep until it is due,
    /// <paramref name="meanwhile"/> at the end of that sleep, the round, then a sampling tick.
    /// </summary>
    private static void TimedRound(Harness h, Action? meanwhile = null)
    {
        (int reads, int asked) = (h.Activity.Reads, h.Disks.SpunDownQueries);
        h.Time.Advance(h.Hub.RunStorageDue()); // nothing is due: the worker sleeps until the round
        Assert.Equal((reads, asked), (h.Activity.Reads, h.Disks.SpunDownQueries)); // and that wake touched no disk
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
            // Thirty seconds to the next round, and nothing to watch: no counter is read.
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
    public void TheWorkerWakesOncePerRound()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd()); // a busy disk: its counters grow at every read
        h.Subscribe(1000);
        h.Hub.TickOnce();

        // The first round: its sample and, at its end, the reference of its successor. The
        // next wake is that successor.
        Assert.Equal(SensorHub.StorageInterval, h.Hub.RunStorageDue());
        Assert.Equal((2, 1, 1), (h.Activity.ReadsOf(0), h.Disks.EnumerateCalls, h.Disks.SpunDownQueries));

        // Woken before the round (a request, say): nothing is read, the reference stays the
        // one the round took.
        h.Advance(20_000);
        Assert.Equal(TimeSpan.FromSeconds(10), h.Hub.RunStorageDue());
        h.Advance(4_000);
        Assert.Equal(TimeSpan.FromSeconds(6), h.Hub.RunStorageDue());
        Assert.Equal((2, 1, 1), (h.Activity.ReadsOf(0), h.Disks.EnumerateCalls, h.Disks.SpunDownQueries));

        // The round samples again, before asking anything: the counters grew, so the disk is asked.
        h.Advance(6_000);
        Assert.Equal(SensorHub.StorageInterval, h.Hub.RunStorageDue());
        Assert.Equal((4, 2, 2), (h.Activity.ReadsOf(0), h.Disks.EnumerateCalls, h.Disks.SpunDownQueries));
    }

    [Fact]
    public void TheBaselineIsTakenAtTheEndOfARound()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);
        h.Advance(29_000);
        h.Activity.Work(0);

        // What the round had sent to the disk, and the schema revision, at each counter read.
        var reads = new List<(int Asked, int Updates, int Revision)>();
        h.Activity.OnRead = _ =>
        {
            h.Hub.TickOnce();
            reads.Add((h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0"), h.Hub.Revision));
        };

        // One wake does the whole round: the sample before any command, and the reference
        // after the round's question and its SMART read. No wake in between.
        int revision = h.Hub.Revision;
        int enumerations = h.Disks.EnumerateCalls;
        Assert.Equal(SensorHub.StorageInterval, h.Hub.RunStorageDue());
        Assert.Equal([(1, 1, revision), (2, 2, revision)], reads);
        Assert.Equal(enumerations + 1, h.Disks.EnumerateCalls); // the reference reads the counters and nothing else

        // A round that asks nothing takes it too, and before it is published: at that read a
        // tick still sees the round before (the disk's new state has not bumped the revision).
        reads.Clear();
        h.Advance(30_000);
        Assert.Equal(SensorHub.StorageInterval, h.Hub.RunStorageDue());
        Assert.Equal([(2, 2, revision), (2, 2, revision)], reads);
        h.Activity.OnRead = null;
        h.Advance(1_000);
        h.Hub.RunDue();
        Assert.Equal(revision + 1, h.Hub.Revision);
        Assert.Equal([HddDrive("idle")], Drives(a));

        // So work right after that round is seen by the next one.
        h.Activity.Work(0);
        h.Advance(29_000);
        Assert.Equal(SensorHub.StorageInterval, h.Hub.RunStorageDue());
        Assert.Equal(3, h.Disks.SpunDownQueries);
    }

    [Fact]
    public void IoAnywhereBetweenTwoRoundsIsActivity()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);

        // A short burst one second after the round ended, and nothing for the rest of the
        // interval: the next round sees it, asks the disk and reads it.
        h.Activity.Work(0);
        TimedRound(h);
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal(50, DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        // In the middle of the interval too.
        h.Advance(14_000);
        h.Activity.Work(0);
        TimedRound(h);
        Assert.Equal((3, 3), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));

        // And once only: the round after it has nothing new to see.
        TimedRound(h);
        Assert.Equal([HddDrive("idle")], Drives(a));
        Assert.Equal((3, 3), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));

        // The closed gate uses the same reference for its blockers: one that worked right
        // after a gate round is asked by the next.
        using Harness gate = GateHarness(wdcSpunDown: true, out _);
        gate.Activity.Work(1);
        TimedRound(gate);
        Assert.Equal((1, 2), Asked(gate));
        TimedRound(gate);
        Assert.Equal((1, 2), Asked(gate));
    }

    [Fact]
    public void TheServicesOwnQueriesAreNotActivity()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);

        // The SMART read of a round moves the disk's counters (LHM reads through the driver).
        h.Tree.BeforeUpdate = root =>
        {
            if (root.Type == HardwareType.Storage)
            {
                h.Activity.Work(0);
            }
        };
        TimedRound(h, () => h.Activity.Work(0));
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));

        // That growth came before the reference, so it is nobody's activity: left alone, the
        // disk is idle in every round that follows instead of being asked for ever.
        for (int round = 0; round < 3; round++)
        {
            TimedRound(h);
            Assert.Equal([HddDrive("idle")], Drives(a));
            Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
            Assert.Equal(50, DriveTemperature(a, 0));
            Assert.True(DriveHeld(a, 0));
        }
    }

    [Fact]
    public void AFailedRoundLeavesNoBaseline()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);

        // The disk works, so the round asks it; then the round fails before its end.
        h.Time.Advance(h.Hub.RunStorageDue());
        h.Activity.Work(0);
        h.Tree.ThrowOnNextRoots();
        Assert.Throws<InvalidOperationException>(() => h.Hub.RunStorageDue());
        Assert.Equal((2, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));

        // The retry, at once, and the disk worked again meanwhile: there is no reference to
        // compare with (the failed round took none, and used up the one before it, which
        // would still be fresh enough), so no activity, no question.
        h.Advance(1_000);
        h.Activity.Work(0);
        RoundAndTick(h);
        Assert.Equal([HddDrive("idle")], Drives(a));
        Assert.Equal((2, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));

        // That round ended, so the next one sees work again.
        TimedRound(h, () => h.Activity.Work(0));
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal((3, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
    }

    [Fact]
    public void ABaselineFromAnotherIdentityOrBeforeSuspendIsNotUsed()
    {
        using Harness h = QuietHddHarness(out _, out _);
        h.Activity.Counters[1] = new DiskCounters(200, 200);

        // A disk plugged in after the reference renumbers the drives: the counters read at a
        // number before may be another disk's, so no reference counts for any drive. (The new
        // disk itself is asked once, as every newly listed one.)
        TimedRound(h, () =>
        {
            h.Disks.Facts[1] = WdcFacts();
            h.Activity.Work(0);
        });
        Assert.Equal((1, 1), Asked(h));

        // The system slept between the two rounds, briefly enough for the round before to
        // stay current: the reference is older than one interval, so it does not count.
        TimedRound(h, () =>
        {
            h.Advance(20_000);
            h.Activity.Work(0);
        });
        Assert.Equal((1, 1), Asked(h));
        Assert.Equal(1, h.Tree.Updates("/hdd/0"));

        // A longer sleep expires the round before: each disk gets the one question of a new
        // episode, with or without growth, and not for the old reference.
        TimedRound(h, () => h.Advance(600_000));
        Assert.Equal((2, 2), Asked(h));
        Assert.Equal(2, h.Tree.Updates("/hdd/0"));

        // The same disks, on time: used.
        TimedRound(h, () => h.Activity.Work(0));
        Assert.Equal((3, 2), Asked(h));
    }

    [Fact]
    public void ALateBaselineDoesNotAuthorizeSmart()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);

        // The round comes later after the one before than one interval and its tolerance:
        // the reference is too old to place the growth.
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

        // Its one question was not kept for later; it is asked when Windows turns it on again.
        TimedRound(h, () => h.Activity.Work(0));
        Assert.Equal(1, h.Disks.SpunDownQueries);
        h.Activity.Powered[0] = true;
        TimedRound(h);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("active")], Drives(b));
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

    /// <summary>A storage round that comes <paramref name="ms"/> after the one before, then a sampling tick.</summary>
    private static void RoundAfter(Harness h, int ms)
    {
        h.Advance(ms - 1000);
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
    }

    [Fact]
    public void AnIdleDiskAfterAStallKeepsNothing()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);
        TimedRound(h);
        Assert.Equal(40, DriveTemperature(a, 0));
        Assert.True(DriveHeld(a, 0));

        // No round for more than two intervals: what the last one kept has expired. The late
        // round asks the disk its one question again; it rests, and gets nothing back from the
        // stale round.
        h.Disks.SpunDown[0] = true;
        RoundAfter(h, 62_000);
        Assert.Equal([HddDrive("standby")], Drives(a));
        Assert.Null(DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));
        Assert.Equal((2, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));

        // The rounds after it are on time: no question, and still nothing to bring back.
        for (int round = 0; round < 2; round++)
        {
            TimedRound(h);
            Assert.Null(DriveTemperature(a, 0));
            Assert.False(DriveHeld(a, 0));
        }

        Assert.Equal((2, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
    }

    [Fact]
    public void AfterAResumeAnActiveHddIsReadOnce()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);
        TimedRound(h);
        Assert.Equal([HddDrive("idle")], Drives(a));
        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));

        // The PC slept for an hour. The disk spins and nothing works on it: without its one
        // question it would show no SMART value until it next works.
        RoundAfter(h, 3_600_000);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal(50, DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        // Once: from the next round on it takes activity, and the values read are kept.
        TimedRound(h);
        TimedRound(h);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("idle")], Drives(a));
        Assert.Equal(50, DriveTemperature(a, 0));
        Assert.True(DriveHeld(a, 0));
    }

    [Fact]
    public void AResumeRightAfterAnEpisodeStartIsAnExpiryOfItsOwn()
    {
        // Storage switched off and on: the round at once has no round before it, which is an
        // episode start and not a late round. A suspension right after it is the first expiry.
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out IFeedSubscription sub);
        h.Advance(120_000);
        SwitchStorageOffAndOn(h, sub);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));

        RoundAfter(h, 120_000); // sooner than a second of two late rounds in a row would be asked
        Assert.Equal((3, 3), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("active")], Drives(a));
    }

    [Fact]
    public void ALateRoundAsksEachWatchedDiskOnce()
    {
        using Harness h = GateHarness(wdcSpunDown: false, out List<FeedUpdate> a);
        TimedRound(h);
        Assert.Equal((1, 1), Asked(h));
        Assert.Equal([HddDrive("idle"), WdcDrive("idle")], Drives(a));

        // A round later than two intervals: each disk Windows reports on is asked once and,
        // active, updated.
        RoundAfter(h, 61_000);
        Assert.Equal((2, 2), Asked(h));
        Assert.Equal((2, 2), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));
        Assert.Equal([HddDrive("active"), WdcDrive("active")], Drives(a));

        TimedRound(h);
        TimedRound(h);
        Assert.Equal((2, 2), Asked(h));
        Assert.Equal([HddDrive("idle"), WdcDrive("idle")], Drives(a));

        // Windows comes first after a late round too: the disk it turned off is sent nothing.
        h.Activity.Powered[1] = false;
        RoundAfter(h, 61_000);
        Assert.Equal((3, 2), Asked(h));
        Assert.Equal((3, 2), (h.Tree.Updates("/hdd/0"), h.Tree.Updates("/hdd/1")));
        Assert.Equal([HddDrive("active"), WdcDrive("standby")], Drives(a));

        // Its question was not kept for later rounds: it takes Windows turning it on again.
        TimedRound(h);
        Assert.Equal((3, 2), Asked(h));
    }

    [Fact]
    public void ConsecutiveLateRoundsDoNotAskAtEveryRound()
    {
        using Harness h = QuietHddHarness(out _, out _);

        // A worker that only manages a round every ninety seconds: every round is late. The
        // disk is asked at the first one, then at most every five minutes, so that Windows
        // can still turn it off.
        int[] asked = new int[9];
        for (int round = 0; round < asked.Length; round++)
        {
            RoundAfter(h, 90_000);
            asked[round] = h.Disks.SpunDownQueries;
        }

        // Asked at 0 s, then at 360 s (the first round five minutes later), then at 720 s.
        Assert.Equal([2, 2, 2, 2, 3, 3, 3, 3, 4], asked);
        Assert.Equal(GateEpisode.BlindRetry, TimeSpan.FromMinutes(5));

        // Rounds on time again, then one late round: that is a new expiry, asked at once.
        TimedRound(h);
        TimedRound(h);
        Assert.Equal(4, h.Disks.SpunDownQueries);
        RoundAfter(h, 61_000);
        Assert.Equal(5, h.Disks.SpunDownQueries);
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

        // Switched on again: a round at once (its sample and its reference), and the worker is
        // back on its schedule.
        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();
        h.Time.Advance(h.Hub.RunStorageDue());
        Assert.Equal((before.Reads + 2, before.PowerQueries + 1), (h.Activity.Reads, h.Activity.PowerQueries));

        // Nobody subscribed: the worker sleeps, although a round is due.
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

        // Windows turned it on again: that round asks it, and the gate opens on its answer.
        h.Activity.Powered[0] = true;
        TimedRound(h);
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

    [Fact]
    public void AFailingRoundAfterReenableAsksEachDiskOnlyOnce()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out IFeedSubscription sub);
        sub.Update(Requests.Of(1000, ServiceModules.Storage));
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();

        // The round at once lists the drives and asks the disk, then fails before it can
        // publish anything; the loop retries it every thirty seconds.
        h.Tree.ThrowOnRoots = true;
        for (int retry = 0; retry < 4; retry++)
        {
            Assert.Throws<InvalidOperationException>(() => h.Hub.RunStorageDue());
            Assert.Equal(2, h.Disks.SpunDownQueries); // one question in all: the retries do not repeat it
            h.Time.Advance(SensorHub.FailureRetryDelay);
        }

        h.Tree.ThrowOnRoots = false;
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal((2, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("idle")], Drives(a));
    }

    [Fact]
    public void ARoundInFlightWhenTheLastClientLeavesDoesNotEraseTheNextEpisodeStart()
    {
        using Harness h = QuietHddHarness(out _, out IFeedSubscription sub);

        // The last client leaves while a round is updating the disk; that round still ends
        // and publishes after the hub went idle.
        h.Tree.BeforeUpdate = root =>
        {
            if (root.Type == HardwareType.Storage)
            {
                sub.Dispose();
            }
        };
        TimedRound(h, () => h.Activity.Work(0));
        h.Tree.BeforeUpdate = null;
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));

        // The client that comes back starts an episode all the same: the quiet disk is read once.
        h.Advance(5_000);
        List<FeedUpdate> b = h.Subscribe(1000);
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal((3, 3), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("active")], Drives(b));
        Assert.Equal(50, DriveTemperature(b, 0));
    }

    [Fact]
    public void ADiskWhoseSmartIsSwitchedOnIsAskedOnce()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out IFeedSubscription sub);
        sub.Update(Requests.Of(1000, ServiceModules.None, HddKey));
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal([HddDrive("smartOff")], Drives(a));
        Assert.Equal(1, h.Disks.SpunDownQueries);

        // Storage stays on; only this disk's SMART comes back: it is watched anew.
        sub.Update(Requests.Of(1000));
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal(50, DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        TimedRound(h);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("idle")], Drives(a));
        Assert.Equal(50, DriveTemperature(a, 0));
        Assert.True(DriveHeld(a, 0));
    }

    [Fact]
    public void ANewlyListedHddIsAskedOnce()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);
        h.Activity.Counters[1] = new DiskCounters(200, 200);

        // Plugged in: asked in the first round that lists it, without any activity.
        h.Disks.Facts[1] = WdcFacts();
        TimedRound(h);
        Assert.Equal((1, 1), Asked(h));
        Assert.Equal([HddDrive("idle"), WdcDrive("active")], Drives(a));

        TimedRound(h);
        Assert.Equal((1, 1), Asked(h));
        Assert.Equal([HddDrive("idle"), WdcDrive("idle")], Drives(a));

        // Another disk at the same number is new as well.
        h.Disks.Facts[1] = WdcFacts() with { Serial = "WD-WCC7K0000002" };
        TimedRound(h);
        Assert.Equal((1, 2), Asked(h));
        TimedRound(h);
        Assert.Equal((1, 2), Asked(h));
    }

    [Fact]
    public void ADiskThatWindowsTurnsBackOnIsAskedThatRound()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);
        h.Activity.Powered[0] = false;
        TimedRound(h);
        Assert.Equal([HddDrive("standby")], Drives(a));
        Assert.Equal(1, h.Disks.SpunDownQueries);

        // Windows powers a disk up for I/O: the transition counts as activity, whatever the
        // counters say.
        h.Activity.Powered[0] = true;
        TimedRound(h);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal(50, DriveTemperature(a, 0));
        Assert.False(DriveHeld(a, 0));

        TimedRound(h); // that round only
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("idle")], Drives(a));
    }

    [Fact]
    public void ABlockerThatWindowsTurnsBackOnIsAskedWithoutCounters()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        h.Activity.Counters[0] = new DiskCounters(100, 100); // readable, and standing still throughout
        h.Activity.Powered[0] = false;
        List<FeedUpdate> a = h.Subscribe(1000);
        h.Hub.TickOnce();
        RoundAndTick(h);
        TimedRound(h);
        Assert.Equal([HddDrive("standby", blocksSmart: true)], Drives(a));
        Assert.Equal(0, h.Disks.SpunDownQueries);

        h.Activity.Powered[0] = true;
        TimedRound(h);
        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.EnableStorageCount));
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal(40, DriveTemperature(a, 0));
    }

    [Fact]
    public void ANullPowerStateAfterOffSendsNothing()
    {
        // Gate open: Windows reported the disk off, then the call fails. The disk is still
        // off as far as anyone knows: standby, values kept, nothing sent, whatever its counters say.
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out IFeedSubscription sub);
        h.Activity.Powered[0] = false;
        TimedRound(h, () => h.Activity.Work(0));
        h.Activity.Powered[0] = null;
        for (int round = 0; round < 2; round++)
        {
            TimedRound(h, () => h.Activity.Work(0));
            Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
            Assert.Equal([HddDrive("standby")], Drives(a));
            Assert.Equal(40, DriveTemperature(a, 0));
            Assert.True(DriveHeld(a, 0));
        }

        // Not even when it is watched anew: storage switched off and on, a client that comes back.
        SwitchStorageOffAndOn(h, sub);
        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("standby")], Drives(a));
        sub.Dispose();
        h.Advance(5_000);
        List<FeedUpdate> b = h.Subscribe(1000);
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();
        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("standby")], Drives(b));

        // Closed gate: the same reading keeps the drive a blocker that is not asked.
        using var gate = new Harness();
        gate.Tree.Initial.Add(Cpu());
        gate.Tree.Storage.Add(Hdd());
        gate.Activity.Counters[0] = null; // not even the five-minute retry of a drive without counters
        gate.Activity.Powered[0] = false;
        List<FeedUpdate> c = gate.Subscribe(1000);
        gate.Hub.TickOnce();
        RoundAndTick(gate);
        gate.Activity.Powered[0] = null;
        for (int round = 0; round < 12; round++)
        {
            gate.Advance(30_000);
            RoundAndTick(gate);
        }

        Assert.Equal((0, 0), (gate.Disks.SpunDownQueries, gate.Tree.EnableStorageCount));
        Assert.Equal([HddDrive("standby", blocksSmart: true)], Drives(c));
    }

    [Fact]
    public void OffThenNullThenOnIsAskedOnce()
    {
        using Harness h = QuietHddHarness(out List<FeedUpdate> a, out _);
        h.Activity.Powered[0] = false;
        TimedRound(h);
        h.Activity.Powered[0] = null;
        TimedRound(h);
        Assert.Equal(1, h.Disks.SpunDownQueries);

        // The first real "on" after the "off" is the transition, although a failed call lay between.
        h.Activity.Powered[0] = true;
        TimedRound(h);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("active")], Drives(a));
        Assert.Equal(50, DriveTemperature(a, 0));

        TimedRound(h);
        Assert.Equal((2, 2), (h.Disks.SpunDownQueries, h.Tree.Updates("/hdd/0")));
        Assert.Equal([HddDrive("idle")], Drives(a));
    }

    [Fact]
    public void ABlockerTurnedOnAcrossAnIdleHubIsAsked()
    {
        using var h = new Harness();
        h.Tree.Initial.Add(Cpu());
        h.Tree.Storage.Add(Hdd());
        h.Tree.Values[HddTemp] = 40;
        h.Activity.Counters[0] = new DiskCounters(100, 100); // readable, and standing still throughout
        h.Activity.Powered[0] = false;
        h.Subscribe(1000, out IFeedSubscription sub);
        h.Hub.TickOnce();
        RoundAndTick(h);
        Assert.Equal((0, 0), (h.Disks.SpunDownQueries, h.Tree.EnableStorageCount));

        // Everybody leaves, Windows turns the disk on, a client comes back.
        sub.Dispose();
        h.Advance(60_000);
        h.Activity.Powered[0] = true;
        List<FeedUpdate> b = h.Subscribe(1000);
        h.Hub.RunDue();
        h.Hub.RunStorageDue();
        h.Advance(1000);
        h.Hub.RunDue();

        Assert.Equal((1, 1), (h.Disks.SpunDownQueries, h.Tree.EnableStorageCount));
        Assert.Equal([HddDrive("active")], Drives(b));
        Assert.Equal(40, DriveTemperature(b, 0));
    }
}
