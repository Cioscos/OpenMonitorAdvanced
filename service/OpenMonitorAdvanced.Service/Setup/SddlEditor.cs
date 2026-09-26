using System.Globalization;

namespace OpenMonitorAdvanced.Service.Setup;

/// <summary>
/// Grants Interactive Users (<c>IU</c>) the right to start and stop the service, without widening
/// any other right on any other ACE. The SDDL string is parsed structurally into its DACL ACEs
/// (type, flags, rights mask, object/inherit GUIDs and SID) and re-emitted; every ACE that is not
/// an <c>IU</c> allow ACE, and the whole <c>O:</c>/<c>G:</c>/<c>S:</c> parts, are copied byte for
/// byte. See spec §2.2/§8/§9 and <c>docs/superpowers/references/m4/s3-pipe-scm.md</c>.
/// </summary>
public static class SddlEditor
{
    private const string InteractiveUsersSid = "IU";
    private const string AllowAceType = "A";

    /// <summary>ACE appended when the DACL has no <c>IU</c> allow ACE at all.</summary>
    private const string NewInteractiveAce = "(A;;CCLCSWRPWPLOCRRC;;;IU)";

    /// <summary>Canonical emission order for the symbolic rights that matter to this service (brief order).</summary>
    private static readonly string[] CanonicalRightOrder =
    [
        "CC", "DC", "LC", "SW", "RP", "WP", "DT", "LO", "CR", "SD", "RC", "WD", "WO",
    ];

    /// <summary>Rights removed from every IU allow ACE: the dangerous set plus the generic rights.</summary>
    private static readonly HashSet<string> ForbiddenRights = ["DC", "SD", "WD", "WO", "DT", "GA", "GW"];

    private static readonly HashSet<string> RightsToAdd = ["RP", "WP"];

    /// <summary>Hex-mask equivalent of <see cref="ForbiddenRights"/> (standard + generic bits from the brief).</summary>
    private const uint ForbiddenMaskBits = 0x2 | 0x40 | 0x10000 | 0x40000 | 0x80000 | 0x10000000 | 0x40000000;

    /// <summary>Hex-mask equivalent of <see cref="RightsToAdd"/> (RP=0x10, WP=0x20).</summary>
    private const uint MaskBitsToAdd = 0x30;

    public static string GrantStartStopToInteractiveUsers(string sddl)
    {
        ArgumentNullException.ThrowIfNull(sddl);

        var sections = FindTopLevelSections(sddl);
        var daclIndex = sections.FindIndex(s => s.Letter == 'D');
        if (daclIndex < 0)
        {
            throw new InvalidOperationException("The SDDL string has no DACL (D:) section.");
        }

        var contentStart = sections[daclIndex].Index + 2; // skip "D:"
        var contentEnd = daclIndex + 1 < sections.Count ? sections[daclIndex + 1].Index : sddl.Length;
        var daclContent = sddl[contentStart..contentEnd];

        var firstParen = daclContent.IndexOf('(');
        var flags = firstParen < 0 ? daclContent : daclContent[..firstParen];
        var aceListText = firstParen < 0 ? string.Empty : daclContent[firstParen..];

        if (flags.Contains("NO_ACCESS_CONTROL", StringComparison.Ordinal))
        {
            throw new InvalidOperationException("The SDDL string has a NULL DACL (D:NO_ACCESS_CONTROL).");
        }

        var aces = SplitTopLevelParenGroups(aceListText);
        var foundInteractiveAllowAce = false;
        var rebuilt = new List<string>(aces.Count + 1);
        foreach (var ace in aces)
        {
            rebuilt.Add(ProcessAce(ace, ref foundInteractiveAllowAce));
        }

        if (!foundInteractiveAllowAce)
        {
            rebuilt.Add(NewInteractiveAce);
        }

        var newDaclContent = flags + string.Concat(rebuilt);
        return sddl[..contentStart] + newDaclContent + sddl[contentEnd..];
    }

