using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// <see cref="SatSense"/> over the sense data captured in the M6b spike (design §2): the answer
/// of <c>ATA PASS-THROUGH(16)</c> CHECK POWER MODE from a SATA HDD and from a USB stick.
/// </summary>
public sealed class SatSenseTests
{
    private static readonly byte[] SataActive = Convert.FromHexString("72000000000000 0E 090C000000FF00FF00000000E050".Replace(" ", ""));
    private static readonly byte[] SataStandby = Convert.FromHexString("72000000000000 0E 090C00000000000000000000E050".Replace(" ", ""));
    private static readonly byte[] UsbActive = Convert.FromHexString("F00001005000FF0A00000000001D00000000");

    private static byte[] With(byte[] sense, int index, byte value)
    {
        byte[] copy = [.. sense];
        copy[index] = value;
        return copy;
    }

    [Fact]
    public void ReadsTheDescriptorFormat()
    {
        Assert.True(SatSense.TryReadRegisters(SataActive, out byte status, out byte sectorCount));
        Assert.Equal((0x50, 0xFF), (status, sectorCount));

        Assert.True(SatSense.TryReadRegisters(SataStandby, out status, out sectorCount));
        Assert.Equal((0x50, 0x00), (status, sectorCount));

        // Deferred-error descriptor format (0x73), and the spare bytes of a 32-byte sense buffer.
        byte[] padded = new byte[32];
        With(SataActive, 0, 0x73).CopyTo(padded, 0);
        Assert.True(SatSense.TryReadRegisters(padded, out status, out sectorCount));
        Assert.Equal((0x50, 0xFF), (status, sectorCount));
    }

    [Fact]
    public void ReadsTheFixedFormatWithTheValidBitSet()
    {
        Assert.True(SatSense.TryReadRegisters(UsbActive, out byte status, out byte sectorCount));
        Assert.Equal((0x50, 0xFF), (status, sectorCount));

        // The same without the VALID bit (0x70).
        Assert.True(SatSense.TryReadRegisters(With(UsbActive, 0, 0x70), out status, out sectorCount));
        Assert.Equal((0x50, 0xFF), (status, sectorCount));
    }

    [Fact]
    public void AFixedFormatWithoutAtaInformationIsUnknown()
    {
        // ASC/ASCQ 00/00 instead of 00/1D (ATA PASS THROUGH INFORMATION AVAILABLE).
        Assert.False(SatSense.TryReadRegisters(With(UsbActive, 13, 0x00), out _, out _));
        Assert.False(SatSense.TryReadRegisters(With(UsbActive, 12, 0x04), out _, out _));
    }

    [Fact]
    public void ATruncatedOrInconsistentSenseIsUnknown()
    {
        Assert.False(SatSense.TryReadRegisters(SataActive.AsSpan(0, 12), out _, out _)); // fewer bytes than declared
        Assert.False(SatSense.TryReadRegisters(With(SataActive, 7, 0xFF), out _, out _)); // declares more than returned
        Assert.False(SatSense.TryReadRegisters(With(SataActive, 8, 0x0A), out _, out _)); // no ATA Status Return descriptor
        Assert.False(SatSense.TryReadRegisters(With(SataActive, 9, 0x04), out _, out _)); // descriptor too short
        Assert.False(SatSense.TryReadRegisters(With(SataActive, 9, 0x40), out _, out _)); // descriptor past the declared end
        Assert.False(SatSense.TryReadRegisters(With(SataActive, 7, 0x08), out _, out _)); // declared end inside the descriptor
        Assert.False(SatSense.TryReadRegisters(UsbActive.AsSpan(0, 13), out _, out _)); // fixed format cut before the ASCQ
        Assert.False(SatSense.TryReadRegisters(With(UsbActive, 7, 0x04), out _, out _)); // fixed format declared too short
        Assert.False(SatSense.TryReadRegisters(With(SataActive, 0, 0x00), out _, out _)); // not sense data
        Assert.False(SatSense.TryReadRegisters(new byte[32], out _, out _)); // an untouched buffer
        Assert.False(SatSense.TryReadRegisters([], out _, out _));
    }
}
