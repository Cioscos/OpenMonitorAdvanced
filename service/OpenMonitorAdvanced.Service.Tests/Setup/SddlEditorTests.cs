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
    public void PreservesAceFlags()
    {
        const string input = "D:(A;OICI;CCLCSWLOCRRC;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;OICI;CCLCSWRPWPLOCRRC;;;IU)", result);
    }

    [Fact]
    public void PreservesObjectAceWithGuidsByteForByte()
    {
        const string input =
            "D:(OA;;CCLCSWRPWPLOCRRC;bf967aba-0de6-11d0-a285-00aa003049e2;;IU)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        // OA is not a plain "A" ACE (it carries object/inherit-object GUIDs); it must pass through
        // untouched, and since there is no plain IU allow ACE the default one is appended.
        Assert.Equal(input + "(A;;CCLCSWRPWPLOCRRC;;;IU)", result);
    }

    [Fact]
    public void PreservesConditionalAceWithNestedParenthesesByteForByte()
    {
        const string input = "D:(XA;;CCLCSWRPWPLOCRRC;;;IU;(Member_of{SID(WD)}))(A;;CCLCSWLOCRRC;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal(
            "D:(XA;;CCLCSWRPWPLOCRRC;;;IU;(Member_of{SID(WD)}))(A;;CCLCSWRPWPLOCRRC;;;IU)",
            result);
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

    // --- Fix round 1: composite (FA/FR/FW/FX/KA/KR/KW/KX) rights must be expanded to their bits,
    // filtered like every other IU allow ACE, then re-emitted symbolically only if every remaining
    // bit still has a service letter, hex otherwise. Never keep the composite token itself, since
    // e.g. FA = 0x1F01FF includes DC/SD/WD/WO (privilege escalation if left as "FA").

    [Fact]
    public void ExpandsFullAccessCompositeAndEmitsHexWhenNotFullyDecomposable()
    {
        // FA = 0x1F01FF; after stripping the forbidden bits and adding RP|WP, 0x100000
        // (SYNCHRONIZE) has no service letter, so the result must be hex, not "...FA".
        const string input = "D:(A;;FA;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;;0x1201bd;;;IU)", result);
    }

    [Fact]
    public void ExpandsKeyAllAccessCompositeAndEmitsSymbolicWhenFullyDecomposable()
    {
        // KA = 0xF003F; after filtering, every remaining bit (CC LC SW RP WP RC) has a service
        // letter, so the result is symbolic.
        const string input = "D:(A;;KA;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;;CCLCSWRPWPRC;;;IU)", result);
    }

    [Fact]
    public void ExpandsFileGenericWriteCompositeAndEmitsHex()
    {
        // FW = FILE_GENERIC_WRITE = 0x120116, which already has DC (0x2) set; after removing it
        // and adding RP|WP (0x30), 0x100000 (SYNCHRONIZE) still has no service letter, so hex.
        const string input = "D:(A;;FW;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;;0x120134;;;IU)", result);
    }

    [Fact]
    public void ExpandsKeyWriteCompositeAndEmitsSymbolic()
    {
        const string input = "D:(A;;KW;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;;LCRPWPRC;;;IU)", result);
    }

    [Fact]
    public void ThrowsOnAnUnmappableSymbolicToken()
    {
        const string input = "D:(A;;ZZ;;;IU)";

        Assert.Throws<InvalidOperationException>(() => SddlEditor.GrantStartStopToInteractiveUsers(input));
    }

    [Fact]
    public void StripsGenericExecuteFromTheInteractiveAce()
    {
        // R14: GENERIC_EXECUTE on a service implies SERVICE_PAUSE_CONTINUE (DT), forbidden.
        const string input = "D:(A;;GX;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;;RPWP;;;IU)", result);
    }

    [Fact]
    public void KeepsGenericReadOnTheInteractiveAce()
    {
        const string input = "D:(A;;GRDT;;;IU)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;;RPWPGR;;;IU)", result);
    }

    [Fact]
    public void MatchesInteractiveUsersCaseInsensitively()
    {
        const string input = "D:(A;;CCLCSWLOCRRC;;;iu)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;;CCLCSWRPWPLOCRRC;;;iu)", result);
    }

    [Fact]
    public void MatchesInteractiveUsersBySidString()
    {
        const string input = "D:(A;;CCLCSWLOCRRC;;;S-1-5-4)";

        var result = SddlEditor.GrantStartStopToInteractiveUsers(input);

        Assert.Equal("D:(A;;CCLCSWRPWPLOCRRC;;;S-1-5-4)", result);
    }

    [Fact]
    public void ThrowsOnTextBetweenAceGroups()
    {
        const string input = "D:(A;;GA;;;SY)X(A;;CCLCSWLOCRRC;;;IU)";

        Assert.Throws<InvalidOperationException>(() => SddlEditor.GrantStartStopToInteractiveUsers(input));
    }

    [Fact]
    public void ThrowsOnAnUnterminatedTrailingAce()
    {
        const string input = "D:(A;;GA;;;SY)(A;;CCLCSWLOCRRC;;;IU";

        Assert.Throws<InvalidOperationException>(() => SddlEditor.GrantStartStopToInteractiveUsers(input));
    }

    [Fact]
    public void ThrowsOnAMalformedHexMask()
    {
        const string input = "D:(A;;0xZZZZ;;;IU)";

        Assert.Throws<InvalidOperationException>(() => SddlEditor.GrantStartStopToInteractiveUsers(input));
    }
}
