using OpenMonitorAdvanced.Service.Frames;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Frames;

public sealed class PresentMonCsvTests
{
    private const string Header =
        "Application,ProcessID,SwapChainAddress,PresentMode,FrameType,TimeInQPC,MsBetweenPresents,"
        + "MsBetweenDisplayChange,MsUntilDisplayed,MsBetweenAppStart,MsPCLatency,MsGPUBusy,PCLFrameId";

    private const string Row =
        "game.exe,4242,0x22A59F87270,Hardware: Independent Flip,Application,366427208391,14.173,13.64,18.98,14.186,60.55,13.69,14347";

    private static PresentMonCsv ReadyParser(string header = Header)
    {
        var csv = new PresentMonCsv();
        Assert.True(csv.TryReadHeader(header, out var missing), missing);
        return csv;
    }

    [Fact]
    public void ReadsColumnsByNameInAnyOrder()
    {
        var permuted = "PCLFrameId,MsGPUBusy,MsPCLatency,MsBetweenAppStart,MsUntilDisplayed,MsBetweenDisplayChange,"
            + "MsBetweenPresents,TimeInQPC,FrameType,PresentMode,SwapChainAddress,ProcessID,Application";
        var csv = ReadyParser(permuted);

        var row = csv.ParseRow(
            "14347,13.69,60.55,14.186,18.98,13.64,14.173,366427208391,Application,Hardware: Independent Flip,0x22A59F87270,4242,game.exe");

        Assert.NotNull(row);
        Assert.Equal(4242u, row.Pid);
        Assert.Equal("game.exe", row.Name);
        Assert.Equal("Hardware: Independent Flip", row.PresentMode);
        var f = row.Frame;
        Assert.Equal(366427208391UL, f.Qpc);
        Assert.Equal(0x22A59F87270UL, f.Swapchain);
        Assert.Equal("app", f.FrameType);
        Assert.True(f.Displayed);
        Assert.Equal(14.173, f.MsBetweenPresents);
        Assert.Equal(13.64, f.MsBetweenDisplayChange);
        Assert.Equal(18.98, f.MsUntilDisplayed);
        Assert.Equal(14.186, f.MsAppFrametime);
        Assert.Equal(60.55, f.MsPcLatency);
        Assert.Equal(13.69, f.MsGpuBusy);
        Assert.Equal(14347UL, f.PclFrameId);
        Assert.Equal(0, csv.Rejected);
    }

    [Fact]
    public void ReportsTheMissingRequiredColumn()
    {
        var csv = new PresentMonCsv();

        var ok = csv.TryReadHeader(Header.Replace("MsBetweenDisplayChange,", ""), out var missing);

        Assert.False(ok);
        Assert.Equal("MsBetweenDisplayChange", missing);
    }

    [Fact]
    public void ToleratesBomAndCarriageReturn()
    {
        var csv = new PresentMonCsv();
        Assert.True(csv.TryReadHeader("\uFEFF" + Header + "\r", out _));

        var row = csv.ParseRow(Row + "\r");

        Assert.NotNull(row);
        Assert.Equal(14347UL, row.Frame.PclFrameId);
        Assert.Equal(0, csv.Rejected);
    }

    [Fact]
    public void NaBecomesNullInOptionalFields()
    {
        var csv = ReadyParser();

        var row = csv.ParseRow("game.exe,4242,0x10,Composed: Flip,Application,100,14.0,NA,NA,NA,NA,NA,NA");

        Assert.NotNull(row);
        var f = row.Frame;
        Assert.False(f.Displayed);
        Assert.Null(f.MsBetweenDisplayChange);
        Assert.Null(f.MsUntilDisplayed);
        Assert.Null(f.MsAppFrametime);
        Assert.Null(f.MsPcLatency);
        Assert.Null(f.MsGpuBusy);
        Assert.Null(f.PclFrameId);
    }

