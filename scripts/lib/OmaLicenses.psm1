#Requires -Version 7
# Pure helpers of scripts/generate-licenses.ps1 (spec M6c §4): NuGet runtime packages from
# project.assets.json, licence of a NuGet package from its nuspec, SPDX expression check,
# merge and rendering of THIRD_PARTY_LICENSES.txt. No network, no external commands. Every
# ordering is ordinal, never culture-dependent, so the output is the same on every machine.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

# Sections of the generated file, in this order; any other ecosystem follows, by name.
$script:EcosystemOrder = @('Rust', 'JavaScript', '.NET', 'Programs', 'Adapted', 'Fonts')
$script:SectionTitles = @{
    'Rust' = 'Rust crates'; 'JavaScript' = 'JavaScript packages'; '.NET' = '.NET packages'
    # Executables shipped unmodified next to the service (M7b: Intel PresentMon), with what they embed.
    'Programs' = 'Programs shipped with the service'
    # Third-party source code adapted into our own crates (M8a1: OpenDCDiag, M8b1: memtest_vulkan, in oma-load).
    'Adapted' = 'Source code adapted into OpenMonitor Advanced'
    # Fonts bundled in the app for the CPU benchmark gauges (M8a2).
    'Fonts' = 'Fonts bundled in the app'
}
$script:Rule = '-' * 79

<#
.SYNOPSIS
  The NuGet packages of project.assets.json that put something in the build output for a target.
.DESCRIPTION
  -Target is a key of "targets" (for example 'net10.0-windows/win-x64') or a runtime
  identifier ('win-x64'), which must then match exactly one key ending in '/<rid>'. A package is
  kept when it has at least one 'runtime', 'native' or 'runtimeTargets' asset other than the
  '_._' placeholder: analyzers and build-only packages have none, and project references are
  not packages. Returns @{ Id; Version } sorted by id.
#>
function Get-OmaNuGetRuntimePackages {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$AssetsJson,
        [Parameter(Mandatory)] [string]$Target
    )
    $assets = [IO.File]::ReadAllText($AssetsJson) | ConvertFrom-Json -AsHashtable
    $targets = $assets['targets']
    $key = $null
    if ($targets.ContainsKey($Target)) {
        $key = $Target
    } else {
        $found = @($targets.Keys | Where-Object { $_.EndsWith("/$Target", [StringComparison]::Ordinal) })
        if ($found.Count -ne 1) {
            throw "project.assets.json has $($found.Count) targets for '$Target' (expected one): $($targets.Keys -join ', ')"
        }
        $key = $found[0]
    }
    $result = [Collections.Generic.List[object]]::new()
    foreach ($name in $targets[$key].Keys) {
        $lib = $targets[$key][$name]
        if ($lib['type'] -ne 'package') { continue }
        $real = $false
        foreach ($group in 'runtime', 'native', 'runtimeTargets') {
            if (-not $lib.ContainsKey($group)) { continue }
            foreach ($asset in $lib[$group].Keys) {
                if (($asset -split '/')[-1] -ne '_._') { $real = $true }
            }
        }
        if (-not $real) { continue }
        $slash = $name.LastIndexOf('/')
        $result.Add([pscustomobject]@{ Id = $name.Substring(0, $slash); Version = $name.Substring($slash + 1) })
    }
    $sorted = $result.ToArray()
    [Array]::Sort($sorted, [Comparison[object]] { param($a, $b) Compare-OmaNameVersion $a.Id $a.Version $b.Id $b.Version })
    $sorted
}

<#
.SYNOPSIS
  Licence of a package in the NuGet global packages folder: @{ License; Copyright; Text; Notices }.
.DESCRIPTION
  Text is the package's own licence file (the nuspec's <license type="file"> or a LICENSE,
  LICENSE.txt or LICENSE.md at the package root), or $null when there is none and the caller
  must use the standard text of the expression. License is the nuspec's SPDX expression, or
  -Overrides[Id] when the nuspec has none (licence file or licenseUrl only); with neither it
  throws, so a new package cannot slip in without a reviewed licence.
