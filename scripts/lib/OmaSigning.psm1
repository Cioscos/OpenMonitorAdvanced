#Requires -Version 7
# Two-pass signing shim behind Tauri's bundle.windows.signCommand (spec M6a §3.2, plan L2/L9).
#
# collect (tauri build):  copies the patched oma-app.exe and the NSIS uninstaller to unsigned/,
#                         records the setup and leaves the NSIS plugins alone;
# apply   (tauri bundle): checks that app and uninstaller are byte for byte what collect saw and
#                         overwrites them with the signed copies imported into signed/.
# Everything is recorded in <StateRoot>/manifest.json. The recognition rules below hold for
# tauri-cli 2.11.5 with NSIS 3.11 (spike of 2026-09-30): a toolchain update means redoing the spike.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'OmaCommon.psm1')

$ProductName = 'OpenMonitor Advanced'
$SignedNames = @('oma-app.exe', 'uninstall.exe', 'oma-service.exe')
# The five plugins the Tauri NSIS bundler passes to signCommand (spec §3.1); third-party code,
# never modified and never sent to SignPath.
$PluginNames = @('NSISdl.dll', 'StartMenu.dll', 'System.dll', 'nsDialogs.dll', 'additional\nsis_tauri_utils.dll')

# From the spike (plan, "Esito dello spike"), matched against the normalised path. \z, not $:
# in .NET $ also accepts a trailing \n.
$UninstallerPathPattern = '(?i)^(?<dir>.+)\\nst[0-9a-f]{1,4}\.tmp\z'

function Get-OmaPluginPathPattern {
    param([Parameter(Mandatory)] [string]$RepoRoot)
    '(?i)^' + [regex]::Escape($RepoRoot) + '\\target\\release\\nsis\\x64\\Plugins\\x86-unicode\\(?:NSISdl|StartMenu|System|nsDialogs|additional\\nsis_tauri_utils)\.dll\z'
}

# --- helpers -------------------------------------------------------------------------------------

# Retries an action on IOException: antivirus scanners briefly lock freshly written executables
# (the spike hit a sharing violation right after a copy). Missing files are not retried.
function Invoke-OmaIoRetry([scriptblock]$Action) {
    for ($i = 1; ; $i++) {
        try {
            return & $Action
        } catch [IO.FileNotFoundException], [IO.DirectoryNotFoundException] {
            throw
        } catch [IO.IOException] {
            if ($i -ge 10) { throw }
            Start-Sleep -Milliseconds (200 * $i)
        }
    }
}

# One File.Copy, never a copy followed by another write to the same file.
function Copy-OmaFile([string]$Source, [string]$Destination, [switch]$Overwrite) {
    Invoke-OmaIoRetry { [IO.File]::Copy($Source, $Destination, [bool]$Overwrite) }
}

function Get-OmaManifestPath([string]$StateRoot) { Join-Path $StateRoot 'manifest.json' }

function Read-OmaManifest([string]$StateRoot) {
    $path = Get-OmaManifestPath $StateRoot
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "no signing state in ${StateRoot}: run sign-shim.ps1 -Mode init first"
    }
    $m = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json -AsHashtable
    if ($m['schema'] -ne 1) { throw "unsupported manifest schema in ${path}: $($m['schema'])" }
    $m['collect'] = @($m['collect'] | Where-Object { $null -ne $_ })
    $m['apply'] = @($m['apply'] | Where-Object { $null -ne $_ })
    if ($null -eq $m['signed']) { $m['signed'] = [ordered]@{} }
    $m
}

# Written to a temporary file and renamed over the manifest, so a crash never leaves half a file.
function Write-OmaJson([string]$Path, $Value) {
    $tmp = "$Path.tmp"
    $json = ConvertTo-Json -InputObject $Value -Depth 10
    [IO.File]::WriteAllText($tmp, $json + "`n", [Text.UTF8Encoding]::new($false))
    Invoke-OmaIoRetry { [IO.File]::Move($tmp, $Path, $true) }
}

function Write-OmaManifest([string]$StateRoot, $Manifest) {
    Write-OmaJson (Get-OmaManifestPath $StateRoot) $Manifest
}