    private static string ProcessAce(string aceWithParens, ref bool foundInteractiveAllowAce)
    {
        // Strip the outer parentheses to get "type;flags;rights;objType;inheritType;sid".
        var inner = aceWithParens[1..^1];
        var fields = inner.Split(';');
        if (fields.Length != 6)
        {
            // Conditional/callback ACE strings (XA, XD, ZA, ...) can carry a 7th field whose
            // condition expression may itself contain ';'. Leave anything that does not match
            // the plain 6-field ACE shape untouched rather than risk corrupting it.
            return aceWithParens;
        }

        if (fields[0] != AllowAceType || fields[5] != InteractiveUsersSid)
        {
            return aceWithParens;
        }

        foundInteractiveAllowAce = true;
        var newRights = TransformRights(fields[2]);
        return $"({fields[0]};{fields[1]};{newRights};{fields[3]};{fields[4]};{fields[5]})";
    }

    private static string TransformRights(string rights)
    {
        if (rights.StartsWith("0x", StringComparison.OrdinalIgnoreCase))
        {
            var value = uint.Parse(rights[2..], NumberStyles.HexNumber, CultureInfo.InvariantCulture);
            value &= ~ForbiddenMaskBits;
            value |= MaskBitsToAdd;
            return "0x" + value.ToString("x", CultureInfo.InvariantCulture);
        }

        // Every standard/generic SDDL right abbreviation is exactly two characters.
        var tokens = new List<string>(rights.Length / 2);
        for (var i = 0; i + 1 < rights.Length; i += 2)
        {
            tokens.Add(rights.Substring(i, 2));
        }

        var kept = new List<string>(tokens.Count + 2);
        var seen = new HashSet<string>();
        foreach (var token in tokens)
        {
            if (ForbiddenRights.Contains(token) || !seen.Add(token))
            {
                continue;
            }

            kept.Add(token);
        }

        foreach (var toAdd in RightsToAdd)
        {
            if (seen.Add(toAdd))
            {
                kept.Add(toAdd);
            }
        }

        var ordered = new List<string>(kept.Count);
        foreach (var canonical in CanonicalRightOrder)
        {
            if (kept.Contains(canonical))
            {
                ordered.Add(canonical);
            }
        }

        foreach (var extra in kept)
        {
            if (!CanonicalRightOrder.Contains(extra))
            {
                ordered.Add(extra);
            }
        }

        return string.Concat(ordered);
    }

    /// <summary>
    /// Locates every top-level (paren-depth 0) SDDL section marker (<c>O:</c>, <c>G:</c>, <c>D:</c>,
    /// <c>S:</c>), in the order they appear in the string.
    /// </summary>
    private static List<(char Letter, int Index)> FindTopLevelSections(string sddl)
    {
        var result = new List<(char, int)>();
        var depth = 0;
        for (var i = 0; i < sddl.Length; i++)
        {
            var c = sddl[i];
            if (c == '(')
            {
                depth++;
            }
            else if (c == ')')
            {
                depth--;
            }
            else if (depth == 0 && i + 1 < sddl.Length && sddl[i + 1] == ':' &&
                     (c is 'O' or 'G' or 'D' or 'S'))
            {
                result.Add((c, i));
            }
        }

        return result;
    }

    /// <summary>Splits a string of consecutive <c>(...)</c> groups, respecting nested parentheses.</summary>
    private static List<string> SplitTopLevelParenGroups(string s)
    {
        var groups = new List<string>();
        var depth = 0;
        var start = -1;
        for (var i = 0; i < s.Length; i++)
        {
            if (s[i] == '(')
            {
                if (depth == 0)
                {
                    start = i;
                }

                depth++;
            }
            else if (s[i] == ')')
            {
                depth--;
                if (depth == 0)
                {
                    groups.Add(s[start..(i + 1)]);
                }
            }
        }

        return groups;
    }
}
