using System.Runtime.InteropServices;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// The pure parts of <see cref="DiskPowerProbe"/>: the ATA CHECK POWER MODE interpretation,
/// the D6 "every rotational disk is active" rule and the marshalled size of every FFI struct
/// (x64, this project's only <c>RuntimeIdentifier</c>). The IOCTLs themselves need an elevated
/// process and a real disk: they are exercised in Task 15.
/// </summary>
public sealed class DiskPowerProbeTests
{
    [Fact]
    public void InterpretsCheckPowerMode()
    {
        Assert.True(DiskPowerProbe.InterpretCheckPowerMode(0x00));
        Assert.True(DiskPowerProbe.InterpretCheckPowerMode(0x01));
        foreach (byte active in new byte[] { 0x40, 0x41, 0x80, 0x81, 0x82, 0x83, 0xFF })
        {
            Assert.False(DiskPowerProbe.InterpretCheckPowerMode(active));
        }

        Assert.Null(DiskPowerProbe.InterpretCheckPowerMode(0x12));
    }

    [Fact]
    public void AnAbortedCommandIsUnknown()
    {
        // Status ERR (bit 0) set: the drive rejected CHECK POWER MODE, the sector count is meaningless.
        Assert.Null(DiskPowerProbe.InterpretAtaResult(status: 0x51, sectorCount: 0x00));
        Assert.True(DiskPowerProbe.InterpretAtaResult(status: 0x50, sectorCount: 0x00));
        Assert.False(DiskPowerProbe.InterpretAtaResult(status: 0x50, sectorCount: 0xFF));
    }

    [Fact]
    public void ParsesTheSeekPenaltyDescriptor()
    {
        // DEVICE_SEEK_PENALTY_DESCRIPTOR: Version (4), Size (4), IncursSeekPenalty (BOOLEAN at offset 8).
        byte[] hdd = [12, 0, 0, 0, 12, 0, 0, 0, 1, 0, 0, 0];
        byte[] ssd = [12, 0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0];
        Assert.True(DiskPowerProbe.ParseSeekPenalty(hdd));
        Assert.False(DiskPowerProbe.ParseSeekPenalty(ssd));
        Assert.Null(DiskPowerProbe.ParseSeekPenalty(hdd.AsSpan(0, 8)));
    }

    private static DriveFacts Drive(int n, uint? bus, bool? seekPenalty, DriveAvailability availability = DriveAvailability.Present, string? model = "Model") =>
        new(n, availability, model, null, bus, seekPenalty);

    /// <summary>Drives the rule would ask (a callback that records and answers "unknown", i.e. would block).</summary>
    private static (IReadOnlyList<DriveCheck> Blockers, List<int> Asked) Gate(params DriveFacts[] drives)
    {
        var asked = new List<int>();
        IReadOnlyList<DriveCheck> checks = DiskPowerProbe.CheckDrives(drives, drive =>
        {
            asked.Add(drive.DriveNumber);
            return null;
        });

        // Every drive is reported, in order, asked or not.
        Assert.Equal(drives, checks.Select(c => c.Drive));
        Assert.Equal(asked, checks.Where(c => c.Asked).Select(c => c.Drive.DriveNumber));
        return (Blockers(checks), asked);
    }

    private static IReadOnlyList<DriveCheck> Blockers(IEnumerable<DriveCheck> checks) => [.. checks.Where(c => c.Blocks)];

    [Fact]
    public void ADriveWithoutMediaDoesNotBlockTheGate()
    {
        (IReadOnlyList<DriveCheck> blockers, List<int> asked) = Gate(Drive(4, bus: null, seekPenalty: null, DriveAvailability.NoMedia));
        Assert.Empty(blockers);
        Assert.Empty(asked);
    }

    [Fact]
    public void VirtualAndStorageSpacesDisksDoNotBlockTheGate()
    {
        // STORAGE_BUS_TYPE BusTypeVirtual (0xE), BusTypeFileBackedVirtual (0xF): VHD/VHDX, ramdisks;
        // BusTypeSpaces (0x10): a Storage Spaces virtual disk (ruling R19).
        (IReadOnlyList<DriveCheck> blockers, List<int> asked) = Gate(Drive(5, bus: 0x0E, seekPenalty: null), Drive(6, bus: 0x0F, seekPenalty: true), Drive(8, bus: 0x10, seekPenalty: null));
        Assert.Empty(blockers);
        Assert.Empty(asked);
    }

