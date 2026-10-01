#Requires -Version 7
# Version consistency check and bump (spec M6a §6). The product version lives in five files;
# the check lists every mismatch, the bump rewrites only the version substrings and restores
# every touched file byte for byte if anything fails. Git and Cargo are injectable so the tests
# never touch the network or the real repository.
#
# Adapter contract for -Git and -Cargo: a scriptblock `param([string[]]$Arguments, [string]$WorkingDirectory)`
# returning @{ Stdout; Stderr; ExitCode } without throwing on a nonzero exit code. The defaults
# wrap Invoke-OmaNative -AllowFailure; arguments are always an array, never a shell string.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'OmaCommon.psm1') -ErrorAction Stop

$script:Utf8 = [Text.UTF8Encoding]::new($false)
$script:TagPattern = '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
$script:CargoCommand = { param([string[]]$Arguments, [string]$WorkingDirectory)
    Invoke-OmaNative -FilePath 'cargo' -ArgumentList $Arguments -WorkingDirectory $WorkingDirectory -AllowFailure }
$script:GitCommand = { param([string[]]$Arguments, [string]$WorkingDirectory)
    Invoke-OmaNative -FilePath 'git' -ArgumentList $Arguments -WorkingDirectory $WorkingDirectory -AllowFailure }

<#
.SYNOPSIS
  Parses X.Y.Z (canonical, no leading zeros, each component 0..65535) into [version].
#>
function ConvertTo-OmaVersion {
    [CmdletBinding()]
    param([Parameter(Mandatory, Position = 0)] [AllowEmptyString()] [string]$Text)
    if ($Text -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
        throw "not a canonical X.Y.Z version: '$Text'"
    }
    $parts = foreach ($i in 1..3) {
        $n = 0
        if (-not [int]::TryParse($Matches[$i], [ref]$n) -or $n -gt 65535) {
            throw "version component above 65535 (PE limit): '$Text'"
        }
        $n
    }
    [version]::new($parts[0], $parts[1], $parts[2])
}

# Relative path (forward slashes) and the regex whose named group 'v' is the version text.
# The JSON patterns take the two-space-indented top-level key, so nested versions (the NSIS
# one in tauri.conf.json) are not matched.
$script:SourceDefinitions = @(
    @{ File = 'Cargo.toml'; Pattern = '(?m)^\[workspace\.package\][ \t]*\r?\n(?:(?!\[)(?!version[ \t]*=)[^\n]*\n)*version[ \t]*=[ \t]*"(?<v>[^"\r\n]*)"' }
    @{ File = 'app/package.json'; Pattern = '(?m)^(?: {2}|\t)"version"[ \t]*:[ \t]*"(?<v>[^"\r\n]*)"' }
    @{ File = 'app/src-tauri/tauri.conf.json'; Pattern = '(?m)^(?: {2}|\t)"version"[ \t]*:[ \t]*"(?<v>[^"\r\n]*)"' }
    @{ File = 'README.md'; Pattern = '(?m)^>[ \t]*\*\*Status:\*\*[^\r\n]*?\(version (?<v>[^)\r\n]*)\)' }
    @{ File = 'README.it.md'; Pattern = '(?m)^>[ \t]*\*\*Stato:\*\*[^\r\n]*?\(versione (?<v>[^)\r\n]*)\)' }
)

<#
.SYNOPSIS
  Reads the version of each of the five files: @{ File; Version ($null when missing); Pattern }.
#>
function Get-OmaVersionSources {
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string]$RepoRoot)
    foreach ($d in $script:SourceDefinitions) {
        $path = Join-Path $RepoRoot $d.File
        $version = $null
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            $m = [regex]::Match($script:Utf8.GetString([IO.File]::ReadAllBytes($path)), $d.Pattern)
            if ($m.Success) { $version = $m.Groups['v'].Value }
        }
        [pscustomobject]@{ File = $d.File; Version = $version; Pattern = $d.Pattern }
    }
}

function Get-FirstLine([string]$Text) {
    $t = "$Text".Trim()
    if (-not $t) { return '(no output)' }
    ($t -split '\r?\n')[0]
}

