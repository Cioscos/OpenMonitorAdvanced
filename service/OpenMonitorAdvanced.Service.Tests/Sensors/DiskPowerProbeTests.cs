using System.Runtime.InteropServices;
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

    [Fact]
    public void EveryRotationalOrUnknownDiskMustBeActive()
    {
        static bool All(bool? seekPenalty, bool? spunDown) =>
            new DiskPowerProbe(() => [0], _ => seekPenalty, _ => spunDown).AllRotationalDisksActive();

        Assert.True(All(seekPenalty: true, spunDown: false));
        Assert.False(All(seekPenalty: true, spunDown: true));
        Assert.False(All(seekPenalty: true, spunDown: null));
        Assert.False(All(seekPenalty: null, spunDown: null)); // unknown = rotational
        Assert.True(All(seekPenalty: null, spunDown: false));
        Assert.True(All(seekPenalty: false, spunDown: null)); // SSD/NVMe: never asked
    }

    [Fact]
    public void SolidStateDisksAreNeverAskedForTheirPowerMode()
    {
        var asked = new List<int>();
        var probe = new DiskPowerProbe(
            () => [0, 1, 2],
            n => n == 1,
            n =>
            {
                asked.Add(n);
                return false;
            });

        Assert.True(probe.AllRotationalDisksActive());
        Assert.Equal([1], asked);
    }

    [Fact]
    public void ANegativeDriveNumberIsUnknown()
    {
        var probe = new DiskPowerProbe(() => [], _ => true, _ => false);
        Assert.Null(probe.IsSpunDown(-1));
        Assert.Null(probe.HasSeekPenalty(-1));
        Assert.True(probe.HasSeekPenalty(0));
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
