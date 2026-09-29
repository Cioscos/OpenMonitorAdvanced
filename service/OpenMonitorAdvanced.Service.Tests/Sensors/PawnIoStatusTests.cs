using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>The pure PawnIO classification (F3.3 with ruling P11) and the marker parsing.</summary>
public sealed class PawnIoStatusTests
{
    private const long Boot = 133_000_000_000_000_000;

    public static TheoryData<KeyState, int?, long?, PawnIoStatus> Table() => new()
    {
        // The device opens: ok, whatever the key and the marker say (P11: no key is only a log).
        { KeyState.Present, null, null, PawnIoStatus.Ok },
        { KeyState.Absent, null, null, PawnIoStatus.Ok },
        { KeyState.Unreadable, null, null, PawnIoStatus.Ok },
        { KeyState.Present, null, Boot + 1, PawnIoStatus.Ok },
        // Device object absent (2 or 3), no uninstall key: not installed.
        { KeyState.Absent, 2, null, PawnIoStatus.Missing },
        { KeyState.Absent, 3, null, PawnIoStatus.Missing },
        { KeyState.Absent, 2, Boot + 1, PawnIoStatus.Missing },
        // Device object absent, key present: reboot pending only with a marker of this boot.
        { KeyState.Present, 2, Boot + 1, PawnIoStatus.RebootPending },
        { KeyState.Present, 3, Boot + 1, PawnIoStatus.RebootPending },
        { KeyState.Present, 2, null, PawnIoStatus.Unavailable },
        { KeyState.Present, 2, Boot, PawnIoStatus.Unavailable },
        { KeyState.Present, 2, Boot - 1, PawnIoStatus.Unavailable },
        // Present but not accessible (5 or any other error): unavailable, the marker is irrelevant.
        { KeyState.Present, 5, null, PawnIoStatus.Unavailable },
        { KeyState.Present, 5, Boot + 1, PawnIoStatus.Unavailable },
        { KeyState.Present, 1275, null, PawnIoStatus.Unavailable },
        // Other error and no key, or an unreadable registry: unknown.
        { KeyState.Absent, 5, null, PawnIoStatus.Unknown },
        { KeyState.Absent, 1275, null, PawnIoStatus.Unknown },
        { KeyState.Unreadable, 2, null, PawnIoStatus.Unknown },
        { KeyState.Unreadable, 2, Boot + 1, PawnIoStatus.Unknown },
        { KeyState.Unreadable, 5, null, PawnIoStatus.Unknown },
    };

    [Theory]
    [MemberData(nameof(Table))]
    public void PawnIoClassifyTable(KeyState key, int? openError, long? markerUtc, PawnIoStatus expected) =>
        Assert.Equal(expected, PawnIoClassifier.Classify(key, openError, markerUtc, Boot));

    [Theory]
    [InlineData(PawnIoStatus.Ok, "ok")]
    [InlineData(PawnIoStatus.Missing, "missing")]
    [InlineData(PawnIoStatus.Unavailable, "unavailable")]
    [InlineData(PawnIoStatus.Unknown, "unknown")]
    [InlineData(PawnIoStatus.RebootPending, "rebootPending")]
    public void TheWireNamesAreTheProtocolOnes(PawnIoStatus status, string wire) =>
        Assert.Equal(wire, PawnIoClassifier.ToWire(status));

    [Theory]
    [InlineData("133000000000000001", 133_000_000_000_000_001L)]
    [InlineData(" 133000000000000001 ", 133_000_000_000_000_001L)]
    public void MarkerIsParsedFromADecimalString(string raw, long expected) =>
        Assert.Equal(expected, PawnIoClassifier.ParseMarker(raw));

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("soon")]
    [InlineData("0x10")]
    [InlineData("12.5")]
    [InlineData("-5")]
    [InlineData("0")]
    [InlineData("99999999999999999999999")]
    public void MarkerThatIsNotANumberIsIgnored(string? raw) => Assert.Null(PawnIoClassifier.ParseMarker(raw));
}