    [Fact]
    public void NvmeDoesNotBlockTheGate()
    {
        (IReadOnlyList<DriveCheck> blockers, List<int> asked) = Gate(Drive(2, bus: 0x11, seekPenalty: null));
        Assert.Empty(blockers);
        Assert.Empty(asked);
    }

    [Fact]
    public void ASolidStateDriveDoesNotBlockTheGate()
    {
        (IReadOnlyList<DriveCheck> blockers, List<int> asked) = Gate(Drive(1, bus: 0x0B, seekPenalty: false));
        Assert.Empty(blockers);
        Assert.Empty(asked);
    }

    [Fact]
    public void RotationalUnknownAndUnreadableDrivesMustBeActive()
    {
        DriveFacts hdd = Drive(0, bus: 0x0B, seekPenalty: true);
        DriveFacts unknown = Drive(3, bus: 0x07, seekPenalty: null); // e.g. USB, seek penalty not answered
        DriveFacts unreadable = Drive(7, bus: null, seekPenalty: null, DriveAvailability.Unreadable);

        foreach (DriveFacts drive in new[] { hdd, unknown, unreadable })
        {
            Assert.Equal([new DriveCheck(drive, Asked: true, SpunDown: false)], DiskPowerProbe.CheckDrives([drive], _ => false));
            DriveCheck standby = Assert.Single(Blockers(DiskPowerProbe.CheckDrives([drive], _ => true)));
            Assert.Equal((drive, (bool?)true), (standby.Drive, standby.SpunDown));
            DriveCheck unanswered = Assert.Single(Blockers(DiskPowerProbe.CheckDrives([drive], _ => null)));
            Assert.Null(unanswered.SpunDown);
        }
    }

    [Fact]
    public void OnlyTheDrivesThatNeedItAreAskedForTheirPowerMode()
    {
        var asked = new List<int>();
        var probe = new DiskPowerProbe(
            () => [Drive(0, 0x0B, true), Drive(1, 0x0B, false), Drive(2, 0x11, null), Drive(3, 0x0E, null), Drive(4, null, null, DriveAvailability.NoMedia)],
            n =>
            {
                asked.Add(n);
                return false;
            });

        Assert.True(probe.AllRotationalDisksActive());
        Assert.Equal([0], asked);
    }

    [Fact]
    public void TheGateReportsEveryDriveAndTheEnumerationAsksNothing()
    {
        var asked = new List<int>();
        DriveFacts[] drives = [Drive(0, 0x0B, true), Drive(2, 0x11, null), Drive(4, DriveFacts.BusTypeUsb, null)];
        var probe = new DiskPowerProbe(
            () => drives,
            n =>
            {
                asked.Add(n);
                return n == 4 ? null : false;
            });

        Assert.Equal(drives, probe.Enumerate());
        Assert.Empty(asked);

        Assert.Equal(
            [
                new DriveCheck(drives[0], Asked: true, SpunDown: false),
                new DriveCheck(drives[1], Asked: false, SpunDown: null), // NVMe: nothing to wake
                new DriveCheck(drives[2], Asked: true, SpunDown: null), // a USB disk is asked too: the first identification touches it
            ],
            probe.CheckGate());
        Assert.Equal([0, 4], asked);
        Assert.False(probe.AllRotationalDisksActive());
    }

