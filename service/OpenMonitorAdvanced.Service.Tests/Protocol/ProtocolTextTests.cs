using OpenMonitorAdvanced.Service.Protocol;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Protocol;

public sealed class ProtocolTextTests
{
    [Fact]
    public void AHugeTextIsClippedWithAnEllipsis()
    {
        string clipped = ProtocolText.Clip(new string('x', 100_000));
        Assert.Equal(65, clipped.Length);
        Assert.EndsWith("…", clipped, StringComparison.Ordinal);
    }

    [Fact]
    public void ControlCharactersBecomeQuestionMarks()
    {
        Assert.Equal("a?b?c", ProtocolText.Clip("a\0b\nc"));
    }

    [Fact]
    public void NullBecomesEmpty()
    {
        Assert.Equal("", ProtocolText.Clip(null));
    }

    [Fact]
    public void AShortTextIsUntouched()
    {
        Assert.Equal("gpu", ProtocolText.Clip("gpu"));
    }

    [Fact]
    public void ASurrogatePairAcrossTheLimitIsNotSplit()
    {
        // The emoji occupies indexes 63 and 64, straddling the 64 character limit.
        string text = new string('a', 63) + "\U0001F600" + new string('b', 10);
        string clipped = ProtocolText.Clip(text);
        Assert.Equal(new string('a', 63) + "…", clipped);
        Assert.All(clipped, c => Assert.False(char.IsSurrogate(c)));
    }
}