#>
function Get-OmaNuGetPackageLicense {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$PackagesRoot,
        [Parameter(Mandatory)] [string]$Id,
        [Parameter(Mandatory)] [string]$Version,
        [hashtable]$Overrides = @{}
    )
    $dir = Join-Path $PackagesRoot (Join-Path $Id.ToLowerInvariant() $Version.ToLowerInvariant())
    $nuspec = Join-Path $dir "$($Id.ToLowerInvariant()).nuspec"
    if (-not (Test-Path -LiteralPath $nuspec)) { throw "nuspec not found for $Id $Version in $dir (run dotnet restore)" }
    $xml = [xml][IO.File]::ReadAllText($nuspec)
    $metadata = $xml.SelectSingleNode("/*[local-name()='package']/*[local-name()='metadata']")
    $licenseNode = $metadata.SelectSingleNode("*[local-name()='license']")
    $copyrightNode = $metadata.SelectSingleNode("*[local-name()='copyright']")

    $expression = $null
    $file = $null
    if ($licenseNode) {
        $value = $licenseNode.InnerText.Trim()
        switch ($licenseNode.GetAttribute('type')) {
            'expression' { $expression = $value }
            'file' { $file = Join-Path $dir $value }
        }
    }
    if (-not $file) {
        $file = Get-ChildItem -LiteralPath $dir -File |
            Where-Object { $_.Name -match '^licen[cs]e(\.(txt|md))?$' } |
            Sort-Object -Property { $_.Name.ToUpperInvariant() } |
            Select-Object -First 1 -ExpandProperty FullName
    }
    $notices = Get-ChildItem -LiteralPath $dir -File |
        Where-Object { $_.Name -ieq 'THIRD-PARTY-NOTICES.TXT' } |
        Select-Object -First 1 -ExpandProperty FullName
    if (-not $expression) {
        if ($Overrides.ContainsKey($Id)) { $expression = $Overrides[$Id] }
        else { throw "$Id $Version has no SPDX licence expression in its nuspec: review its terms and add an override" }
    }
    [pscustomobject]@{
        License   = $expression
        Copyright = if ($copyrightNode -and $copyrightNode.InnerText.Trim()) { $copyrightNode.InnerText.Trim() } else { $null }
        Text      = if ($file) { [IO.File]::ReadAllText($file) } else { $null }
        Notices   = if ($notices) { [IO.File]::ReadAllText($notices) } else { $null }
    }
}

<#
.SYNOPSIS
  True when an SPDX expression can be satisfied with the -Accepted licences.
.DESCRIPTION
  Supports OR, AND, parentheses and WITH (an "X WITH Y" term is accepted only when the exact
  "X WITH Y" is in the list), and the legacy "A/B" form, read as "A OR B". Comparison ignores
  case, as SPDX identifiers do.
#>
function Test-OmaLicenseAccepted {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$Expression,
        [Parameter(Mandatory)] [string[]]$Accepted
    )
    $set = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($a in $Accepted) { [void]$set.Add(($a -replace '\s+', ' ').Trim()) }
    $text = $Expression -replace '/', ' OR ' -replace '\(', ' ( ' -replace '\)', ' ) '
    $tokens = @($text -split '\s+' | Where-Object { $_ })
    $state = @{ Pos = 0 }

    $peek = { if ($state.Pos -lt $tokens.Count) { $tokens[$state.Pos] } else { $null } }
    $next = { $t = & $peek; $state.Pos++; $t }
    $parseExpr = $null
    $parseFactor = {
        $t = & $next
        if ($null -eq $t) { throw "incomplete licence expression: $Expression" }
        if ($t -eq '(') {
            $v = & $parseExpr
            if ((& $next) -ne ')') { throw "unbalanced parentheses in licence expression: $Expression" }
            return $v
        }
        if ((& $peek) -eq 'WITH') {
            [void](& $next)
            $exception = & $next
            return $set.Contains("$t WITH $exception")
        }
        $set.Contains($t)
    }
    $parseTerm = {
        $v = & $parseFactor
        while ((& $peek) -eq 'AND') { [void](& $next); $r = & $parseFactor; $v = $v -and $r }
        $v
    }
    $parseExpr = {
        $v = & $parseTerm
        while ((& $peek) -eq 'OR') { [void](& $next); $r = & $parseTerm; $v = $v -or $r }
        $v
    }
    $value = & $parseExpr
    if ($state.Pos -ne $tokens.Count) { throw "unexpected '$($tokens[$state.Pos])' in licence expression: $Expression" }
    [bool]$value
}