function Assert-OmaVersionString([string]$Version) {
    if ($Version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' -or
        @($Version.Split('.') | Where-Object { [long]$_ -gt 65535 }).Count -gt 0) {
        throw "invalid version: '$Version' (expected X.Y.Z, each component <= 65535)"
    }
}

# The context ends up on the signCommand line, which makensis reads inside '...': only plain
# characters are allowed.
function Assert-OmaRunContextValues([string]$Commit, [string]$Version, [string]$RunId, [string]$RunAttempt) {
    if ($Commit -cnotmatch '^([0-9a-f]{40}|[0-9a-f]{64})$') { throw "invalid commit: '$Commit' (expected a full lowercase SHA)" }
    Assert-OmaVersionString $Version
    if ($RunId -cnotmatch '^[0-9A-Za-z._-]{1,64}$') { throw "invalid run id: '$RunId'" }
    if ($RunAttempt -cnotmatch '^[1-9][0-9]{0,5}$') { throw "invalid run attempt: '$RunAttempt'" }
}

# makensis runs the uninstaller command from a single-quoted NSIS string (spike): no ' or $,
# and nothing that Tauri's argument rendering or %1 substitution could alter.
function Assert-OmaCommandLineSafe([string]$Value, [string]$What) {
    if ($Value -match '[''$"%`\r\n\t]') { throw "$What contains a character not allowed on the signCommand line (' `$ `" % ``): $Value" }
}

function Test-OmaUnder([string]$Path, [string]$Dir) {
    $Path.StartsWith($Dir.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)
}

