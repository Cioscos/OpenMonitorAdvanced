using System.Runtime.InteropServices;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// <see cref="DiskActivity"/>: when a disk's counters prove recent activity (design M6b §4.4),
/// what a round does with a disk whose power mode matters, and the layout of the struct the
/// counters are read into.
/// </summary>
public sealed class DiskActivityTests
{
    private static readonly DriveFacts Hdd = new(0, DriveAvailability.Present, "ST2000DM008-2FR102", "ZFL0", BusType: 0x0B, SeekPenalty: true);

    [Fact]
    public void ACounterThatGrewIsActivity()
    {
        Assert.True(DiskActivity.Between(new DiskCounters(10, 20), new DiskCounters(11, 20)));
        Assert.True(DiskActivity.Between(new DiskCounters(10, 20), new DiskCounters(10, 21)));
        Assert.True(DiskActivity.Between(new DiskCounters(10, 20), new DiskCounters(15, 29)));
        Assert.False(DiskActivity.Between(new DiskCounters(10, 20), new DiskCounters(10, 20)));
    }

    [Fact]
    public void AMissingBaselineIsNotActivity()
    {
        Assert.False(DiskActivity.Between(null, new DiskCounters(10, 20)));
        Assert.False(DiskActivity.Between(new DiskCounters(10, 20), null));
        Assert.False(DiskActivity.Between(null, null));
    }

    [Fact]
    public void ACounterThatWentBackIsNotActivity()
    {
        Assert.False(DiskActivity.Between(new DiskCounters(10, 20), new DiskCounters(9, 20)));
        Assert.False(DiskActivity.Between(new DiskCounters(10, 20), new DiskCounters(10, 0)));

        // Not even when the other one grew: the counters were reset, so the samples do not compare.
        Assert.False(DiskActivity.Between(new DiskCounters(10, 20), new DiskCounters(11, 19)));
        Assert.False(DiskActivity.Between(new DiskCounters(10, 20), new DiskCounters(9, 21)));
    }

    [Fact]
    public void DiskPerformanceIsEightyEightBytes()
    {
        Assert.Equal(88, Marshal.SizeOf<DiskActivityProbe.NativeMethods.DiskPerformance>());
        Assert.Equal(40, (int)Marshal.OffsetOf<DiskActivityProbe.NativeMethods.DiskPerformance>(nameof(DiskActivityProbe.NativeMethods.DiskPerformance.ReadCount)));
        Assert.Equal(44, (int)Marshal.OffsetOf<DiskActivityProbe.NativeMethods.DiskPerformance>(nameof(DiskActivityProbe.NativeMethods.DiskPerformance.WriteCount)));
        Assert.Equal(0x00070020u, DiskActivityProbe.NativeMethods.IoctlDiskPerformance);
        Assert.Equal(TimeSpan.FromSeconds(10), DiskActivity.Window);
    }

    [Fact]
    public void ADiskIsAskedOnlyWhenWindowsHasItOnAndItWorkedRecently()
    {
        int asked = 0;
        bool? Ask(DriveFacts drive)
        {
            asked++;
            return true;
        }

        // Windows turned it off: standby without a command, whatever the counters say.
        DriveCheck off = DiskActivity.Check(Hdd, new DriveActivity(PoweredOff: true, Readable: true, Recent: true), first: false, Ask);
        Assert.Equal((false, true, false, 0), (off.Asked, off.PoweredOff, off.Idle, asked));

        // Not even in the first round of a storage episode.
        off = DiskActivity.Check(Hdd, new DriveActivity(PoweredOff: true, Readable: true, Recent: false), first: true, Ask);
        Assert.Equal((false, true, false, 0), (off.Asked, off.PoweredOff, off.Idle, asked));

        // On, no recent activity: idle, no command.
        DriveCheck idle = DiskActivity.Check(Hdd, new DriveActivity(PoweredOff: false, Readable: true, Recent: false), first: false, Ask);
        Assert.Equal((false, false, true, 0), (idle.Asked, idle.PoweredOff, idle.Idle, asked));

        // On and working: asked, and its answer is the round's.
        DriveCheck working = DiskActivity.Check(Hdd, new DriveActivity(PoweredOff: false, Readable: true, Recent: true), first: false, Ask);
        Assert.Equal((true, (bool?)true, false, false, 1), (working.Asked, working.SpunDown, working.PoweredOff, working.Idle, asked));

        // On, in the first round of a storage episode: asked once without activity.
        DriveCheck starting = DiskActivity.Check(Hdd, new DriveActivity(PoweredOff: false, Readable: false, Recent: false), first: true, Ask);
        Assert.Equal((true, (bool?)true, false, false, 2), (starting.Asked, starting.SpunDown, starting.PoweredOff, starting.Idle, asked));
    }
}
