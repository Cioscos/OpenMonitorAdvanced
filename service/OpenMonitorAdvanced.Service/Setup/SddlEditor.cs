using System.Globalization;

namespace OpenMonitorAdvanced.Service.Setup;

/// <summary>
/// Grants Interactive Users (<c>IU</c>) the right to start and stop the service, without widening
/// any other right on any other ACE. The SDDL string is parsed structurally into its DACL ACEs
/// (type, flags, rights mask, object/inherit GUIDs and SID) and re-emitted; every ACE that is not
/// a plain <c>A</c> (allow) IU ACE, and the whole <c>O:</c>/<c>G:</c>/<c>S:</c> parts, are copied
/// byte for byte. Every rights token — including the file/registry composite masks Windows'
/// converter can emit (FA/FR/FW/FX/KA/KR/KW/KX) — is expanded to its numeric bits before the
/// forbidden-rights filter runs, so a composite that happens to include a forbidden bit (e.g. FA
/// includes DC/SD/WD/WO) can never survive as a symbolic token. See spec §2.2/§8/§9 and
/// <c>docs/superpowers/references/m4/s3-pipe-scm.md</c>.
/// </summary>
public static class SddlEditor
{
    private const string AllowAceType = "A";

    /// <summary>ACE appended when the DACL has no <c>IU</c> allow ACE at all.</summary>
    private const string NewInteractiveAce = "(A;;CCLCSWRPWPLOCRRC;;;IU)";

    /// <summary>
    /// Elementary (single-bit) SDDL right letters, in the order they must be emitted when the
    /// result can be expressed symbolically. The service-specific letters (CC..WO) come from the
    /// brief's canonical order; the generic letters (GA/GR/GW/GX) are appended after them, since
    /// the brief's canonical order does not define a position for them.
    /// </summary>
    private static readonly (string Letter, uint Bit)[] CanonicalElementaryOrder =
    [
        ("CC", 0x1), ("DC", 0x2), ("LC", 0x4), ("SW", 0x8),
        ("RP", 0x10), ("WP", 0x20), ("DT", 0x40), ("LO", 0x80), ("CR", 0x100),
        ("SD", 0x10000), ("RC", 0x20000), ("WD", 0x40000), ("WO", 0x80000),
        ("GA", 0x10000000), ("GR", 0x80000000), ("GW", 0x40000000), ("GX", 0x20000000),
    ];

    /// <summary>
    /// Every symbolic token this editor understands on an IU allow ACE's rights mask, mapped to
    /// its numeric value: the elementary service/generic rights, plus the file/registry composite
    /// masks the Windows SDDL converter emits when a mask exactly matches one of them. Values
    /// verified against winnt.h (FILE_ALL_ACCESS, FILE_GENERIC_*, KEY_*).
    /// </summary>
    private static readonly Dictionary<string, uint> KnownRights = BuildKnownRights();

    private static Dictionary<string, uint> BuildKnownRights()
    {
        var rights = new Dictionary<string, uint>(StringComparer.Ordinal);
        foreach (var (letter, bit) in CanonicalElementaryOrder)
        {
            rights[letter] = bit;
        }

        rights["FA"] = 0x1F01FF; // FILE_ALL_ACCESS
        rights["FR"] = 0x120089; // FILE_GENERIC_READ
        rights["FW"] = 0x120116; // FILE_GENERIC_WRITE
        rights["FX"] = 0x1200A0; // FILE_GENERIC_EXECUTE
        rights["KA"] = 0xF003F; // KEY_ALL_ACCESS
        rights["KR"] = 0x20019; // KEY_READ
        rights["KW"] = 0x20006; // KEY_WRITE
        rights["KX"] = 0x20019; // KEY_EXECUTE (same value as KEY_READ)
        return rights;
    }

