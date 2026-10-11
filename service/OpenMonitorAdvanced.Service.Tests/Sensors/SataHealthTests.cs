using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

public sealed class SataHealthTests
{
    [Fact]
    public void ThresholdReachedIsCritical()
    {
        Assert.True(SataHealth.Evaluate(rotational: true, attributes: [new AtaAttribute(0x09, 10, 10, 0)]).Critical);
        Assert.True(SataHealth.Evaluate(rotational: true, attributes: [new AtaAttribute(0x09, 5, 10, 0)]).Critical);
        Assert.False(SataHealth.Evaluate(rotational: true, attributes: [new AtaAttribute(0x09, 11, 10, 0)]).Critical);
    }

    [Fact]
    public void ZeroThresholdNeverCritical() =>
        Assert.False(SataHealth.Evaluate(rotational: true, attributes: [new AtaAttribute(0x09, 0, 0, 0)]).Critical);

    [Theory]
    [InlineData((byte)0x05)]
    [InlineData((byte)0xC5)]
    [InlineData((byte)0xC6)]
    public void SectorCountsAboveZeroWarn(byte id)
    {
        Assert.True(SataHealth.Evaluate(rotational: true, attributes: [new AtaAttribute(id, 100, 36, 1)]).SectorWarning);
        Assert.False(SataHealth.Evaluate(rotational: true, attributes: [new AtaAttribute(id, 100, 36, 0)]).SectorWarning);
    }

    [Fact]
    public void SsdSectorCountsAreShownWithoutWarning()
    {
        // A SATA SSD counts retired NAND blocks in 05h: wear, not damage (CrystalDiskInfo keeps it "Good").
        SataHealthResult r = SataHealth.Evaluate(rotational: false, attributes: [new AtaAttribute(0x05, 100, 50, 1), new AtaAttribute(0xC5, 100, 0, 2), new AtaAttribute(0xC6, 100, 0, 3)]);
        Assert.False(r.SectorWarning);
        Assert.Equal((1u, 2u, 3u), (r.Reallocated, r.Pending, r.Uncorrectable));
        Assert.True(SataHealth.Evaluate(rotational: false, attributes: [new AtaAttribute(0x05, 50, 50, 1)]).Critical);
    }

    [Fact]
    public void HighRawBytesAreIgnored()
    {
        SataHealthResult r = SataHealth.Evaluate(rotational: true, attributes: [new AtaAttribute(0x05, 100, 36, 0x0000_0001_0000_0000)]);
        Assert.False(r.SectorWarning);
        Assert.Equal(0u, r.Reallocated);
    }

    [Fact]
    public void MissingAttributesGiveNullCounters()
    {
        SataHealthResult r = SataHealth.Evaluate(rotational: true, attributes: [new AtaAttribute(0x05, 100, 36, 7)]);
        Assert.Equal(7u, r.Reallocated);
        Assert.Null(r.Pending);
        Assert.Null(r.Uncorrectable);
        Assert.Equal(new SataHealthResult(false, false, null, null, null), SataHealth.Evaluate(rotational: true, attributes: []));
    }
}