<#
.SYNOPSIS
  Returns the list of inconsistencies (empty = ok): five-file mismatch, stale Cargo.lock and,
  with -Tag, tag shape, tag/version, tag commit vs HEAD vs the run SHA, and origin/main ancestry.
#>
function Test-OmaVersionConsistency {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$RepoRoot,
        [string]$Tag,
        [scriptblock]$Cargo,
        [scriptblock]$Git,
        [string]$ExpectedSha
    )
    if (-not $Cargo) { $Cargo = $script:CargoCommand }
    if (-not $Git) { $Git = $script:GitCommand }
    $problems = [Collections.Generic.List[string]]::new()

    $sources = @(Get-OmaVersionSources $RepoRoot)
    $present = @($sources | Where-Object { $null -ne $_.Version })
    foreach ($s in $sources | Where-Object { $null -eq $_.Version }) {
        $problems.Add("$($s.File): missing version field")
    }
    foreach ($s in $present) {
        try { $null = ConvertTo-OmaVersion $s.Version }
        catch { $problems.Add("$($s.File): $($_.Exception.Message)") }
    }

    # Reference = the most common version (Cargo.toml wins a tie), so one wrong file yields one line.
    $reference = $null
    if ($present) {
        $reference = ($present | Group-Object Version |
                Sort-Object @{ Expression = 'Count'; Descending = $true },
                @{ Expression = { if ($_.Group.File -contains 'Cargo.toml') { 0 } else { 1 } } } |
                Select-Object -First 1).Name
        foreach ($s in $present | Where-Object { $_.Version -ne $reference }) {
            $problems.Add("$($s.File): version $($s.Version) differs from $reference")
        }
    }

    try {
        # No --no-deps: with it cargo skips the resolution and accepts a stale Cargo.lock.
        $r = & $Cargo @('metadata', '--locked', '--format-version', '1') $RepoRoot
        if ($r.ExitCode -ne 0) {
            $problems.Add("Cargo.lock is not aligned with the manifests (cargo metadata --locked failed: $(Get-FirstLine $r.Stderr))")
        }
    } catch {
        $problems.Add("cargo metadata could not run: $($_.Exception.Message)")
    }

    if ($Tag -or $ExpectedSha) {
        $shaOfHead = $null
        $shaOfTag = $null
        $gitCall = {
            param([string[]]$GitArgs)
            try { & $Git $GitArgs $RepoRoot }
            catch { [pscustomobject]@{ Stdout = ''; Stderr = $_.Exception.Message; ExitCode = 127 } }
        }

        $tagValid = $false
        if ($Tag) {
            if ($Tag -notmatch $script:TagPattern) {
                $problems.Add("tag '$Tag' does not match vX.Y.Z (numeric, no leading zeros)")
            } else {
                $tagValid = $true
                if ($reference -and $Tag -ne "v$reference") {
                    $problems.Add("tag $Tag does not match the version $reference")
                }
            }
        }

        if ($tagValid) {
            $f = & $gitCall @('fetch', 'origin', '+refs/heads/main:refs/remotes/origin/main')
            if ($f.ExitCode -ne 0) {
                $problems.Add("git fetch of origin/main failed: $(Get-FirstLine $f.Stderr)")
            }
        }

        $h = & $gitCall @('rev-parse', 'HEAD')
        if ($h.ExitCode -eq 0) { $shaOfHead = $h.Stdout.Trim() }
        elseif ($tagValid -or $ExpectedSha) { $problems.Add("git rev-parse HEAD failed: $(Get-FirstLine $h.Stderr)") }

        if ($tagValid) {
            $t = & $gitCall @('rev-parse', '--verify', "refs/tags/$Tag^{commit}")
            if ($t.ExitCode -eq 0) { $shaOfTag = $t.Stdout.Trim() }
            else { $problems.Add("tag $Tag cannot be resolved to a commit: $(Get-FirstLine $t.Stderr)") }
        }

        if ($shaOfTag -and $shaOfHead -and $shaOfTag -ne $shaOfHead) {
            $problems.Add("tag $Tag points to $shaOfTag but HEAD is $shaOfHead")
        }
        if ($ExpectedSha) {
            if ($shaOfHead -and $shaOfHead -ne $ExpectedSha.Trim().ToLowerInvariant()) {
                $problems.Add("HEAD $shaOfHead differs from the run SHA $ExpectedSha")
            }
            if ($shaOfTag -and $shaOfTag -ne $ExpectedSha.Trim().ToLowerInvariant()) {
                $problems.Add("tag $Tag commit $shaOfTag differs from the run SHA $ExpectedSha")
            }
        }

        if ($shaOfTag) {
            $a = & $gitCall @('merge-base', '--is-ancestor', $shaOfTag, 'origin/main')
            if ($a.ExitCode -eq 1) {
                $problems.Add("tag $Tag commit $shaOfTag is not reachable from origin/main")
            } elseif ($a.ExitCode -ne 0) {
                $problems.Add("git merge-base --is-ancestor failed: $(Get-FirstLine $a.Stderr)")
            }
        }
    }

    $problems.ToArray()
}

