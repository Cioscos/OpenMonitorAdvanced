using OpenMonitorAdvanced.Service.Frames;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Frames;

public sealed class PresentMonPinTests
{
    [Fact]
    public void PinMatchesTheInstallerFile()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "presentmon.sha256");
        Assert.Equal(PresentMonPin.Sha256, File.ReadAllText(path).Trim());
    }

    [Fact]
    public void HashOfAMissingFileIsNull() =>
        Assert.Null(PresentMonPin.HashOf(Path.Combine(AppContext.BaseDirectory, "no-such-file.exe")));

    [Fact]
    public void HashOfAFileIsUppercaseSha256()
    {
        string path = Path.GetTempFileName();
        try
        {
            File.WriteAllBytes(path, []);
            Assert.Equal("E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855", PresentMonPin.HashOf(path));
        }
        finally
        {
            File.Delete(path);
        }
    }
}