<#
.SYNOPSIS
  Merges per-ecosystem sections: sorted entries and each distinct licence text once.
.DESCRIPTION
  -Sections are @{ Ecosystem; Entries }, each entry @{ Ecosystem; Name; Version; License;
  Copyright; Texts = @(@{ Title; Body }) }. Sections of the same ecosystem are joined and
  ordered Rust, JavaScript, .NET, Programs, Adapted, then by name; entries are sorted by name (ordinal,
  ignoring case) and version (numeric segments compared as numbers), and the same name and
  version is one entry. Texts are compared after normalisation (LF line ends, no BOM, no trailing
  whitespace, no leading or trailing blank lines), so the same text from two packages is kept
  once. A text is labelled "<title> (<first user>)", which does not change when an unrelated
  dependency is added (see the label block below for collisions). Returns @{ Sections = @(@{ Ecosystem; Title;
  Entries = @(@{ Name; Version; License; Copyright; Refs }) }); Texts = @(@{ Label; Body }) }.
#>
function Merge-OmaLicenseSections {
    [CmdletBinding()]
    param([Parameter(Mandatory)] [AllowEmptyCollection()] [object[]]$Sections)

    $byEco = [ordered]@{}
    foreach ($s in $Sections) {
        if (-not $byEco.Contains($s.Ecosystem)) { $byEco[$s.Ecosystem] = [Collections.Generic.List[object]]::new() }
        foreach ($e in @($s.Entries)) { $byEco[$s.Ecosystem].Add($e) }
    }
    $ecos = [string[]]@($byEco.Keys)
    [Array]::Sort($ecos, [Comparison[string]] {
            param($a, $b)
            $ia = [Array]::IndexOf($script:EcosystemOrder, $a); if ($ia -lt 0) { $ia = $script:EcosystemOrder.Count }
            $ib = [Array]::IndexOf($script:EcosystemOrder, $b); if ($ib -lt 0) { $ib = $script:EcosystemOrder.Count }
            if ($ia -ne $ib) { return $ia.CompareTo($ib) }
            [string]::CompareOrdinal($a, $b)
        })

    # Distinct texts, keyed by normalised body (ordinal: [ordered]@{} would ignore case), with
    # the first entry that references each one.
    $texts = [Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
    $outSections = foreach ($eco in $ecos) {
        # Join duplicate entries (same name and version).
        $merged = [ordered]@{}
        foreach ($e in $byEco[$eco]) {
            $k = "$($e.Name)`n$($e.Version)"
            if (-not $merged.Contains($k)) {
                $merged[$k] = [pscustomobject]@{
                    Name = $e.Name; Version = $e.Version; License = $e.License
                    Copyright = [Collections.Generic.List[string]]::new(); Bodies = [Collections.Generic.List[object]]::new()
                }
            }
            $m = $merged[$k]
            if (-not $m.License) { $m.License = $e.License }
            if ($e.PSObject.Properties['Copyright'] -and $e.Copyright) {
                foreach ($line in ($e.Copyright -split "\r?\n")) {
                    $line = $line.Trim()
                    if ($line -and -not $m.Copyright.Contains($line)) { $m.Copyright.Add($line) }
                }
            }
            foreach ($t in @($e.Texts)) { $m.Bodies.Add($t) }
        }
        $entries = [object[]]@($merged.Values)
        [Array]::Sort($entries, [Comparison[object]] { param($a, $b) Compare-OmaNameVersion $a.Name $a.Version $b.Name $b.Version })

        $outEntries = foreach ($m in $entries) {
            $keys = [Collections.Generic.List[string]]::new()
            foreach ($t in $m.Bodies) {
                $body = ConvertTo-OmaNormalizedText $t.Body
                if (-not $texts.ContainsKey($body)) {
                    $texts[$body] = [pscustomobject]@{
                        Title = $t.Title; Body = $body; FirstName = $m.Name; FirstVersion = $m.Version; Label = $null
                        Neutral = $false
                    }
                }
                if ($t.PSObject.Properties['Neutral'] -and $t.Neutral) { $texts[$body].Neutral = $true }
                if (-not $keys.Contains($body)) { $keys.Add($body) }
            }
            [pscustomobject]@{
                Name = $m.Name; Version = $m.Version; License = $m.License
                Copyright = @($m.Copyright); RefKeys = $keys
            }
        }
        [pscustomobject]@{
            Ecosystem = $eco
            Title     = if ($script:SectionTitles.ContainsKey($eco)) { $script:SectionTitles[$eco] } else { "$eco components" }
            Entries   = @($outEntries)
        }
    }

    # Labels, stable when a dependency is added: "<title> (<first user>)", where the first user
    # is the first entry of the file that references the text. Should two texts collide, the
    # user's version is added, then the start of the text's SHA-256. A text marked Neutral (a
    # standard text of our own scripts/licenses folder) is labelled by its title alone, so a
    # component never points at a text named after an unrelated package.
    $all = [object[]]@($texts.Values)
    foreach ($t in $all) { $t.Label = if ($t.Neutral) { $t.Title } else { "$($t.Title) ($($t.FirstName))" } }
    foreach ($t in (Get-OmaCollidingTexts $all)) { $t.Label = "$($t.Title) ($($t.FirstName) $($t.FirstVersion))" }
    foreach ($t in (Get-OmaCollidingTexts $all)) {
        $hash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($t.Body)))
        $t.Label = "$($t.Title) ($($t.FirstName) $($t.FirstVersion) #$($hash.Substring(0, 8).ToLowerInvariant()))"
    }
    [Array]::Sort($all, [Comparison[object]] {
            param($a, $b)
            $c = [StringComparer]::OrdinalIgnoreCase.Compare($a.Label, $b.Label)
            if ($c -eq 0) { $c = [string]::CompareOrdinal($a.Label, $b.Label) }
            $c
        })

    foreach ($s in @($outSections)) {
        foreach ($e in $s.Entries) {
            $refs = [string[]]@($e.RefKeys | ForEach-Object { $texts[$_].Label })
            [Array]::Sort($refs, [StringComparer]::Ordinal)
            $e | Add-Member -NotePropertyName Refs -NotePropertyValue $refs
            $e.PSObject.Properties.Remove('RefKeys')
        }
    }
    [pscustomobject]@{
        Sections = @($outSections)
        Texts    = @($all | ForEach-Object { [pscustomobject]@{ Label = $_.Label; Body = $_.Body } })
    }
}

