using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

public sealed class DisplayNameTests
{
    [Fact]
    public void TrailingNulsAndSpacesAreRemoved()
    {
        Assert.Equal("SanDisk pSSD", DisplayName.Clean("SanDisk pSSD   \0\0\0"));
    }

    [Fact]
    public void InnerSpacesAreKeptAndEdgesTrimmed()
    {
        Assert.Equal("CPU  Total", DisplayName.Clean("  CPU  Total \t"));
    }

    [Fact]
    public void NullBecomesEmpty()
    {
        Assert.Equal("", DisplayName.Clean(null));
    }
}