    [Fact]
    public void TheEnumerationAloneForgetsTheRouteOfADriveThatIsGone()
    {
        // After the gate opened the hub only enumerates: the route memory must follow that too.
        var routes = new Routes { Native = null, Sat = false };
        routes.Drives.Add(UsbStick());
        DiskPowerProbe probe = routes.Probe();
        Assert.False(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.False(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Single(routes.NativeAsked); // SAT is remembered

        Assert.Single(probe.Enumerate());
        Assert.False(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Single(routes.NativeAsked); // still there: the same disk

        routes.Drives.Clear();
        Assert.Empty(probe.Enumerate());
        routes.Drives.Add(UsbStick());
        Assert.False(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Equal(2, routes.NativeAsked.Count); // unplugged in between: asked from scratch
    }

    [Fact]
    public void TheBlockingDrivesAreLoggedOncePerChange()
    {
        var log = new ListLogger<DiskPowerProbe>();
        var drives = new List<DriveFacts> { Drive(0, 0x0B, true, model: "ST2000DM008-2FR102") };
        var probe = new DiskPowerProbe(() => drives, _ => true, log);

        for (int i = 0; i < 3; i++)
        {
            Assert.False(probe.AllRotationalDisksActive());
        }

        LogEntry first = Assert.Single(log.Entries, e => e.Level == LogLevel.Information && e.Message.Contains("PhysicalDrive0", StringComparison.Ordinal));
        Assert.Contains("ST2000DM008-2FR102", first.Message, StringComparison.Ordinal);
        Assert.Contains("0x0B", first.Message, StringComparison.Ordinal);

        drives.Add(Drive(3, 0x07, null, model: "USB Stick"));
        Assert.False(probe.AllRotationalDisksActive());
        Assert.False(probe.AllRotationalDisksActive());

        Assert.Single(log.Entries, e => e.Level == LogLevel.Information && e.Message.Contains("PhysicalDrive3", StringComparison.Ordinal));
        Assert.Equal(2, log.Entries.Count(e => e.Level == LogLevel.Information && e.Message.Contains("PhysicalDrive0", StringComparison.Ordinal)));
    }

    [Fact]
    public void Win32ErrorsAreLoggedOncePerChangeAndResetOnSuccess()
    {
        var log = new ListLogger<DiskPowerProbe>();
        var errors = new DiskPowerProbe.Win32ErrorLog(log);

        errors.Failed(0, "open", 5);
        errors.Failed(0, "open", 5);
        Assert.Single(log.Entries);

        errors.Failed(0, "ioctl", 5); // another operation
        Assert.Equal(2, log.Entries.Count);

        errors.Succeeded(0, "open");
        errors.Failed(0, "open", 5); // same error again after a success: logged again
        Assert.Equal(3, log.Entries.Count);
        Assert.All(log.Entries, e => Assert.Equal(LogLevel.Warning, e.Level));
    }

    [Fact]
    public void AnEmptyReplyIsNotLoggedAsWin32ErrorZero()
    {
        var log = new ListLogger<DiskPowerProbe>();
        var errors = new DiskPowerProbe.Win32ErrorLog(log);

        errors.EmptyReply(0, "ioctl");
        errors.EmptyReply(0, "ioctl");
        LogEntry entry = Assert.Single(log.Entries);
        Assert.Contains("empty reply", entry.Message, StringComparison.Ordinal);
        Assert.DoesNotContain("Win32 error", entry.Message, StringComparison.Ordinal);

        errors.Succeeded(0, "ioctl");
        errors.EmptyReply(0, "ioctl");
        Assert.Equal(2, log.Entries.Count);
    }

    [Fact]
    public void ANegativeDriveNumberIsUnknown()
    {
        var probe = new DiskPowerProbe(() => [Drive(0, 0x0B, true)], _ => false);
        Assert.Null(probe.IsSpunDown(-1, "Model", null));
        Assert.Null(probe.Describe(-1));
        Assert.Equal(Drive(0, 0x0B, true), probe.Describe(0));
        Assert.Null(probe.Describe(9));
    }

    /// <summary>A probe over scripted answers of the two routes, recording which drive each one was asked for.</summary>
    private sealed class Routes
    {
        public bool? Native { get; set; }

        public bool? Sat { get; set; }

        public List<int> NativeAsked { get; } = [];

        public List<int> SatAsked { get; } = [];

        public List<DriveFacts> Drives { get; } = [];

        public FakeTimeProvider Time { get; } = new();

        public DiskPowerProbe Probe() => new(
            () => [.. Drives],
            n =>
            {
                NativeAsked.Add(n);
                return Native;
            },
            n =>
            {
                SatAsked.Add(n);
                return Sat;
            },
            Time);
    }

    private static DriveFacts UsbStick(string? model = "Extreme", string? serial = "4C53") =>
        new(4, DriveAvailability.Present, model, serial, BusType: 0x07, SeekPenalty: null);

    [Fact]
    public void SatIsTriedWhenTheNativeCommandGivesNoAnswer()
    {
        var routes = new Routes { Native = null, Sat = false };

        Assert.False(routes.Probe().IsSpunDown(4, "Extreme", "4C53"));
        Assert.Equal([4], routes.NativeAsked);
        Assert.Equal([4], routes.SatAsked);
    }

    [Fact]
    public void ANativeStandbyAnswerDoesNotTryTheFallback()
    {
        var routes = new Routes { Native = true, Sat = false };

        Assert.True(routes.Probe().IsSpunDown(0, "ST2000DM008-2FR102", "ZFL0"));
        Assert.Equal([0], routes.NativeAsked);
        Assert.Empty(routes.SatAsked);
    }

    [Fact]
    public void TheWorkingRouteIsRememberedPerDrive()
    {
        var routes = new Routes { Native = null, Sat = false };
        DiskPowerProbe probe = routes.Probe();

        Assert.False(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.False(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Equal([4], routes.NativeAsked);
        Assert.Equal([4, 4], routes.SatAsked);

        // Another drive starts from the native command again.
        Assert.False(probe.IsSpunDown(5, "Other", "77"));
        Assert.Equal([4, 5], routes.NativeAsked);

        // The answer itself is never remembered: only the route.
        routes.Sat = true;
        Assert.True(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Equal([4, 5], routes.NativeAsked);
    }

    [Fact]
    public void WhenTheRememberedRouteStopsAnsweringTheOtherIsTried()
    {
        var routes = new Routes { Native = null, Sat = false };
        DiskPowerProbe probe = routes.Probe();
        Assert.False(probe.IsSpunDown(4, "Extreme", "4C53"));

        (routes.Native, routes.Sat) = (true, null);
        Assert.True(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Equal([4, 4], routes.SatAsked); // the remembered route first
        Assert.Equal([4, 4], routes.NativeAsked);

        // The native route is the remembered one now.
        Assert.True(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Equal([4, 4], routes.SatAsked);
        Assert.Equal([4, 4, 4], routes.NativeAsked);
    }

    [Fact]
    public void ADeadRouteIsRetriedOnlyAfterFiveMinutes()
    {
        var routes = new Routes { Native = null, Sat = null };
        DiskPowerProbe probe = routes.Probe();
        Assert.Null(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Equal((1, 1), (routes.NativeAsked.Count, routes.SatAsked.Count));

        // Both routes would answer now, but nothing is sent and nothing is assumed.
        (routes.Native, routes.Sat) = (false, false);
        routes.Time.Advance(TimeSpan.FromSeconds(299));
        Assert.Null(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Equal((1, 1), (routes.NativeAsked.Count, routes.SatAsked.Count));

        (routes.Native, routes.Sat) = (null, null);
        routes.Time.Advance(TimeSpan.FromSeconds(1));
        Assert.Null(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Equal((2, 2), (routes.NativeAsked.Count, routes.SatAsked.Count));

        // The retry failed too: another five minutes.
        routes.Time.Advance(TimeSpan.FromSeconds(299));
        Assert.Null(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Equal((2, 2), (routes.NativeAsked.Count, routes.SatAsked.Count));
    }

    [Fact]
    public void ANewModelOrSerialForgetsTheRoute()
    {
        var routes = new Routes { Native = null, Sat = false };
        DiskPowerProbe probe = routes.Probe();
        Assert.False(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.False(probe.IsSpunDown(4, "Extreme", "4C53"));
        Assert.Single(routes.NativeAsked);

        Assert.False(probe.IsSpunDown(4, "Extreme", "0000")); // another serial
        Assert.Equal(2, routes.NativeAsked.Count);
        Assert.False(probe.IsSpunDown(4, "Ultra", "0000")); // another model
        Assert.Equal(3, routes.NativeAsked.Count);
        Assert.False(probe.IsSpunDown(4, "Ultra", "0000"));
        Assert.Equal(3, routes.NativeAsked.Count);

        // The same when the enumeration sees the new identity first.
        routes.Drives.Add(UsbStick(model: "Ultra", serial: "1111"));
        Assert.Empty(Blockers(probe.CheckGate()));
        Assert.Equal(4, routes.NativeAsked.Count);
    }

    [Fact]
    public void RemovingAndReaddingTheSameIdentityForgetsTheRoute()
    {
        var routes = new Routes { Native = null, Sat = false };
        routes.Drives.Add(UsbStick());
        DiskPowerProbe probe = routes.Probe();
        Assert.Empty(Blockers(probe.CheckGate()));
        Assert.Empty(Blockers(probe.CheckGate()));
        Assert.Single(routes.NativeAsked);

        routes.Drives.Clear();
        Assert.Empty(Blockers(probe.CheckGate()));
        routes.Drives.Add(UsbStick());
        Assert.Empty(Blockers(probe.CheckGate()));
        Assert.Equal(2, routes.NativeAsked.Count);
    }

    [Fact]
    public void AMissingIdentityDoesNotKeepARouteAcrossEnumeration()
    {
        foreach (DriveFacts unidentified in new[] { UsbStick(serial: null), UsbStick(model: null), UsbStick(serial: "") })
        {
            var routes = new Routes { Native = null, Sat = false };
            routes.Drives.Add(unidentified);
            DiskPowerProbe probe = routes.Probe();

            Assert.Empty(Blockers(probe.CheckGate()));
            Assert.Empty(Blockers(probe.CheckGate()));
            Assert.Equal(2, routes.NativeAsked.Count);
        }

        // Not even the "no route" memory: a drive that cannot be told apart is asked every round.
        var dead = new Routes { Native = null, Sat = null };
        dead.Drives.Add(UsbStick(serial: null));
        DiskPowerProbe deadProbe = dead.Probe();
        Assert.Single(Blockers(deadProbe.CheckGate()));
        Assert.Single(Blockers(deadProbe.CheckGate()));
        Assert.Equal((2, 2), (dead.NativeAsked.Count, dead.SatAsked.Count));

        // And not between enumerations either (the hub asks without enumerating).
        Assert.Null(deadProbe.IsSpunDown(4, "Extreme", null));
        Assert.Null(deadProbe.IsSpunDown(4, "Extreme", null));
        Assert.Equal((4, 4), (dead.NativeAsked.Count, dead.SatAsked.Count));
    }

    [Fact]
    public void ASuccessfulIoctlWithUnknownRegistersTriesSat()
    {
        // The native IOCTL succeeded but its registers say nothing (aborted, or a sector count
        // outside the CHECK POWER MODE values): the native route reports that as "no answer".
        foreach (bool? unknown in new[] { DiskPowerProbe.InterpretAtaResult(status: 0x51, sectorCount: 0x00), DiskPowerProbe.InterpretAtaResult(status: 0x50, sectorCount: 0x12) })
        {
            var routes = new Routes { Native = unknown, Sat = true };

            Assert.True(routes.Probe().IsSpunDown(0, "ST2000DM008-2FR102", "ZFL0"));
            Assert.Equal([0], routes.SatAsked);
        }
    }

    private static DiskPowerProbe.NativeMethods.ScsiPassThroughWithSense SatReply(byte scsiStatus, string senseHex)
    {
        byte[] sense = new byte[32];
        Convert.FromHexString(senseHex).CopyTo(sense, 0);
        return new DiskPowerProbe.NativeMethods.ScsiPassThroughWithSense
        {
            Spt = new DiskPowerProbe.NativeMethods.ScsiPassThrough { ScsiStatus = scsiStatus, SenseInfoLength = 32, SenseInfoOffset = 56 },
            Sense = sense,
        };
    }

    [Fact]
    public void DescriptorSenseIsAcceptedWithScsiStatusZero()
    {
        // What the SATA HDD of the spike answers: SCSI status GOOD, registers in the sense data anyway.
        Assert.False(DiskPowerProbe.InterpretSatReply(SatReply(0x00, "720000000000000E090C000000FF00FF00000000E050"), returned: 88));
        Assert.True(DiskPowerProbe.InterpretSatReply(SatReply(0x00, "720000000000000E090C00000000000000000000E050"), returned: 88));

        // The USB stick: CHECK CONDITION, fixed format.
        Assert.False(DiskPowerProbe.InterpretSatReply(SatReply(0x02, "F00001005000FF0A00000000001D00000000"), returned: 88));
    }

    [Fact]
    public void OnlyTheSenseBytesActuallyReturnedAreRead()
    {
        DiskPowerProbe.NativeMethods.ScsiPassThroughWithSense reply = SatReply(0x00, "720000000000000E090C000000FF00FF00000000E050");

        Assert.Null(DiskPowerProbe.InterpretSatReply(reply, returned: 0));
        Assert.Null(DiskPowerProbe.InterpretSatReply(reply, returned: 56)); // the header alone: no sense came back
        Assert.Null(DiskPowerProbe.InterpretSatReply(reply, returned: 56 + 21)); // one byte short of the 22 declared
        Assert.False(DiskPowerProbe.InterpretSatReply(reply, returned: 56 + 22));
        Assert.False(DiskPowerProbe.InterpretSatReply(reply, returned: 4096)); // never past the sense buffer

        reply.Sense = null!; // a reply the marshaller left without its array
        Assert.Null(DiskPowerProbe.InterpretSatReply(reply, returned: 88));
    }

    [Fact]
    public void TheSenseSliceIsBoundedByTheLengthTheReplyDeclares()
    {
        // The USB stick's 18 sense bytes.
        DiskPowerProbe.NativeMethods.ScsiPassThroughWithSense reply = SatReply(0x02, "F00001005000FF0A00000000001D00000000");
        reply.Spt.SenseInfoLength = 18;
        Assert.False(DiskPowerProbe.InterpretSatReply(reply, returned: 88));

        reply.Spt.SenseInfoLength = 17; // one byte short of what the sense data itself declares
        Assert.Null(DiskPowerProbe.InterpretSatReply(reply, returned: 88));

        reply.Spt.SenseInfoLength = 0; // the driver wrote no sense data: the buffer is not an answer
        Assert.Null(DiskPowerProbe.InterpretSatReply(reply, returned: 88));

        reply.Spt.SenseInfoLength = 255; // never past the buffer
        Assert.False(DiskPowerProbe.InterpretSatReply(reply, returned: 88));
    }

    [Fact]
    public void ASenseOffsetOtherThanTheBufferIsUnknown()
    {
        foreach (uint offset in new uint[] { 0, 48, 60, 88 })
        {
            DiskPowerProbe.NativeMethods.ScsiPassThroughWithSense reply = SatReply(0x02, "F00001005000FF0A00000000001D00000000");
            reply.Spt.SenseInfoOffset = offset;
            Assert.Null(DiskPowerProbe.InterpretSatReply(reply, returned: 88));
        }
    }

    [Fact]
    public void ScsiPassThroughIsFiftySixBytesOnX64()
    {
        // USHORT Length (0), UCHAR ScsiStatus/PathId/TargetId/Lun/CdbLength/SenseInfoLength/DataIn (2-8),
        // 3 bytes padding, ULONG DataTransferLength (12), ULONG TimeOutValue (16), 4 bytes padding,
        // ULONG_PTR DataBufferOffset (24), ULONG SenseInfoOffset (32), UCHAR Cdb[16] (36), padded
        // to 8 = 56 bytes on x64; then our 32 sense bytes.
        Assert.Equal(56, Marshal.SizeOf<DiskPowerProbe.NativeMethods.ScsiPassThrough>());
        Assert.Equal(2, (int)Marshal.OffsetOf<DiskPowerProbe.NativeMethods.ScsiPassThrough>(nameof(DiskPowerProbe.NativeMethods.ScsiPassThrough.ScsiStatus)));
        Assert.Equal(8, (int)Marshal.OffsetOf<DiskPowerProbe.NativeMethods.ScsiPassThrough>(nameof(DiskPowerProbe.NativeMethods.ScsiPassThrough.DataIn)));
        Assert.Equal(24, (int)Marshal.OffsetOf<DiskPowerProbe.NativeMethods.ScsiPassThrough>(nameof(DiskPowerProbe.NativeMethods.ScsiPassThrough.DataBufferOffset)));
        Assert.Equal(32, (int)Marshal.OffsetOf<DiskPowerProbe.NativeMethods.ScsiPassThrough>(nameof(DiskPowerProbe.NativeMethods.ScsiPassThrough.SenseInfoOffset)));
        Assert.Equal(36, (int)Marshal.OffsetOf<DiskPowerProbe.NativeMethods.ScsiPassThrough>(nameof(DiskPowerProbe.NativeMethods.ScsiPassThrough.Cdb)));

        Assert.Equal(88, Marshal.SizeOf<DiskPowerProbe.NativeMethods.ScsiPassThroughWithSense>());
        Assert.Equal(56, (int)Marshal.OffsetOf<DiskPowerProbe.NativeMethods.ScsiPassThroughWithSense>(nameof(DiskPowerProbe.NativeMethods.ScsiPassThroughWithSense.Sense)));
    }

    [Fact]
    public void TheSatRequestIsCheckPowerModeAsAtaPassThrough16()
    {
        DiskPowerProbe.NativeMethods.ScsiPassThroughWithSense request = DiskPowerProbe.SatCheckPowerModeRequest();

        // ATA PASS-THROUGH(16), protocol non-data, CK_COND set, command 0xE5: nothing else is ever sent.
        Assert.Equal(Convert.FromHexString("8506200000000000000000000000E500"), request.Spt.Cdb);
        Assert.Equal((56, 16, 32, 56u), (request.Spt.Length, request.Spt.CdbLength, request.Spt.SenseInfoLength, request.Spt.SenseInfoOffset));
        Assert.Equal(2, request.Spt.DataIn); // SCSI_IOCTL_DATA_UNSPECIFIED
        Assert.Equal((0u, (nuint)0, 5u), (request.Spt.DataTransferLength, request.Spt.DataBufferOffset, request.Spt.TimeOutValue));
        Assert.Equal(32, request.Sense.Length);
        Assert.Equal(0x0004D004u, DiskPowerProbe.NativeMethods.IoctlScsiPassThrough);
    }

    [Fact]
    public void AtaPassThroughExIsFortyEightBytesOnX64()
    {
        // USHORT Length (0), USHORT AtaFlags (2), UCHAR PathId/TargetId/Lun/ReservedAsUchar (4-7),
        // ULONG DataTransferLength (8), ULONG TimeOutValue (12), ULONG ReservedAsUlong (16),
        // 4 bytes padding, ULONG_PTR DataBufferOffset (24), UCHAR PreviousTaskFile[8] (32),
        // UCHAR CurrentTaskFile[8] (40) = 48 bytes on x64 (40 on x86).
        Assert.Equal(48, Marshal.SizeOf<DiskPowerProbe.NativeMethods.AtaPassThroughEx>());
        Assert.Equal(24, (int)Marshal.OffsetOf<DiskPowerProbe.NativeMethods.AtaPassThroughEx>(nameof(DiskPowerProbe.NativeMethods.AtaPassThroughEx.DataBufferOffset)));
        Assert.Equal(40, (int)Marshal.OffsetOf<DiskPowerProbe.NativeMethods.AtaPassThroughEx>(nameof(DiskPowerProbe.NativeMethods.AtaPassThroughEx.CurrentTaskFile)));
    }

    [Fact]
    public void StoragePropertyQueryIsTwelveBytes()
    {
        // STORAGE_PROPERTY_ID (4) + STORAGE_QUERY_TYPE (4) + UCHAR AdditionalParameters[1], padded to 4: 12.
        Assert.Equal(12, Marshal.SizeOf<DiskPowerProbe.NativeMethods.StoragePropertyQuery>());
    }

    [Fact]
    public void DeviceSeekPenaltyDescriptorIsTwelveBytes()
    {
        // ULONG Version (4) + ULONG Size (4) + BOOLEAN IncursSeekPenalty (1), padded to 4: 12.
        Assert.Equal(12, Marshal.SizeOf<DiskPowerProbe.NativeMethods.DeviceSeekPenaltyDescriptor>());
    }
}
