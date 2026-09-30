using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// The AMD identification rules and the Intel gate of
/// <c>docs/superpowers/references/m5/s1-lhm-sensors.md</c> §3, §4.2 and "Per il piano M5b" point 6.
/// </summary>
public sealed class CpuIdentityTests
{
    private static CpuInfo Amd(string brand, int family, int model = 0x61) => new("AMD", brand, family, model, null);

    [Theory]
    [InlineData("AMD Ryzen 7 5800X3D 8-Core Processor", 0x19, 90)]
    [InlineData("AMD Ryzen 5 5600G with Radeon Graphics", 0x19, 95)]
    [InlineData("AMD Ryzen 7 8700G w/ Radeon 780M Graphics", 0x19, 95)]
    [InlineData("AMD Ryzen 9 9950X3D 16-Core Processor", 0x1A, 95)]
    [InlineData("AMD Ryzen 7 7800X3D 8-Core Processor", 0x19, 89)]
    public void AmdDesktopPartsAreIdentifiedByBrandAndFamily(string brand, int family, int expected)
    {
        Assert.Equal(expected, CpuIdentity.TjMaxC(Amd(brand, family)));
    }

    [Fact]
    public void AWrongCpuidFamilyGivesNoTjMax()
    {
        Assert.Null(CpuIdentity.TjMaxC(Amd("AMD Ryzen 7 7800X3D 8-Core Processor", 0x1A)));
    }

    [Fact]
    public void ATailOutsideTheAllowedOnesGivesNoTjMax()
    {
        Assert.Null(CpuIdentity.NormalizeAmdKey("AMD Ryzen 7 7800X3D Engineering"));
        Assert.Null(CpuIdentity.TjMaxC(Amd("AMD Ryzen 7 7800X3D Engineering", 0x19)));
    }

    [Fact]
    public void AnEngineeringSampleGivesNoTjMax()
    {
        Assert.Null(CpuIdentity.TjMaxC(Amd("AMD Eng Sample: 100-000000910-40_Y", 0x19)));
    }

    [Fact]
    public void AModelAbsentFromTheTableGivesNoTjMax()
    {
        // The AMD table does not publish a value for the 3300X: it stays out on purpose.
        Assert.Equal("Ryzen 3 3300X", CpuIdentity.NormalizeAmdKey("AMD Ryzen 3 3300X 4-Core Processor"));
        Assert.Null(CpuIdentity.TjMaxC(Amd("AMD Ryzen 3 3300X 4-Core Processor", 0x17)));
    }

    [Fact]
    public void ThreadripperGivesNoTjMax()
    {
        Assert.Null(CpuIdentity.NormalizeAmdKey("AMD Ryzen Threadripper 3970X 32-Core Processor"));
        Assert.Null(CpuIdentity.TjMaxC(Amd("AMD Ryzen Threadripper 3970X 32-Core Processor", 0x17)));
    }

    [Fact]
    public void AnIntelBrandStringNeverReadsTheAmdTable()
    {
        var intel = new CpuInfo("Intel", "Intel(R) Core(TM) i9-13900K", 6, 0xB7, null);

        Assert.Null(CpuIdentity.TjMaxC(intel));
    }

    [Fact]
    public void AnAmdBrandStringUnderAnotherVendorGivesNoTjMax()
    {
        var other = new CpuInfo("Unknown", "AMD Ryzen 7 7800X3D 8-Core Processor", 0x19, 0x61, null);

        Assert.Null(CpuIdentity.TjMaxC(other));
    }

    [Fact]
    public void TheBrandStringIsCleanedBeforeMatching()
    {
        // Padding, trademark marks, repeated spaces and a replacement character (all seen in
        // real CPUID strings) do not change the key; the comparison stays case-sensitive.
        Assert.Equal("Ryzen 7 7800X3D", CpuIdentity.NormalizeAmdKey("AMD Ryzen 7  7800X3D   8-Core Processor            "));
        Assert.Equal("Ryzen 7 7800X3D", CpuIdentity.NormalizeAmdKey("AMD Ryzen™ 7 7800X3D 8-Core Processor\0"));
        Assert.Equal("Ryzen 5 PRO 5650G", CpuIdentity.NormalizeAmdKey("AMD Ryzen 5 PRO 5650G with Radeon Graphics"));
        Assert.Equal("Ryzen 9 9950X3D2", CpuIdentity.NormalizeAmdKey("AMD Ryzen 9 9950X3D2 Dual Edition"));
        Assert.Equal("Ryzen 5 3600", CpuIdentity.NormalizeAmdKey("AMD Ryzen 5 3600 6-Core Processor"));
        Assert.Null(CpuIdentity.NormalizeAmdKey("amd ryzen 7 7800x3d 8-core processor"));
        Assert.Null(CpuIdentity.NormalizeAmdKey(""));
    }