    [Fact]
    public void AbsentOptionalColumnsGiveNullsAndUnknownFrameType()
    {
        var csv = ReadyParser(
            "Application,ProcessID,SwapChainAddress,PresentMode,TimeInQPC,MsBetweenPresents,MsBetweenDisplayChange,MsUntilDisplayed,MsBetweenAppStart");

        var row = csv.ParseRow("game.exe,1,0x10,Composed: Flip,100,14.0,14.0,15.0,14.0");

        Assert.NotNull(row);
        Assert.Equal("unknown", row.Frame.FrameType);
        Assert.Null(row.Frame.MsPcLatency);
        Assert.Null(row.Frame.MsGpuBusy);
        Assert.Null(row.Frame.PclFrameId);
    }

    [Theory]
    [InlineData("Application", "app")]
    [InlineData("Intel XeSS-FG", "generated_intel_xefg")]
    [InlineData("AMD AFMF", "generated_amd_afmf")]
    [InlineData("NVIDIA DLSS-FG", "generated_other")]
    public void MapsFrameTypes(string text, string expected)
    {
        var csv = ReadyParser();

        var row = csv.ParseRow(Row.Replace("Application,366", text + ",366"));

        Assert.NotNull(row);
        Assert.Equal(expected, row.Frame.FrameType);
    }

    [Fact]
    public void RejectsLongLinesAndNonNumericValues()
    {
        var csv = ReadyParser();

        Assert.Null(csv.ParseRow(Row + new string('x', 4100)));
        Assert.Null(csv.ParseRow(Row.Replace("14.173", "fast")));
        Assert.Equal(2, csv.Rejected);
        Assert.NotNull(csv.ParseRow(Row));
        Assert.Equal(2, csv.Rejected);
    }

    [Theory]
    [InlineData("14.173", "NaN")]
    [InlineData("14.173", "Infinity")]
    [InlineData("13.64", "NaN")]
    [InlineData("60.55", "-Infinity")]
    public void RejectsNonFiniteNumbers(string original, string replacement)
    {
        var csv = ReadyParser();

        Assert.Null(csv.ParseRow(Row.Replace(original, replacement)));
        Assert.Equal(1, csv.Rejected);
    }

    [Fact]
    public void RejectsRowsWithTheWrongFieldCountOrBeforeTheHeader()
    {
        var early = new PresentMonCsv();
        Assert.Null(early.ParseRow(Row));
        Assert.Equal(1, early.Rejected);

        var csv = ReadyParser();
        Assert.Null(csv.ParseRow(Row + ",extra"));
        Assert.Equal(1, csv.Rejected);
    }

    [Fact]
    public void ZeroPclFrameIdIsNull()
    {
        var csv = ReadyParser();

        var row = csv.ParseRow(Row.Replace(",14347", ",0"));

        Assert.NotNull(row);
        Assert.Null(row.Frame.PclFrameId);
    }

    public static TheoryData<string, int> Fixtures => new()
    {
        { "nofg", 742 },
        { "nofg-pcl", 970 },
        { "cpubound", 1564 },
        { "dlssfg", 1302 },
        { "dlssfg-pcl", 1306 },
        { "fsrfg", 1259 },
        { "fsrfg-pcl", 1063 },
        { "smooth", 1572 },
        { "smooth-pcl", 1572 },
    };

    [Theory]
    [MemberData(nameof(Fixtures))]
    public void ParsesEveryFixtureWithoutRejects(string name, int rows)
    {
        var lines = File.ReadAllLines(Path.Combine(AppContext.BaseDirectory, "Fixtures", "PresentMon", name + ".csv"));
        var csv = new PresentMonCsv();
        Assert.True(csv.TryReadHeader(lines[0], out var missing), missing);

        var parsed = lines.Skip(1).Select(csv.ParseRow).Where(r => r is not null).Count();

        Assert.Equal(rows, parsed);
        Assert.Equal(0, csv.Rejected);
    }
}
