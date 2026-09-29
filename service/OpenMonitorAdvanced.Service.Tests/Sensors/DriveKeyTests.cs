using System.Text.Json;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// <see cref="DriveKey.Compute"/> against the vector shared with the Rust side
/// (<c>protocol/fixtures/drive_key.json</c>, <c>crates/oma-ipc/tests/fixtures.rs</c>).
/// </summary>
public sealed class DriveKeyTests
{
    private const string Samsung = "589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d83";

    public static TheoryData<string, string, string?> SharedVector()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "Fixtures", "drive_key.json");
        using JsonDocument doc = JsonDocument.Parse(File.ReadAllText(path));
        var data = new TheoryData<string, string, string?>();
        foreach (JsonElement item in doc.RootElement.EnumerateArray())
        {
            JsonElement key = item.GetProperty("key");
            data.Add(
                item.GetProperty("model").GetString()!,
                item.GetProperty("serial").GetString()!,
                key.ValueKind == JsonValueKind.Null ? null : key.GetString());
        }

        return data;
    }

    [Theory]
    [MemberData(nameof(SharedVector))]
    public void MatchesTheSharedVector(string model, string serial, string? expected) =>
        Assert.Equal(expected, DriveKey.Compute(model, serial));

    [Fact]
    public void DriveKeyUsesTheTrimmedDescriptorModelAndSerial()
    {
        Assert.Equal(Samsung, DriveKey.Compute("Samsung SSD 990 PRO 2TB", "0025_38B1_4150_2A6C."));
        Assert.Equal(Samsung, DriveKey.Compute("  Samsung SSD 990 PRO 2TB\t", " 0025_38B1_4150_2A6C.\r\n"));
    }

    [Theory]
    [InlineData("Msft Virtual Disk", "")]
    [InlineData("Msft Virtual Disk", "   ")]
    [InlineData("Msft Virtual Disk", null)]
    [InlineData("", "SERIAL")]
    [InlineData(null, "SERIAL")]
    [InlineData(null, null)]
    public void ADiskWithoutDescriptorSerialHasNoDriveKey(string? model, string? serial) =>
        Assert.Null(DriveKey.Compute(model, serial));
}