<#
.SYNOPSIS
  Renders THIRD_PARTY_LICENSES.txt from a flat list of entries (see Merge-OmaLicenseSections).
.DESCRIPTION
  The result depends only on the content of the entries, never on their order, and holds no
  date or path. Lines end with LF and the text ends with exactly one LF; write it as UTF-8
  without BOM.
#>
function ConvertTo-OmaLicenseText {
    [CmdletBinding()]
    param([Parameter(Mandatory)] [AllowEmptyCollection()] [object[]]$Entries)

    $sections = [ordered]@{}
    foreach ($e in $Entries) {
        if (-not $sections.Contains($e.Ecosystem)) { $sections[$e.Ecosystem] = [Collections.Generic.List[object]]::new() }
        $sections[$e.Ecosystem].Add($e)
    }
    $model = Merge-OmaLicenseSections -Sections @($sections.Keys | ForEach-Object {
            [pscustomobject]@{ Ecosystem = $_; Entries = $sections[$_].ToArray() } })

    $out = [Collections.Generic.List[string]]::new()
    $title = 'OpenMonitor Advanced - third-party licences'
    $out.Add($title); $out.Add('=' * $title.Length); $out.Add('')
    $out.Add('OpenMonitor Advanced is licensed under GPL-3.0-or-later (see LICENSE). It includes the')
    $out.Add('third-party components listed below: the Rust crates compiled into the application, the')
    $out.Add('JavaScript packages bundled into its user interface, the .NET packages and runtime built')
    $out.Add('into the oma-service hardware service, the programs shipped unmodified with that')
    $out.Add('service and the source code adapted into the oma-load program. Each component names, in')
    $out.Add('square brackets, the licence texts that apply to it;')
    $out.Add('every text is printed once, at the end of this file. Further notices are in')
    $out.Add('THIRD_PARTY_NOTICES.')
    $out.Add('')
    $out.Add('This file is generated by scripts/generate-licenses.ps1. Do not edit it by hand.')

    foreach ($s in $model.Sections) {
        $out.Add(''); $out.Add('')
        $heading = "$($s.Title) ($(@($s.Entries).Count))"
        $out.Add($heading); $out.Add('-' * $heading.Length)
        foreach ($e in $s.Entries) {
            $out.Add('')
            $out.Add("$($e.Name) $($e.Version)")
            $out.Add("    Licence: $($e.License)")
            foreach ($c in $e.Copyright) { $out.Add("    Copyright: $c") }
            $out.Add("    Text: $(($e.Refs | ForEach-Object { "[$_]" }) -join ' ')")
        }
    }

    $out.Add(''); $out.Add('')
    $out.Add('Licence texts'); $out.Add('=============')
    foreach ($t in $model.Texts) {
        $out.Add('')
        $out.Add($script:Rule); $out.Add("[$($t.Label)]"); $out.Add($script:Rule)
        $out.Add('')
        $out.Add($t.Body)
    }
    ($out -join "`n") + "`n"
}