    /// <summary>
    /// Rights removed from every IU allow ACE: the dangerous service rights (DC, SD, WD, WO, DT)
    /// plus the generic rights that imply them on a service (GA, GW; GX implies SERVICE_PAUSE_CONTINUE
    /// = DT per ruling R14). GR is intentionally not forbidden.
    /// </summary>
    private const uint ForbiddenMaskBits =
        0x2 /* DC */ | 0x40 /* DT */ | 0x10000 /* SD */ | 0x40000 /* WD */ | 0x80000 /* WO */
        | 0x10000000 /* GA */ | 0x40000000 /* GW */ | 0x20000000 /* GX */;

    /// <summary>RP (0x10) | WP (0x20): the only rights ever added.</summary>
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

        var aces = SplitTopLevelParenGroups(aceListText, sddl);
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

    private static bool IsInteractiveUsersSid(string sid) =>
        string.Equals(sid, "IU", StringComparison.OrdinalIgnoreCase) ||
        string.Equals(sid, "S-1-5-4", StringComparison.OrdinalIgnoreCase);

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

        if (fields[0] != AllowAceType || !IsInteractiveUsersSid(fields[5]))
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
            uint value;
            try
            {
                value = uint.Parse(rights[2..], NumberStyles.HexNumber, CultureInfo.InvariantCulture);
            }
            catch (Exception ex) when (ex is FormatException or OverflowException)
            {
                throw new InvalidOperationException($"Malformed hex rights mask '{rights}'.", ex);
            }

            value = ApplyFilter(value);
            return "0x" + value.ToString("x", CultureInfo.InvariantCulture);
        }

        // Every SDDL right abbreviation (elementary or composite) is exactly two characters.
        var mask = 0u;
        for (var i = 0; i + 1 < rights.Length; i += 2)
        {
            var token = rights.Substring(i, 2);
            if (!KnownRights.TryGetValue(token, out var bit))
            {
                throw new InvalidOperationException(
                    $"Unknown SDDL right '{token}' on an Interactive Users allow ACE (rights '{rights}').");
            }

            mask |= bit;
        }

        mask = ApplyFilter(mask);
        return FormatSymbolicOrHex(mask);
    }

    private static uint ApplyFilter(uint mask)
    {
        mask &= ~ForbiddenMaskBits;
        mask |= MaskBitsToAdd;
        return mask;
    }

    /// <summary>
    /// Emits <paramref name="mask"/> as a sequence of elementary right letters, in canonical
    /// order, only if every set bit is covered by one of them; otherwise emits hex, because a
    /// composite input (e.g. FA) can leave bits with no elementary letter (e.g. SYNCHRONIZE).
    /// </summary>
    private static string FormatSymbolicOrHex(uint mask)
    {
        var letters = new List<string>();
        var remaining = mask;
        foreach (var (letter, bit) in CanonicalElementaryOrder)
        {
            if ((remaining & bit) == bit)
            {
                letters.Add(letter);
                remaining &= ~bit;
            }
        }

        return remaining == 0
            ? string.Concat(letters)
            : "0x" + mask.ToString("x", CultureInfo.InvariantCulture);
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

    /// <summary>
    /// Splits a string that must consist solely of consecutive <c>(...)</c> groups (respecting
    /// nested parentheses), such as a DACL's ACE list. Anything else — stray text between groups,
    /// or an unterminated trailing group — is a malformed DACL and throws rather than being
    /// silently dropped.
    /// </summary>
    private static List<string> SplitTopLevelParenGroups(string s, string originalSddl)
    {
        var groups = new List<string>();
        var i = 0;
        while (i < s.Length)
        {
            if (s[i] != '(')
            {
                throw new InvalidOperationException(
                    $"Malformed DACL: expected '(' at position {i} of the ACE list, found '{s[i]}' in '{originalSddl}'.");
            }

            var start = i;
            var depth = 0;
            do
            {
                if (s[i] == '(')
                {
                    depth++;
                }
                else if (s[i] == ')')
                {
                    depth--;
                }

                i++;
            }
            while (depth > 0 && i < s.Length);

            if (depth != 0)
            {
                throw new InvalidOperationException(
                    $"Malformed DACL: unterminated ACE starting at position {start} in '{originalSddl}'.");
            }

            groups.Add(s[start..i]);
        }

        return groups;
    }
}