# Cargo.lock reduced to what the structural check compares.
function ConvertFrom-OmaLock([string]$Text) {
    $packages = [Collections.Generic.List[object]]::new()
    $cur = $null
    $inDeps = $false
    foreach ($line in $Text -split '\r?\n') {
        if ($line -eq '[[package]]') {
            $cur = [pscustomobject]@{ Name = $null; Version = $null; Source = $null; Checksum = $null; Deps = [Collections.Generic.List[string]]::new() }
            $packages.Add($cur)
            $inDeps = $false
        } elseif ($line -match '^\[') {
            $cur = $null
            $inDeps = $false
        } elseif ($cur) {
            if ($inDeps) {
                if ($line -match '^\s*\]') { $inDeps = $false }
                elseif ($line -match '^\s*"(?<d>[^"]*)"') { $cur.Deps.Add($Matches['d']) }
            } elseif ($line -match '^dependencies\s*=\s*\[\s*\]') {
                continue
            } elseif ($line -match '^dependencies\s*=\s*\[') {
                $inDeps = $true
            } elseif ($line -match '^(?<k>name|version|source|checksum)\s*=\s*"(?<v>[^"]*)"') {
                switch ($Matches['k']) {
                    'name' { $cur.Name = $Matches['v'] }
                    'version' { $cur.Version = $Matches['v'] }
                    'source' { $cur.Source = $Matches['v'] }
                    'checksum' { $cur.Checksum = $Matches['v'] }
                }
            }
        }
    }
    , $packages.ToArray()
}

# Throws unless the only differences between the two lock files are the workspace packages'
# own versions (old -> new) and the internal references to them.
function Assert-OmaLockBump {
    param([string]$OldText, [string]$NewText, [string[]]$LocalNames, [string]$OldVersion, [string]$NewVersion)
    $old = ConvertFrom-OmaLock $OldText
    $new = ConvertFrom-OmaLock $NewText
    $isLocal = { param($n) $LocalNames -contains $n }

    $normDeps = {
        param($pkg)
        $pkg.Deps | ForEach-Object {
            $name = ($_ -split ' ')[0]
            if (& $isLocal $name) { $name } else { $_ }
        } | Sort-Object
    }
    $canon = {
        param($pkg)
        "$($pkg.Name)|$($pkg.Version)|$($pkg.Source)|$($pkg.Checksum)|$((& $normDeps $pkg) -join ',')"
    }

    $oldExt = @($old | Where-Object { -not (& $isLocal $_.Name) } | ForEach-Object { & $canon $_ } | Sort-Object)
    $newExt = @($new | Where-Object { -not (& $isLocal $_.Name) } | ForEach-Object { & $canon $_ } | Sort-Object)
    if (($oldExt -join "`n") -ne ($newExt -join "`n")) {
        $diff = Compare-Object $oldExt $newExt | Select-Object -First 3 | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }
        throw "Cargo.lock changed outside the workspace packages: $($diff -join '; ')"
    }

    $oldLocal = @($old | Where-Object { & $isLocal $_.Name })
    $newLocal = @($new | Where-Object { & $isLocal $_.Name })
    if ($oldLocal.Count -ne $newLocal.Count) { throw 'Cargo.lock gained or lost a workspace package' }
    foreach ($o in $oldLocal) {
        $n = @($newLocal | Where-Object { $_.Name -eq $o.Name })
        if ($n.Count -ne 1) { throw "Cargo.lock workspace package $($o.Name) appears $($n.Count) times" }
        $n = $n[0]
        if ($n.Source -ne $o.Source -or $n.Checksum -ne $o.Checksum) {
            throw "Cargo.lock source or checksum of $($o.Name) changed"
        }
        if ($n.Version -ne $o.Version -and -not ($o.Version -eq $OldVersion -and $n.Version -eq $NewVersion)) {
            throw "Cargo.lock version of $($o.Name) changed from $($o.Version) to $($n.Version), expected $NewVersion"
        }
        if (((& $normDeps $o) -join ',') -ne ((& $normDeps $n) -join ',')) {
            throw "Cargo.lock dependencies of $($o.Name) changed"
        }
    }
}

