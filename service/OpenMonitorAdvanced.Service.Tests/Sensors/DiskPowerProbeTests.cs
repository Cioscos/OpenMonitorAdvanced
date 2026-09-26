using System.Runtime.InteropServices;
using Microsoft.Extensions.Logging;
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
    private static (IReadOnlyList<DriveBlocker> Blockers, List<int> Asked) Gate(params DriveFacts[] drives)
    {
        var asked = new List<int>();
        IReadOnlyList<DriveBlocker> blockers = DiskPowerProbe.FindGateBlockers(drives, n =>
        {
            asked.Add(n);
            return null;
        });
        return (blockers, asked);
    }

    [Fact]
    public void ADriveWithoutMediaDoesNotBlockTheGate()
    {
        (IReadOnlyList<DriveBlocker> blockers, List<int> asked) = Gate(Drive(4, bus: null, seekPenalty: null, DriveAvailability.NoMedia));
        Assert.Empty(blockers);
        Assert.Empty(asked);
    }

    [Fact]
    public void VirtualAndStorageSpacesDisksDoNotBlockTheGate()
    {
        // STORAGE_BUS_TYPE BusTypeVirtual (0xE), BusTypeFileBackedVirtual (0xF): VHD/VHDX, ramdisks;
        // BusTypeSpaces (0x10): a Storage Spaces virtual disk (ruling R19).
        (IReadOnlyList<DriveBlocker> blockers, List<int> asked) = Gate(Drive(5, bus: 0x0E, seekPenalty: null), Drive(6, bus: 0x0F, seekPenalty: true), Drive(8, bus: 0x10, seekPenalty: null));
        Assert.Empty(blockers);
        Assert.Empty(asked);
    }

    [Fact]
    public void NvmeDoesNotBlockTheGate()
    {
        (IReadOnlyList<DriveBlocker> blockers, List<int> asked) = Gate(Drive(2, bus: 0x11, seekPenalty: null));
        Assert.Empty(blockers);
        Assert.Empty(asked);
    }

    [Fact]
    public void ASolidStateDriveDoesNotBlockTheGate()
    {
        (IReadOnlyList<DriveBlocker> blockers, List<int> asked) = Gate(Drive(1, bus: 0x0B, seekPenalty: false));
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
            Assert.Empty(DiskPowerProbe.FindGateBlockers([drive], _ => false));
            DriveBlocker standby = Assert.Single(DiskPowerProbe.FindGateBlockers([drive], _ => true));
            Assert.Equal((drive, (bool?)true), (standby.Drive, standby.SpunDown));
            DriveBlocker unanswered = Assert.Single(DiskPowerProbe.FindGateBlockers([drive], _ => null));
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
        Assert.Null(probe.IsSpunDown(-1));
        Assert.Null(probe.Describe(-1));
        Assert.Equal(Drive(0, 0x0B, true), probe.Describe(0));
        Assert.Null(probe.Describe(9));
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