# Texts whose label is shared with another text.
function Get-OmaCollidingTexts([object[]]$Texts) {
    $count = [Collections.Generic.Dictionary[string, int]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($t in $Texts) { $count[$t.Label] = 1 + $(if ($count.ContainsKey($t.Label)) { $count[$t.Label] } else { 0 }) }
    @($Texts | Where-Object { $count[$_.Label] -gt 1 })
}

<#
.SYNOPSIS
  True when two text files hold the same text, ignoring line ends, BOM and trailing whitespace.
.DESCRIPTION
  Used to compare a pinned copy under scripts/licenses/ with the file of the restored package.
#>
function Test-OmaPinnedText {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$Pinned,
        [Parameter(Mandatory)] [string]$Current
    )
    (ConvertTo-OmaNormalizedText ([IO.File]::ReadAllText($Pinned))) -ceq (ConvertTo-OmaNormalizedText ([IO.File]::ReadAllText($Current)))
}

# Text with LF line ends, no BOM, no trailing whitespace and no leading or trailing blank lines.
function ConvertTo-OmaNormalizedText([string]$Text) {
    $t = $Text.Replace("`r`n", "`n").Replace("`r", "`n").Replace([string][char]0xFEFF, '')
    $lines = $t -split "`n" | ForEach-Object { $_.TrimEnd() }
    ($lines -join "`n").Trim("`n")
}

# Ordinal name order (ignoring case, then exact), then version with numeric segments as numbers.
function Compare-OmaNameVersion([string]$NameA, [string]$VersionA, [string]$NameB, [string]$VersionB) {
    $c = [StringComparer]::OrdinalIgnoreCase.Compare($NameA, $NameB)
    if ($c -eq 0) { $c = [string]::CompareOrdinal($NameA, $NameB) }
    if ($c -ne 0) { return $c }
    $sa = $VersionA -split '[.+-]'
    $sb = $VersionB -split '[.+-]'
    for ($i = 0; $i -lt [Math]::Min($sa.Count, $sb.Count); $i++) {
        $na = 0L; $nb = 0L
        if ([long]::TryParse($sa[$i], [ref]$na) -and [long]::TryParse($sb[$i], [ref]$nb)) { $c = $na.CompareTo($nb) }
        else { $c = [string]::CompareOrdinal($sa[$i], $sb[$i]) }
        if ($c -ne 0) { return $c }
    }
    $c = $sa.Count.CompareTo($sb.Count)
    if ($c -ne 0) { return $c }
    [string]::CompareOrdinal($VersionA, $VersionB)
}

Export-ModuleMember -Function Get-OmaNuGetRuntimePackages, Get-OmaNuGetPackageLicense, Test-OmaLicenseAccepted,
    Merge-OmaLicenseSections, ConvertTo-OmaLicenseText, Test-OmaPinnedText
