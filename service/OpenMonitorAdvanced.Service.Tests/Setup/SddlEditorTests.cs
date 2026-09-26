using OpenMonitorAdvanced.Service.Setup;

using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Setup;

public sealed class SddlEditorTests
{
    // Windows 11 default service descriptor, verified with `sc.exe sdshow`.
    private const string DefaultWindows11Sddl =
        "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)";

    [Fact]
    public void AddsStartAndStopToTheInteractiveAce()
    {
        var result = SddlEditor.GrantStartStopToInteractiveUsers(DefaultWindows11Sddl);

        Assert.Equal(
            "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWRPWPLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)",
            result);
    }

    [Fact]
    public void KeepsTheSaclUntouched()
    {
        const string input = DefaultWindows11Sddl + "S:(AU;FA;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;WD)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.EndsWith("S:(AU;FA;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;WD)", result);
    }

    [Fact]
    public void IsIdempotent()
    {
        var once = SddlEditor.GrantStartStopToInteractiveUsers(DefaultWindows11Sddl);
        var twice = SddlEditor.GrantStartStopToInteractiveUsers(once);

        Assert.Equal(once, twice);
    }

    [Fact]
    public void AppendsAnInteractiveAceWhenMissing()
    {
        const string input = "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal(input + "(A;;CCLCSWRPWPLOCRRC;;;IU)", result);
    }

    [Fact]
    public void AppendsAnInteractiveAceWhenMissingBeforeTheSacl()
    {
        const string input = "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)S:(AU;FA;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;WD)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal(
            "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCLCSWRPWPLOCRRC;;;IU)S:(AU;FA;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;WD)",
            result);
    }

    [Fact]
    public void RemovesDangerousRightsFromTheInteractiveAce()
    {
        const string input = "D:(A;;CCLCSWDTSDWDWOLOCRRC;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;;CCLCSWRPWPLOCRRC;;;IU)", result);
    }

    [Fact]
    public void HandlesAHexMask()
    {
        const string input = "D:(A;;0x2008d;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;;0x200bd;;;IU)", result);
    }

    [Fact]
    public void RemovesGenericRightsFromEveryInteractiveAllowAce()
    {
        // The GA/GW bits (0x10000000 / 0x40000000) and every other forbidden bit must be
        // stripped from *every* IU allow ACE, symbolic and hex alike; other SIDs are untouched.
        // 0x500d0043 = every forbidden bit (0x2|0x40|0x10000|0x40000|0x80000|0x10000000|0x40000000)
        // plus 0x1; after clearing the forbidden bits and adding RP|WP (0x30) only 0x31 remains.
        const string input =
            "D:(A;;CCLCSWGADTGWLOCRRC;;;IU)(A;;0x500d0043;;;IU)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal(
            "D:(A;;CCLCSWRPWPLOCRRC;;;IU)(A;;0x31;;;IU)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)",
            result);
    }

    [Fact]
    public void LeavesDenyAcesAlone()
    {
        const string input = "D:(D;;WP;;;IU)(A;;CCLCSWLOCRRC;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(D;;WP;;;IU)(A;;CCLCSWRPWPLOCRRC;;;IU)", result);
    }

    [Fact]
    public void PreservesAceFlagsAndObjectGuidFields()
    {
        const string input = "D:(A;OICI;CCLCSWLOCRRC;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;OICI;CCLCSWRPWPLOCRRC;;;IU)", result);
    }

    [Fact]
    public void PreservesLeadingOwnerAndGroupAndDaclControlFlags()
    {
        const string input = "O:SYG:SYD:P(A;;CCLCSWLOCRRC;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("O:SYG:SYD:P(A;;CCLCSWRPWPLOCRRC;;;IU)", result);
    }

    [Fact]
    public void ThrowsOnMissingDacl()
    {
        const string input = "O:SYG:SY";

        Assert.Throws<InvalidOperationException>(() => SddlEditor.GrantStartStopToInteractiveUsers(input));
    }

    [Fact]
    public void ThrowsOnNullDacl()
    {
        const string input = "O:SYG:SYD:NO_ACCESS_CONTROL";

        Assert.Throws<InvalidOperationException>(() => SddlEditor.GrantStartStopToInteractiveUsers(input));
    }
}