<#
.SYNOPSIS
  Bumps the five files and Cargo.lock to a higher X.Y.Z, restoring all six byte for byte on any failure.
#>
function Set-OmaVersion {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$RepoRoot,
        [Parameter(Mandatory)] [string]$Version,
        [scriptblock]$Cargo,
        [scriptblock]$Git
    )
    if (-not $Cargo) { $Cargo = $script:CargoCommand }
    if (-not $Git) { $Git = $script:GitCommand }

    $newVersion = ConvertTo-OmaVersion $Version
    $problems = @(Test-OmaVersionConsistency -RepoRoot $RepoRoot -Cargo $Cargo -Git $Git)
    if ($problems.Count -gt 0) {
        throw "the tree is not consistent, refusing to bump:`n  $($problems -join "`n  ")"
    }
    $sources = @(Get-OmaVersionSources $RepoRoot)
    $current = $sources[0].Version
    if ($newVersion -le (ConvertTo-OmaVersion $current)) {
        throw "new version $Version must be greater than the current $current"
    }

    $paths = @($sources.File) + 'Cargo.lock' | ForEach-Object { [pscustomobject]@{ Rel = $_; Path = Join-Path $RepoRoot $_ } }
    $lockPath = Join-Path $RepoRoot 'Cargo.lock'
    if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) { throw 'Cargo.lock not found' }
    $backup = @{}
    foreach ($p in $paths) { $backup[$p.Path] = [IO.File]::ReadAllBytes($p.Path) }

    try {
        foreach ($s in $sources) {
            $path = Join-Path $RepoRoot $s.File
            $text = $script:Utf8.GetString($backup[$path])
            $g = [regex]::Match($text, $s.Pattern).Groups['v']
            $text = $text.Substring(0, $g.Index) + $Version + $text.Substring($g.Index + $g.Length)
            [IO.File]::WriteAllBytes($path, $script:Utf8.GetBytes($text))
        }

        $u = & $Cargo @('update', '--workspace', '--offline') $RepoRoot
        if ($u.ExitCode -ne 0) { throw "cargo update failed: $(Get-FirstLine $u.Stderr)" }

        $m = & $Cargo @('metadata', '--format-version', '1', '--no-deps', '--offline') $RepoRoot
        if ($m.ExitCode -ne 0) { throw "cargo metadata failed: $(Get-FirstLine $m.Stderr)" }
        $localNames = @((ConvertFrom-Json $m.Stdout).packages.name)
        Assert-OmaLockBump -OldText $script:Utf8.GetString($backup[$lockPath]) `
            -NewText $script:Utf8.GetString([IO.File]::ReadAllBytes($lockPath)) `
            -LocalNames $localNames -OldVersion $current -NewVersion $Version

        $after = @(Test-OmaVersionConsistency -RepoRoot $RepoRoot -Cargo $Cargo -Git $Git)
        if ($after.Count -gt 0) { throw "the tree is not consistent after the bump:`n  $($after -join "`n  ")" }
    } catch {
        foreach ($p in $paths) { [IO.File]::WriteAllBytes($p.Path, $backup[$p.Path]) }
        throw
    }
}

Export-ModuleMember -Function ConvertTo-OmaVersion, Get-OmaVersionSources, Test-OmaVersionConsistency, Set-OmaVersion
