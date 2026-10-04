using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// <see cref="DriveStates"/>: the state the service block reports for a drive (design M6b §3.1)
/// and which drives have their SMART off (§4.2).
/// </summary>
public sealed class DriveStatesTests
{
    private const string Key = "589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d83";
    private const string OtherKey = "0000000000000000000000000000000000000000000000000000000000000001";

    private static DriveFacts Hdd(DriveAvailability availability = DriveAvailability.Present) =>
        new(0, availability, "ST2000DM008-2FR102", "ZFL0", BusType: 0x0B, SeekPenalty: true);

    private static DriveFacts Nvme() =>
        new(2, DriveAvailability.Present, "Fanxiang S880 2TB", "NVME", DriveFacts.BusTypeNvme, SeekPenalty: false);

    private static DriveFacts Usb() =>
        new(4, DriveAvailability.Present, "SanDisk Extreme", "4C53", DriveFacts.BusTypeUsb, SeekPenalty: null);

    private static EffectiveConfig Config(ServiceModules enabled = ServiceModules.All, string[]? disabled = null, string[]? smartOn = null) =>
        new(enabled, new HashSet<string>(disabled ?? [], StringComparer.Ordinal), new HashSet<string>(smartOn ?? [], StringComparer.Ordinal));

    [Theory]
    // No media wins over everything, whatever was asked or answered.
    [InlineData(DriveAvailability.NoMedia, true, true, true, "noMedia")]
    [InlineData(DriveAvailability.NoMedia, false, true, false, "noMedia")]
    [InlineData(DriveAvailability.NoMedia, false, false, null, "noMedia")]
    // SMART off hides the power answer (which may still be what keeps the gate closed).
    [InlineData(DriveAvailability.Present, true, true, true, "smartOff")]
    [InlineData(DriveAvailability.Present, true, false, null, "smartOff")]
    // Then the power answer.
    [InlineData(DriveAvailability.Present, false, true, true, "standby")]
    [InlineData(DriveAvailability.Present, false, true, false, "active")]
    [InlineData(DriveAvailability.Present, false, true, null, "unknown")]
    // A rotational drive that was not asked is unknown, never active.
    [InlineData(DriveAvailability.Present, false, false, null, "unknown")]
    [InlineData(DriveAvailability.Unreadable, false, false, null, "unknown")]
    public void TheStateOfADriveThatNeedsAPowerCheckFollowsThePrecedence(DriveAvailability availability, bool smartOff, bool asked, bool? spunDown, string expected) =>
        Assert.Equal(expected, DriveStates.Of(new DriveCheck(Hdd(availability), asked, spunDown), smartOff));

    [Theory]
    // Windows turned the disk off: standby, without asking it.
    [InlineData(DriveAvailability.Present, false, true, false, "standby")]
    // On, and not asked for lack of recent activity.
    [InlineData(DriveAvailability.Present, false, false, true, "idle")]
    // No media and SMART off still come first.
    [InlineData(DriveAvailability.NoMedia, false, true, false, "noMedia")]
    [InlineData(DriveAvailability.NoMedia, false, false, true, "noMedia")]
    [InlineData(DriveAvailability.Present, true, true, false, "smartOff")]
    [InlineData(DriveAvailability.Present, true, false, true, "smartOff")]
    public void ADriveThatIsLeftAloneIsStandbyWhenWindowsTurnedItOffAndIdleOtherwise(DriveAvailability availability, bool smartOff, bool poweredOff, bool idle, string expected) =>
        Assert.Equal(expected, DriveStates.Of(new DriveCheck(Hdd(availability), Asked: false, SpunDown: null) { PoweredOff = poweredOff, Idle = idle }, smartOff));

    [Fact]
    public void OnlyADriveKnownToBeActiveDoesNotBlockAndOnlyALeftAloneOrSleepingOneRests()
    {
        var unasked = new DriveCheck(Hdd(), Asked: false, SpunDown: null);
        Assert.Equal((false, false), (unasked.Blocks, unasked.Rests));
        Assert.Equal((true, true), ((unasked with { PoweredOff = true }).Blocks, (unasked with { PoweredOff = true }).Rests));
        Assert.Equal((true, true), ((unasked with { Idle = true }).Blocks, (unasked with { Idle = true }).Rests));
        Assert.Equal((true, true), (new DriveCheck(Hdd(), Asked: true, SpunDown: true).Blocks, new DriveCheck(Hdd(), Asked: true, SpunDown: true).Rests));
        Assert.Equal((true, false), (new DriveCheck(Hdd(), Asked: true, SpunDown: null).Blocks, new DriveCheck(Hdd(), Asked: true, SpunDown: null).Rests));
        Assert.Equal((false, false), (new DriveCheck(Hdd(), Asked: true, SpunDown: false).Blocks, new DriveCheck(Hdd(), Asked: true, SpunDown: false).Rests));
    }

    [Fact]
    public void ADriveThatNeedsNoPowerCheckIsActiveWithoutBeingAsked()
    {
        Assert.False(Nvme().RequiresPowerCheck);
        Assert.Equal("active", DriveStates.Of(new DriveCheck(Nvme(), Asked: false, SpunDown: null), smartOff: false));
        Assert.Equal("smartOff", DriveStates.Of(new DriveCheck(Nvme(), Asked: false, SpunDown: null), smartOff: true));
    }

    [Fact]
    public void OnlyAUsbDiskIsOffByDefault()
    {
        Assert.Equal(0x07u, DriveFacts.BusTypeUsb);
        Assert.True(Usb().SmartOffByDefault);
        Assert.False(Hdd().SmartOffByDefault);
        Assert.False(Nvme().SmartOffByDefault);
        Assert.False((Usb() with { BusType = null }).SmartOffByDefault);
    }

    [Fact]
    public void SmartIsOffWithStorageOffOrWhenTheKeyIsDisabled()
    {
        Assert.False(DriveStates.IsSmartOff(Hdd(), Key, Config()));
        Assert.True(DriveStates.IsSmartOff(Hdd(), Key, Config(ServiceModules.All & ~ServiceModules.Storage)));
        Assert.True(DriveStates.IsSmartOff(Hdd(), Key, Config(disabled: [Key])));
        Assert.False(DriveStates.IsSmartOff(Hdd(), Key, Config(disabled: [OtherKey])));
        Assert.False(DriveStates.IsSmartOff(Hdd(), key: null, Config(disabled: [Key]))); // nothing to match
    }

    [Fact]
    public void ADefaultOffDriveIsOnOnlyWhenItsKeyIsEnabled()
    {
        Assert.True(DriveStates.IsSmartOff(Usb(), Key, Config()));
        Assert.True(DriveStates.IsSmartOff(Usb(), Key, Config(smartOn: [OtherKey])));
        Assert.False(DriveStates.IsSmartOff(Usb(), Key, Config(smartOn: [Key])));
        Assert.True(DriveStates.IsSmartOff(Usb(), key: null, Config(smartOn: [Key]))); // without a key it cannot be switched on
        Assert.True(DriveStates.IsSmartOff(Usb(), Key, Config(ServiceModules.None, smartOn: [Key])));
        Assert.True(DriveStates.IsSmartOff(Usb(), Key, Config(disabled: [Key], smartOn: [Key])));
        Assert.False(DriveStates.IsSmartOff(Hdd(), Key, Config(smartOn: [OtherKey]))); // the list only concerns default-off drives
    }
}