# Refuses reparse points (junctions, symlinks) on every existing component from the repository
# root down to the state root, so the reset can never follow a link out of target/.
function Assert-OmaNoReparsePoint([string]$RepoRoot, [string]$StateRoot) {
    $current = $RepoRoot
    $segments = @($current) + @($StateRoot.Substring($RepoRoot.Length).Trim('\').Split('\'))
    for ($i = 0; $i -lt $segments.Count; $i++) {
        if ($i -gt 0) { $current = Join-Path $current $segments[$i] }
        if (-not (Test-Path -LiteralPath $current)) { continue }
        if ([IO.File]::GetAttributes($current).HasFlag([IO.FileAttributes]::ReparsePoint)) {
            throw "reparse point on the signing state path: $current"
        }
    }
}

function Assert-OmaRunContext($Manifest, $ExpectedContext) {
    if ($null -eq $ExpectedContext) { throw 'an expected run context (commit, version, run id, attempt) is required' }
    foreach ($pair in @(@('commit', 'Commit'), @('version', 'Version'), @('runId', 'RunId'), @('runAttempt', 'RunAttempt'))) {
        $found = [string]$Manifest[$pair[0]]
        $want = [string]$ExpectedContext.($pair[1])
        if ($found -cne $want) {
            throw "manifest belongs to another run: $($pair[0]) is '$found', expected '$want'"
        }
    }
}

function New-OmaEntry([string]$Role, [string]$Path, [string]$Name, [string]$Sha, [string]$After) {
    [ordered]@{ role = $Role; path = $Path; name = $Name; sha256 = $Sha; after = $After }
}

function Get-OmaEntries($List, [string]$Role, [string]$Name) {
    @($List | Where-Object { $_['role'] -eq $Role -and (-not $Name -or $_['name'] -eq $Name) })
}

function Assert-OmaCollectOpen($Manifest) {
    if ($Manifest['apply'].Count -gt 0 -or $Manifest['signed'].Count -gt 0) {
        throw 'the collect pass is closed: signed files are already imported'
    }
}

# Maps a normalised path to its role and canonical name, or fails.
function Resolve-OmaSignRole($Manifest, [string]$Received, [string]$Path) {
    $exp = $Manifest['expected']
    if ($Path -ieq $exp['app']) { return @('app', 'oma-app.exe') }
    if ($Path -ieq (Join-Path $exp['setupDir'] $exp['setupName'])) { return @('setup', $exp['setupName']) }
    $repo = $Manifest['repoRoot']
    if ($Path -match (Get-OmaPluginPathPattern -RepoRoot $repo)) {
        $pluginDir = Join-Path $repo 'target\release\nsis\x64\Plugins\x86-unicode'
        return @('plugin', $Path.Substring($pluginDir.Length + 1))
    }
    # One expression: on its own, $Matches could be left over from an earlier match.
    if (($Path -match $UninstallerPathPattern) -and ($Matches['dir'] -ieq [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\'))) {
        return @('uninstaller', 'uninstall.exe')
    }
    throw "unexpected file passed to signCommand: $Received"
}

# --- public functions ----------------------------------------------------------------------------

<#
.SYNOPSIS
  Recreates an empty signing state and writes manifest.json and the two Tauri configs.
.DESCRIPTION
  Before deleting anything, RepoRoot and StateRoot are resolved as absolute paths and StateRoot
  must lie strictly under <RepoRoot>\target, outside the build and payload directories, with no
  reparse point from the repository root down. Only that verified directory is emptied.
#>
function Initialize-OmaSigningState {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$StateRoot,
        [Parameter(Mandatory)] [string]$RepoRoot,
        [Parameter(Mandatory)] [string]$Commit,
        [Parameter(Mandatory)] [string]$Version,
        [Parameter(Mandatory)] [string]$RunId,
        [Parameter(Mandatory)] [string]$RunAttempt
    )
    Assert-OmaRunContextValues $Commit $Version $RunId $RunAttempt
    $repo = Resolve-OmaPath $RepoRoot
    if ($repo -eq [IO.Path]::GetPathRoot($repo)) { throw "the repository root cannot be a drive root: $repo" }
    if (-not (Test-Path -LiteralPath $repo -PathType Container)) { throw "repository root not found: $repo" }
    $state = Resolve-OmaPath $StateRoot
    $target = Join-Path $repo 'target'
    if (-not (Test-OmaUnder $state $target)) {
        throw "the signing state must be a dedicated directory strictly under ${target}: $state"
    }
    foreach ($reserved in 'installer-payload', 'release', 'debug') {
        $dir = Join-Path $target $reserved
        if ($state -ieq $dir -or (Test-OmaUnder $state $dir)) {
            throw "the signing state cannot be inside ${dir}: $state"
        }
    }
    $script = Resolve-OmaPath (Join-Path $PSScriptRoot '..\sign-shim.ps1')
    Assert-OmaCommandLineSafe $state 'the state root'
    Assert-OmaCommandLineSafe $script 'the shim path'
    Assert-OmaNoReparsePoint $repo $state

    if (Test-Path -LiteralPath $state) { Remove-Item -LiteralPath $state -Recurse -Force }
    New-Item -ItemType Directory -Path (Join-Path $state 'unsigned'), (Join-Path $state 'signed') -Force | Out-Null

    $manifest = [ordered]@{
        schema     = 1
        commit     = $Commit
        version    = $Version
        runId      = $RunId
        runAttempt = $RunAttempt
        repoRoot   = $repo
        expected   = [ordered]@{
            app       = Join-Path $repo 'target\release\oma-app.exe'
            setupDir  = Join-Path $repo 'target\release\bundle\nsis'
            setupName = "${ProductName}_${Version}_x64-setup.exe"
            service   = Join-Path $repo 'target\installer-payload\service\oma-service.exe'
        }
        collect    = @()
        apply      = @()
        signed     = [ordered]@{}
    }
    Write-OmaManifest $state $manifest

    foreach ($mode in 'collect', 'apply') {
        $arguments = @(
            '-NoProfile', '-NonInteractive', '-File', $script,
            '-Mode', $mode, '-StateRoot', $state,
            '-Commit', $Commit, '-Version', $Version, '-RunId', $RunId, '-RunAttempt', $RunAttempt,
            '-Path', '%1')
        $config = [ordered]@{ bundle = [ordered]@{ windows = [ordered]@{ signCommand = [ordered]@{ cmd = 'pwsh'; args = $arguments } } } }
        Write-OmaJson (Join-Path $state "tauri.sign.$mode.json") $config
    }
}

<#
.SYNOPSIS
  Handles one signCommand call from Tauri or makensis in the collect or apply pass.
.DESCRIPTION
  Returns the manifest entry it recorded. With -ExpectedContext, the manifest must belong to
  that run first.
#>
function Invoke-OmaSignShim {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [ValidateSet('collect', 'apply')] [string]$Mode,
        [Parameter(Mandatory)] [string]$StateRoot,
        [Parameter(Mandatory)] [string]$Path,
        [pscustomobject]$ExpectedContext
    )
    $state = Resolve-OmaPath $StateRoot
    $m = Read-OmaManifest $state
    if ($null -ne $ExpectedContext) { Assert-OmaRunContext $m $ExpectedContext }
    if ($Mode -eq 'collect') { Assert-OmaCollectOpen $m }

    $p = Resolve-OmaPath $Path
    $role, $name = Resolve-OmaSignRole $m $Path $p
    if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { throw "file passed to signCommand not found: $p" }
    if ($role -ne 'plugin' -and @(Get-OmaEntries $m[$Mode] $role).Count -gt 0) { throw "duplicate $role call" }

    $sha = Get-OmaSha256 $p
    $after = $sha
    if ($Mode -eq 'collect') {
        if ($role -in 'app', 'uninstaller') {
            $copy = Join-Path $state "unsigned\$name"
            Copy-OmaFile $p $copy
            if ((Get-OmaSha256 $copy) -ne $sha) { throw "the copy of $name in unsigned/ does not match the received file" }
        }
        $after = Get-OmaSha256 $p
        if ($after -ne $sha) { throw "$name changed while the shim was reading it: $sha, then $after" }
    } else {
        if ($role -in 'app', 'uninstaller') {
            $collected = @(Get-OmaEntries $m['collect'] $role)
            if ($collected.Count -ne 1) { throw "no collected $role in the manifest: run the collect pass first" }
            $want = $collected[0]['sha256']
            if ($sha -ne $want) { throw "hash mismatch for ${name}: collected $want, received $sha" }
            $signed = Join-Path $state "signed\$name"
            if (-not $m['signed'].Contains($name) -or -not (Test-Path -LiteralPath $signed -PathType Leaf)) {
                throw "signed copy missing for $name"
            }
            $imported = $m['signed'][$name]
            $current = Get-OmaSha256 $signed
            if ($current -ne $imported) { throw "signed copy of $name does not match its imported hash: imported $imported, found $current" }
            Copy-OmaFile $signed $p -Overwrite
            $after = Get-OmaSha256 $p
            if ($after -ne $imported) { throw "after the copy $name has hash $after, expected the signed $imported" }
        } else {
            if ($role -eq 'plugin') {
                $collected = @(Get-OmaEntries $m['collect'] 'plugin' $name)
                if ($collected.Count -ne 1) { throw "plugin $name was not seen in the collect pass" }
                if ($collected[0]['sha256'] -ne $sha) {
                    throw "plugin $name differs from the collect pass: collected $($collected[0]['sha256']), received $sha"
                }
            }
            $after = Get-OmaSha256 $p
            if ($after -ne $sha) { throw "$name changed while the shim was reading it: $sha, then $after" }
        }
    }
    $entry = New-OmaEntry $role $p $name $sha $after
    $m[$Mode] = @($m[$Mode]) + @($entry)
    Write-OmaManifest $state $m
    [pscustomobject]$entry
}

<#
.SYNOPSIS
  Records the payload oma-service.exe (role 'service') in the collect pass and copies it to unsigned/.
#>
function Register-OmaService {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$StateRoot,
        [Parameter(Mandatory)] [string]$Path
    )
    $state = Resolve-OmaPath $StateRoot
    $m = Read-OmaManifest $state
    Assert-OmaCollectOpen $m
    $p = Resolve-OmaPath $Path
    if ($p -ine $m['expected']['service']) { throw "unexpected service path: $Path (expected $($m['expected']['service']))" }
    if (@(Get-OmaEntries $m['collect'] 'service').Count -gt 0) { throw 'duplicate service call' }
    if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { throw "service not found: $p" }
    $sha = Get-OmaSha256 $p
    $copy = Join-Path $state 'unsigned\oma-service.exe'
    Copy-OmaFile $p $copy
    if ((Get-OmaSha256 $copy) -ne $sha) { throw 'the copy of oma-service.exe in unsigned/ does not match the payload' }
    $entry = New-OmaEntry 'service' $p 'oma-service.exe' $sha (Get-OmaSha256 $p)
    $m['collect'] = @($m['collect']) + @($entry)
    Write-OmaManifest $state $m
    [pscustomobject]$entry
}

<#
.SYNOPSIS
  Copies the three signed files from -From into signed/ and records their hashes.
.DESCRIPTION
  -From must hold exactly oma-app.exe, uninstall.exe and oma-service.exe, nothing more and
  nothing less. Signature verification is separate (verify-signatures.ps1) and runs before.
#>
function Import-OmaSignedFiles {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$StateRoot,
        [Parameter(Mandatory)] [string]$From
    )
    $state = Resolve-OmaPath $StateRoot
    $m = Read-OmaManifest $state
    $src = Resolve-OmaPath $From
    if (-not (Test-Path -LiteralPath $src -PathType Container)) { throw "signed files directory not found: $src" }
    $items = @(Get-ChildItem -LiteralPath $src -Force)
    $files = @($items | Where-Object { -not $_.PSIsContainer } | ForEach-Object Name)
    $missing = @($SignedNames | Where-Object { $_ -notin $files })
    $unexpected = @($items | Where-Object { $_.PSIsContainer -or $_.Name -notin $SignedNames } | ForEach-Object Name)
    if ($missing.Count -gt 0 -or $unexpected.Count -gt 0) {
        throw "import-signed expects exactly $($SignedNames -join ', ') in ${src}; missing: $($missing -join ', '); unexpected: $($unexpected -join ', ')"
    }
    $signedDir = Join-Path $state 'signed'
    if ($m['signed'].Count -gt 0 -or @(Get-ChildItem -LiteralPath $signedDir -Force).Count -gt 0) {
        throw 'signed files are already imported'
    }
    foreach ($role in 'app', 'uninstaller', 'service') {
        if (@(Get-OmaEntries $m['collect'] $role).Count -ne 1) { throw "cannot import signed files: the collect pass has no $role" }
    }
    $signed = [ordered]@{}
    foreach ($n in $SignedNames) {
        $dest = Join-Path $signedDir $n
        Copy-OmaFile (Join-Path $src $n) $dest
        $signed[$n] = Get-OmaSha256 $dest
    }
    $m['signed'] = $signed
    Write-OmaManifest $state $m
    [pscustomobject]$signed
}