    [Fact]
    public void TheOnlyEntryWithoutACodenameSkipsTheFamilyCheck()
    {
        Assert.True(AmdTjMaxTable.TryGet("Ryzen 5 PRO 3350GE", out int tjMax, out int? family));
        Assert.Equal(95, tjMax);
        Assert.Null(family);

        foreach (int anyFamily in new[] { 0x17, 0x19, 0x1A })
        {
            Assert.Equal(95, CpuIdentity.TjMaxC(Amd("AMD Ryzen 5 PRO 3350GE with Radeon Graphics", anyFamily)));
        }
    }

    [Fact]
    public void EveryTableEntryIsFoundByItsOwnKey()
    {
        Assert.Equal(164, AmdTjMaxTable.Entries.Count);
        Assert.Single(AmdTjMaxTable.Entries, e => e.Value.Family is null);

        foreach ((string key, (int tjMax, int? family)) in AmdTjMaxTable.Entries)
        {
            Assert.StartsWith("Ryzen ", key, StringComparison.Ordinal);
            Assert.Contains(tjMax, new[] { 89, 90, 95 });
            Assert.Contains(family, new int?[] { null, 0x17, 0x19, 0x1A });

            string brand = "AMD " + key + " 8-Core Processor";
            Assert.Equal(key, CpuIdentity.NormalizeAmdKey(brand));
            Assert.True(AmdTjMaxTable.TryGet(key, out int foundTjMax, out int? foundFamily), key);
            Assert.Equal(tjMax, foundTjMax);
            Assert.Equal(family, foundFamily);
            Assert.Equal(tjMax, CpuIdentity.TjMaxC(Amd(brand, family ?? 0x19)));

            // Any other family is refused for an entry that has one.
            if (family is int expected)
            {
                Assert.Null(CpuIdentity.TjMaxC(Amd(brand, expected == 0x17 ? 0x19 : 0x17)));
            }
        }
    }

    [Fact]
    public void TheTableKeepsTheOfficialValuesOfTheReferenceMachineAndOfTheOutliers()
    {
        Assert.True(AmdTjMaxTable.TryGet("Ryzen 7 7800X3D", out int a, out int? fa));
        Assert.Equal((89, 0x19), (a, fa));
        Assert.True(AmdTjMaxTable.TryGet("Ryzen 5 5500", out int b, out int? fb));
        Assert.Equal((90, 0x19), (b, fb));
        Assert.True(AmdTjMaxTable.TryGet("Ryzen 9 9950X3D2", out int c, out int? fc));
        Assert.Equal((95, 0x1A), (c, fc));
        Assert.True(AmdTjMaxTable.TryGet("Ryzen 3 PRO 3200G", out int d, out int? fd));
        Assert.Equal((95, 0x17), (d, fd));
        Assert.False(AmdTjMaxTable.TryGet("Ryzen 3 3100", out _, out _));
        Assert.False(AmdTjMaxTable.TryGet("Ryzen 3 3300X", out _, out _));
        Assert.False(AmdTjMaxTable.TryGet("Ryzen 7 1800X", out _, out _));
    }

    [Theory]
    [InlineData(0x9E, 100.0, 100)] // i7-9700K
    [InlineData(0x97, 100.0, 100)] // i5-12600K (hybrid)
    [InlineData(0x8E, 95.0, 95)]
    public void IntelTakesTheTjMaxReadFromTheMsr(int model, double read, int expected)
    {
        var cpu = new CpuInfo("Intel", "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz", 6, model, read);

        Assert.Equal(expected, CpuIdentity.TjMaxC(cpu));
    }

    [Theory]
    [InlineData(0x0F)] // Core 2: fixed table, not the MSR
    [InlineData(0x17)]
    [InlineData(0x1C)] // Atom 45 nm
    public void IntelModelsWithATabulatedTjMaxAreLeftOut(int model)
    {
        Assert.Null(CpuIdentity.TjMaxC(new CpuInfo("Intel", "Intel(R) CPU", 6, model, 100.0)));
    }

    [Fact]
    public void IntelOutsideFamily6OrWithoutAReadingGivesNoTjMax()
    {
        Assert.Null(CpuIdentity.TjMaxC(new CpuInfo("Intel", "Intel(R) Pentium(R) 4", 15, 0x04, 100.0)));
        Assert.Null(CpuIdentity.TjMaxC(new CpuInfo("Intel", "Intel(R) Core(TM) i7-9700K", 6, 0x9E, null)));
        Assert.Null(CpuIdentity.TjMaxC(new CpuInfo("Intel", "Intel(R) Core(TM) i7-9700K", 6, 0x9E, double.NaN)));
        Assert.Null(CpuIdentity.TjMaxC(new CpuInfo("Intel", "Intel(R) Core(TM) i7-9700K", 6, 0x9E, 0.0)));
    }
}