<#
.SYNOPSIS
  Gate after a bundle: the pass saw exactly the expected calls and the state is consistent.
.DESCRIPTION
  The run context comes from the caller (GITHUB_SHA, validated version, run id/attempt), never
  from the manifest being checked. collect needs one app, uninstaller, setup and service; apply
  one app, uninstaller and setup. Both need one call for each of the five NSIS plugins, intact.
#>
function Assert-OmaSigningPass {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$StateRoot,
        [Parameter(Mandatory)] [ValidateSet('collect', 'apply')] [string]$Pass,
        [Parameter(Mandatory)] [pscustomobject]$ExpectedContext
    )
    $state = Resolve-OmaPath $StateRoot
    $m = Read-OmaManifest $state
    Assert-OmaRunContext $m $ExpectedContext
    $repo = $m['repoRoot']
    $exp = $m['expected']
    if ($exp['app'] -ine (Join-Path $repo 'target\release\oma-app.exe') -or
        $exp['setupDir'] -ine (Join-Path $repo 'target\release\bundle\nsis') -or
        $exp['setupName'] -cne "${ProductName}_$($ExpectedContext.Version)_x64-setup.exe") {
        throw 'the expected paths in the manifest do not match the repository and version'
    }

    $entries = $m[$Pass]
    $required = if ($Pass -eq 'collect') { 'app', 'uninstaller', 'setup', 'service' } else { 'app', 'uninstaller', 'setup' }
    foreach ($role in $required) {
        $n = @(Get-OmaEntries $entries $role).Count
        if ($n -ne 1) { throw "check ${Pass}: expected exactly one $role call, found $n" }
    }
    if ($Pass -eq 'apply' -and @(Get-OmaEntries $entries 'service').Count -gt 0) { throw 'check apply: the service is not part of the apply pass' }
    foreach ($plugin in $PluginNames) {
        $n = @(Get-OmaEntries $entries 'plugin' $plugin).Count
        if ($n -ne 1) { throw "check ${Pass}: expected exactly one call for plugin $plugin, found $n" }
    }
    # Only apply replaces files (app and uninstaller, checked below); everything else stays as received.
    foreach ($e in $entries) {
        $replaced = $Pass -eq 'apply' -and $e['role'] -in 'app', 'uninstaller'
        if (-not $replaced -and $e['after'] -ne $e['sha256']) { throw "check ${Pass}: $($e['role']) $($e['name']) changed during the pass" }
    }

    # The collect rows and the unsigned copies must be intact after either pass.
    $unsignedDir = Join-Path $state 'unsigned'
    $unsigned = @(Get-ChildItem -LiteralPath $unsignedDir -Force | ForEach-Object Name | Sort-Object)
    if (($unsigned -join '|') -ne (($SignedNames | Sort-Object) -join '|')) {
        throw "check ${Pass}: unsigned/ must hold exactly $($SignedNames -join ', '); found $($unsigned -join ', ')"
    }
    foreach ($role in 'app', 'uninstaller', 'service') {
        $c = @(Get-OmaEntries $m['collect'] $role)
        if ($c.Count -ne 1) { throw "check ${Pass}: the collect pass has no $role" }
        $h = Get-OmaSha256 (Join-Path $unsignedDir $c[0]['name'])
        if ($h -ne $c[0]['sha256']) { throw "check ${Pass}: unsigned/$($c[0]['name']) no longer matches the collected hash" }
    }

    # The setup on disk must be the one this pass handed to the shim (a failed bundle leaves the
    # previous setup in place). After apply, the collect setup has legitimately been replaced.
    $setupPath = Join-Path $exp['setupDir'] $exp['setupName']
    if ($Pass -eq 'apply' -or @(Get-OmaEntries $m['apply'] 'setup').Count -eq 0) {
        $s = @(Get-OmaEntries $entries 'setup')[0]
        if (-not (Test-Path -LiteralPath $setupPath -PathType Leaf) -or (Get-OmaSha256 $setupPath) -ne $s['sha256']) {
            throw "check ${Pass}: the setup at $setupPath is not the one the shim saw"
        }
    }

    if ($Pass -eq 'apply') {
        $names = @($m['signed'].Keys | Sort-Object)
        if (($names -join '|') -ne (($SignedNames | Sort-Object) -join '|')) { throw 'check apply: the signed files were not imported' }
        foreach ($role in 'app', 'uninstaller') {
            $a = @(Get-OmaEntries $entries $role)[0]
            $c = @(Get-OmaEntries $m['collect'] $role)[0]
            if ($a['sha256'] -ne $c['sha256']) { throw "check apply: $($a['name']) received in apply differs from the collected one" }
            if ($a['after'] -ne $m['signed'][$a['name']]) { throw "check apply: $($a['name']) was not replaced with its signed copy" }
        }
        foreach ($n in $SignedNames) {
            if ((Get-OmaSha256 (Join-Path $state "signed\$n")) -ne $m['signed'][$n]) { throw "check apply: signed/$n no longer matches its imported hash" }
        }
        foreach ($plugin in $PluginNames) {
            $a = @(Get-OmaEntries $entries 'plugin' $plugin)[0]
            $c = @(Get-OmaEntries $m['collect'] 'plugin' $plugin)
            if ($c.Count -ne 1 -or $c[0]['sha256'] -ne $a['sha256']) { throw "check apply: plugin $plugin differs from the collect pass" }
        }
    }
}

Export-ModuleMember -Function Initialize-OmaSigningState, Invoke-OmaSignShim, Register-OmaService,
    Import-OmaSignedFiles, Assert-OmaSigningPass, Get-OmaPluginPathPattern -Variable UninstallerPathPattern
