#Requires -Version 7
# Two-pass signing shim behind Tauri's bundle.windows.signCommand (spec M6a §3.2, plan L2/L9).
#
# collect (tauri build):  copies the patched oma-app.exe and the NSIS uninstaller to unsigned/,
#                         records the setup and leaves the NSIS plugins alone; register-payload
#                         adds oma-service.exe and oma-overlay.exe from the installer payload;
# apply   (tauri bundle): checks that app and uninstaller are byte for byte what collect saw and
#                         overwrites them with the signed copies imported into signed/.
# Everything is recorded in <StateRoot>/manifest.json. The recognition rules below hold for
# tauri-cli 2.11.5 with NSIS 3.11 (spike of 2026-09-30): a toolchain update means redoing the spike.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'OmaCommon.psm1')
Import-Module (Join-Path $PSScriptRoot 'OmaPawnIoPins.psm1')
Import-Module (Join-Path $PSScriptRoot 'OmaPresentMonPins.psm1')

$ProductName = 'OpenMonitor Advanced'
$SignedNames = @('oma-app.exe', 'uninstall.exe', 'oma-service.exe', 'oma-overlay.exe')
# The executables of target\installer-payload that NSIS embeds without passing them to
# signCommand (register-payload): manifest key and role, then file name.
$PayloadExes = [ordered]@{ service = 'oma-service.exe'; overlay = 'oma-overlay.exe' }
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

# In GitHub Actions every pass is also compared with the run that is actually executing, read from
# the runner's own variables (inherited by Tauri and makensis), not only with the context that
# init wrote into the configs.
function Assert-OmaGitHubContext($Manifest) {
    if ($env:GITHUB_ACTIONS -ne 'true') { return }
    foreach ($pair in @(@('commit', 'GITHUB_SHA'), @('runId', 'GITHUB_RUN_ID'), @('runAttempt', 'GITHUB_RUN_ATTEMPT'))) {
        $actual = [Environment]::GetEnvironmentVariable($pair[1])
        if (-not $actual) { throw "running in GitHub Actions but $($pair[1]) is not set" }
        $found = [string]$Manifest[$pair[0]]
        if ($found -cne $actual) {
            throw "manifest belongs to another run: $($pair[0]) is '$found', $($pair[1]) is '$actual'"
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
            overlay   = Join-Path $repo 'target\installer-payload\overlay\oma-overlay.exe'
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
  Returns the manifest entry it recorded. The manifest must first belong to -ExpectedContext
  (required; the generated configs always pass it) and, in GitHub Actions, to the run described
  by GITHUB_SHA, GITHUB_RUN_ID and GITHUB_RUN_ATTEMPT.
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
    Assert-OmaRunContext $m $ExpectedContext
    Assert-OmaGitHubContext $m
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
  Records one payload executable in the collect pass and copies it to unsigned/: oma-service.exe
  (role 'service') or oma-overlay.exe (role 'overlay'), each only at its expected path under
  target\installer-payload. Same run context checks as Invoke-OmaSignShim.
#>
function Register-OmaPayload {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$StateRoot,
        [Parameter(Mandatory)] [string]$Path,
        [pscustomobject]$ExpectedContext
    )
    $state = Resolve-OmaPath $StateRoot
    $m = Read-OmaManifest $state
    Assert-OmaRunContext $m $ExpectedContext
    Assert-OmaGitHubContext $m
    Assert-OmaCollectOpen $m
    $p = Resolve-OmaPath $Path
    $role = @($PayloadExes.Keys | Where-Object { $p -ieq $m['expected'][$_] })
    if ($role.Count -ne 1) {
        $wanted = @($PayloadExes.Keys | ForEach-Object { $m['expected'][$_] }) -join ' or '
        throw "unexpected payload path: $Path (expected $wanted)"
    }
    $role = $role[0]
    $name = $PayloadExes[$role]
    if (@(Get-OmaEntries $m['collect'] $role).Count -gt 0) { throw "duplicate $role call" }
    if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { throw "$role not found: $p" }
    $sha = Get-OmaSha256 $p
    $copy = Join-Path $state "unsigned\$name"
    Copy-OmaFile $p $copy
    if ((Get-OmaSha256 $copy) -ne $sha) { throw "the copy of $name in unsigned/ does not match the payload" }
    $entry = New-OmaEntry $role $p $name $sha (Get-OmaSha256 $p)
    $m['collect'] = @($m['collect']) + @($entry)
    Write-OmaManifest $state $m
    [pscustomobject]$entry
}

<#
.SYNOPSIS
  Copies the four signed files from -From into signed/ and records their hashes.
.DESCRIPTION
  -From must hold exactly oma-app.exe, uninstall.exe, oma-service.exe and oma-overlay.exe, nothing more and
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
    foreach ($role in 'app', 'uninstaller', 'service', 'overlay') {
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
  The run context (GITHUB_SHA, validated version, run id/attempt) and the repository root come
  from the caller, never from the manifest being checked; the manifest's expected paths are
  rebuilt from them. collect needs one app, uninstaller, setup, service and overlay; apply one
  app, uninstaller and setup. Both need one call for each of the five NSIS plugins, intact.
  -RepoRoot and -ExpectedContext are required (checked here, so a missing one fails instead of
  prompting).
#>
function Assert-OmaSigningPass {
    [CmdletBinding()]
    param(
        [string]$RepoRoot,
        [Parameter(Mandatory)] [string]$StateRoot,
        [Parameter(Mandatory)] [ValidateSet('collect', 'apply')] [string]$Pass,
        [pscustomobject]$ExpectedContext
    )
    if (-not $RepoRoot) { throw 'the repository root is required to check a pass' }
    $repo = Resolve-OmaPath $RepoRoot
    $state = Resolve-OmaPath $StateRoot
    $m = Read-OmaManifest $state
    Assert-OmaRunContext $m $ExpectedContext
    if ([string]$m['repoRoot'] -ine $repo) {
        throw "manifest belongs to another repository: repoRoot is '$($m['repoRoot'])', expected '$repo'"
    }
    $exp = $m['expected']
    $want = [ordered]@{
        app       = Join-Path $repo 'target\release\oma-app.exe'
        setupDir  = Join-Path $repo 'target\release\bundle\nsis'
        setupName = "${ProductName}_$($ExpectedContext.Version)_x64-setup.exe"
        service   = Join-Path $repo 'target\installer-payload\service\oma-service.exe'
        overlay   = Join-Path $repo 'target\installer-payload\overlay\oma-overlay.exe'
    }
    foreach ($k in $want.Keys) {
        $same = if ($k -eq 'setupName') { [string]$exp[$k] -ceq $want[$k] } else { [string]$exp[$k] -ieq $want[$k] }
        if (-not $same) { throw "the manifest's expected.$k is '$($exp[$k])', expected '$($want[$k])'" }
    }

    $entries = $m[$Pass]
    $required = if ($Pass -eq 'collect') { 'app', 'uninstaller', 'setup', 'service', 'overlay' } else { 'app', 'uninstaller', 'setup' }
    foreach ($role in $required) {
        $n = @(Get-OmaEntries $entries $role).Count
        if ($n -ne 1) { throw "check ${Pass}: expected exactly one $role call, found $n" }
    }
    foreach ($role in $PayloadExes.Keys) {
        if ($Pass -eq 'apply' -and @(Get-OmaEntries $entries $role).Count -gt 0) { throw "check apply: the $role is not part of the apply pass" }
    }
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
    foreach ($role in 'app', 'uninstaller', 'service', 'overlay') {
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

# --- verification (spec §5.1, plan L4/L5) ---------------------------------------------------------
#
# Every check returns a list of problems (strings); an empty list means the file passed. Access to
# signatures, certificate chains, signtool, version resources, 7-Zip and the certificate store goes
# through injectable providers so the tests use fakes; the defaults are the real ones and
# scripts/verify-signatures.ps1 exposes no way to replace them.

$script:SignPathFoundationCn = 'SignPath Foundation'
$script:OwnPayloadNames = @('oma-app.exe', 'oma-service.exe', 'oma-overlay.exe')
# Intel's PresentMon console, shipped unmodified with the service (M7b): checked against its own
# pins, never against our signing policy.
$script:PresentMonName = 'PresentMon-2.6.0-x64.exe'

$script:DefaultSignatureProvider = { param($Path) Get-AuthenticodeSignature -LiteralPath $Path }
$script:DefaultEmbeddedSignatureProvider = { param($Path) Get-OmaEmbeddedSignature -Path $Path }
$script:DefaultChainProvider = { param($Certificate, $Path) Get-OmaCertificateChain -Certificate $Certificate -Path $Path }
$script:DefaultVersionInfoProvider = { param($Path) [Diagnostics.FileVersionInfo]::GetVersionInfo($Path) }
$script:DefaultExtractor = {
    param($Setup, $Destination)
    $r = Invoke-OmaNative -FilePath (Get-Oma7ZipPath) -AllowFailure -ArgumentList @('x', '-y', '-bd', "-o$Destination", '--', $Setup)
    [pscustomobject]@{ ExitCode = $r.ExitCode; Output = ($r.Stdout + $r.Stderr).Trim() }
}
# Every archive entry, one path each: two entries at the same path stay two here, while `7z x -y`
# would leave a single file.
$script:DefaultLister = {
    param($Setup)
    $r = Invoke-OmaNative -FilePath (Get-Oma7ZipPath) -AllowFailure -ArgumentList @('l', '-slt', '-ba', '--', $Setup)
    [pscustomobject]@{ ExitCode = $r.ExitCode; Paths = @(ConvertFrom-Oma7ZipListing $r.Stdout); Output = ($r.Stdout + $r.Stderr).Trim() }
}

<#
.SYNOPSIS
  The file paths of a `7z l -slt -ba` listing: one "Path = " record per entry, folders
  ("Folder = +" or a D attribute) left out, duplicates kept.
#>
function ConvertFrom-Oma7ZipListing {
    param([AllowEmptyString()] [string]$Text)
    $paths = [Collections.Generic.List[string]]::new()
    $current = $null
    $folder = $false
    foreach ($line in @($Text -split '\r?\n') + 'Path = ') {
        if ($line.StartsWith('Path = ')) {
            if ($null -ne $current -and -not $folder) { $paths.Add($current) }
            $current = $line.Substring(7)
            $folder = $false
        } elseif ($line -ceq 'Folder = +' -or $line -cmatch '^Attributes = \S*D') {
            $folder = $true
        }
    }
    $paths.ToArray()
}

# Cert:\LocalMachine\Root, not CurrentUser\Root: adding to the user root store opens a Windows
# confirmation dialog, which blocks a runner without a desktop (plan L5).
function Use-OmaRootStore([Security.Cryptography.X509Certificates.OpenFlags]$Flags, [scriptblock]$Action) {
    $store = [Security.Cryptography.X509Certificates.X509Store]::new('Root', 'LocalMachine')
    try {
        $store.Open($Flags)
        & $Action $store
    } finally {
        $store.Dispose()
    }
}

$script:DefaultTrustStore = @{
    IsElevated = {
        ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
            [Security.Principal.WindowsBuiltInRole]::Administrator)
    }
    # A single public certificate (DER or PEM); a PFX is refused by the loader.
    LoadRoot   = { param($Path) [Security.Cryptography.X509Certificates.X509CertificateLoader]::LoadCertificateFromFile($Path) }
    Contains   = {
        param($Thumbprint)
        Use-OmaRootStore 'ReadOnly' { param($s) $s.Certificates.Find('FindByThumbprint', $Thumbprint, $false).Count -gt 0 }
    }
    Add        = { param($Certificate) Use-OmaRootStore 'ReadWrite' { param($s) $s.Add($Certificate) } }
    Remove     = {
        param($Thumbprint)
        Use-OmaRootStore 'ReadWrite' {
            param($s)
            foreach ($c in @($s.Certificates.Find('FindByThumbprint', $Thumbprint, $false))) { $s.Remove($c) }
        }
    }
}

function Get-OmaProperty($Object, [string]$Name) {
    if ($null -eq $Object) { return $null }
    if ($Object -is [Collections.IDictionary]) { return $Object[$Name] }
    $p = $Object.PSObject.Properties[$Name]
    if ($null -eq $p) { return $null }
    $p.Value
}

<#
.SYNOPSIS
  signtool.exe from the newest Windows 10/11 SDK (x64), else from PATH, else $null.
#>
function Get-OmaSignToolPath {
    $kits = if (${env:ProgramFiles(x86)}) { Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin' }
    if ($kits -and (Test-Path -LiteralPath $kits -PathType Container)) {
        $newest = Get-ChildItem -LiteralPath $kits -Directory |
            Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' -and (Test-Path -LiteralPath (Join-Path $_.FullName 'x64\signtool.exe') -PathType Leaf) } |
            Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
        if ($newest) { return Join-Path $newest.FullName 'x64\signtool.exe' }
    }
    $cmd = Get-Command 'signtool.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($cmd) { return $cmd.Source }
    $null
}

<#
.SYNOPSIS
  7z.exe from the default 7-Zip install directory, else from PATH; throws when missing.
#>
function Get-Oma7ZipPath {
    foreach ($dir in @($env:ProgramFiles, $env:ProgramW6432) | Where-Object { $_ } | Select-Object -Unique) {
        $exe = Join-Path $dir '7-Zip\7z.exe'
        if (Test-Path -LiteralPath $exe -PathType Leaf) { return $exe }
    }
    $cmd = Get-Command '7z.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($cmd) { return $cmd.Source }
    throw '7-Zip not found (expected C:\Program Files\7-Zip\7z.exe or 7z on PATH)'
}

<#
.SYNOPSIS
  Verifies the embedded Authenticode signature of a file with the Windows verifier (signtool).
.DESCRIPTION
  Returns @{ Embedded; SignatureValid; TimestampValid; Detail }. No catalog option (/a, /ad, /as,
  /ag, /c) is passed, so only a signature embedded in the file counts: a catalog-signed Windows
  file reports "No signature found". /pa uses the Default Authentication Verification Policy and
  /all checks every signature. The first run gives the cryptographic outcome; the second adds
  /tw, which turns a missing timestamp into a warning (exit code 2), and requires one verified
  timestamp per signature. Warnings and errors are never success.
#>
function Get-OmaEmbeddedSignature {
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string]$Path)
    $signtool = Get-OmaSignToolPath
    if (-not $signtool) { throw 'signtool.exe not found: install the Windows SDK' }
    $clean = { param($r, [string]$Text) $r.ExitCode -eq 0 -and $Text -match 'Number of errors: 0' -and $Text -match 'Number of warnings: 0' }

    $sig = Invoke-OmaNative -FilePath $signtool -AllowFailure -ArgumentList @('verify', '/pa', '/all', '/v', $Path)
    $sigText = "$($sig.Stdout)`n$($sig.Stderr)"
    $embedded = $sigText -notmatch 'No signature found' -and $sigText -notmatch 'file format cannot be verified'
    $signatureValid = $embedded -and (& $clean $sig $sigText) -and $sigText -match 'Successfully verified'

    $ts = Invoke-OmaNative -FilePath $signtool -AllowFailure -ArgumentList @('verify', '/pa', '/all', '/tw', '/v', $Path)
    $tsText = "$($ts.Stdout)`n$($ts.Stderr)"
    $signatures = [regex]::Matches($tsText, 'Signature Index: ').Count
    $stamped = [regex]::Matches($tsText, 'The signature is timestamped: ').Count
    $timestampValid = $signatureValid -and (& $clean $ts $tsText) -and $signatures -ge 1 -and $stamped -eq $signatures

    $lines = @(($sigText + "`n" + $tsText) -split '\r?\n' | Where-Object { $_ -match 'SignTool Error|SignTool Warning|Number of (errors|warnings)' } |
            ForEach-Object Trim | Select-Object -Unique)
    [pscustomobject]@{
        Embedded       = $embedded
        SignatureValid = $signatureValid
        TimestampValid = $timestampValid
        Detail         = "signtool exit codes $($sig.ExitCode)/$($ts.ExitCode): $($lines -join '; ')"
    }
}

<#
.SYNOPSIS
  Thumbprints of the chain built for a certificate, leaf first, and the last one as RootThumbprint.
  With -Path, the certificates embedded in that file's signature are offered to the chain builder.
  Trust is decided by the Authenticode status; the chain only tells which root it ends at.
#>
function Get-OmaCertificateChain {
    param([Parameter(Mandatory)] $Certificate, [string]$Path)
    $chain = [Security.Cryptography.X509Certificates.X509Chain]::new()
    try {
        # Revocation is checked by WinVerifyTrust (Get-AuthenticodeSignature, signtool).
        $chain.ChainPolicy.RevocationMode = 'NoCheck'
        if ($Path) {
            # The certificates embedded in the file's signature (on Windows, Import reads the PKCS #7
            # of a signed PE), so an intermediate missing from the local stores does not end the
            # chain early. They only help building it: the root must still be a pinned one.
            $embedded = [Security.Cryptography.X509Certificates.X509Certificate2Collection]::new()
            $embedded.Import($Path)
            $chain.ChainPolicy.ExtraStore.AddRange($embedded)
        }
        [void]$chain.Build($Certificate)
        $thumbprints = @($chain.ChainElements | ForEach-Object { $_.Certificate.Thumbprint })
        [pscustomobject]@{ Thumbprints = $thumbprints; RootThumbprint = if ($thumbprints.Count) { $thumbprints[-1] } }
    } finally {
        $chain.Dispose()
    }
}

# The CN values of a distinguished name, or none if it does not parse.
function Get-OmaCommonNames([string]$Subject) {
    try { $dn = [Security.Cryptography.X509Certificates.X500DistinguishedName]::new($Subject) } catch { return @() }
    @($dn.EnumerateRelativeDistinguishedNames() |
            Where-Object { -not $_.HasMultipleElements -and $_.GetSingleElementType().Value -eq '2.5.4.3' } |
            ForEach-Object { $_.GetSingleElementValue() })
}

# The approved certificates of a policy (plan L4), or a problem. An empty required field fails the
# policy; so does a malformed thumbprint.
function Get-OmaPolicyCertificates($Certificates, [string]$Policy) {
    $none = "no approved certificate configured for policy $Policy"
    $c = Get-OmaProperty $Certificates $Policy
    $subject = [string](Get-OmaProperty $c 'subject')
    $thumbprints = @(Get-OmaProperty $c 'thumbprints' | Where-Object { $_ })
    $fields = [ordered]@{ subject = $subject; thumbprints = $thumbprints }
    if ($Policy -eq 'test') {
        $fields.rootThumbprints = @(Get-OmaProperty $c 'rootThumbprints' | Where-Object { $_ })
        $fields.rootCertificatePath = [string](Get-OmaProperty $c 'rootCertificatePath')
    }
    foreach ($k in $fields.Keys) {
        if (@($fields[$k] | Where-Object { $_ }).Count -eq 0) { return [pscustomobject]@{ Problem = $none } }
    }
    foreach ($k in @('thumbprints', 'rootThumbprints') | Where-Object { $fields.Contains($_) }) {
        foreach ($t in $fields[$k]) {
            if ("$t" -notmatch '^[0-9A-Fa-f]{40}$') { return [pscustomobject]@{ Problem = "invalid $Policy.$k entry in .signpath/certificates.json: '$t'" } }
        }
        $fields[$k] = @($fields[$k] | ForEach-Object { "$_".ToUpperInvariant() })
    }
    if ($Policy -eq 'release' -and @(Get-OmaCommonNames $subject) -cnotcontains $script:SignPathFoundationCn) {
        return [pscustomobject]@{ Problem = "release.subject in .signpath/certificates.json must be the exact DN with CN=$($script:SignPathFoundationCn): '$subject'" }
    }
    if ($Policy -eq 'test' -and -not [IO.Path]::IsPathFullyQualified($fields.rootCertificatePath)) {
        $fields.rootCertificatePath = Join-Path (Resolve-OmaPath (Join-Path $PSScriptRoot '..\..')) $fields.rootCertificatePath
    }
    $fields.Problem = $null
    [pscustomobject]$fields
}

# Plan L5 guard: the test root is trusted only on a declared isolated machine, as administrator.
function Get-OmaIsolationProblem([hashtable]$TrustStore) {
    $declared = $env:RUNNER_ENVIRONMENT -eq 'github-hosted' -or $env:OMA_ISOLATED_TRUST -eq '1'
    if (-not $declared) {
        return 'policy test needs an isolated environment (a GitHub-hosted runner, or OMA_ISOLATED_TRUST=1 in a VM or Windows Sandbox): no test root imported'
    }
    if (-not (& $TrustStore.IsElevated)) {
        return 'policy test must run as administrator to trust its root in Cert:\LocalMachine\Root: no test root imported'
    }
    $null
}

# Trusts the pinned test root for the duration of $Action and removes it afterwards, but only if
# this call added it: a root that was already trusted stays.
function Invoke-OmaWithTestRoot($Config, [hashtable]$TrustStore, [scriptblock]$Action) {
    $root = & $TrustStore.LoadRoot $Config.rootCertificatePath
    if ($null -eq $root) { throw "cannot load the test root certificate $($Config.rootCertificatePath)" }
    if ($root.HasPrivateKey) { throw "the test root certificate file must hold a public certificate only: $($Config.rootCertificatePath)" }
    $thumbprint = "$($root.Thumbprint)".ToUpperInvariant()
    if ($thumbprint -notin $Config.rootThumbprints) {
        throw "the test root certificate $($Config.rootCertificatePath) has thumbprint $thumbprint, which is not in test.rootThumbprints"
    }
    $added = $false
    try {
        if (-not (& $TrustStore.Contains $thumbprint)) {
            $added = $true
            & $TrustStore.Add $root
        }
        & $Action
    } finally {
        if ($added) { & $TrustStore.Remove $thumbprint }
    }
}

function Get-OmaSignatureProblems([string]$Path, [string]$Policy, $Config, [scriptblock]$SignatureProvider,
    [scriptblock]$ChainProvider, [scriptblock]$EmbeddedSignatureProvider) {
    $name = Split-Path -Leaf $Path
    $sig = & $SignatureProvider $Path
    if ($null -eq $sig) { return "${name}: no signature information" }
    $status = "$(Get-OmaProperty $sig 'Status')"
    if ($status -cne 'Valid') { "${name}: Authenticode status $status ($(Get-OmaProperty $sig 'StatusMessage'))" }
    $type = Get-OmaProperty $sig 'SignatureType'
    if ("$type" -cne 'Authenticode') {
        "${name}: signature type '$type'; an embedded Authenticode signature is required, a catalog signature is not accepted"
    }
    $cert = Get-OmaProperty $sig 'SignerCertificate'
    if ($null -eq $cert) {
        "${name}: no signer certificate"
    } else {
        $subject = [string](Get-OmaProperty $cert 'Subject')
        $thumbprint = "$(Get-OmaProperty $cert 'Thumbprint')".ToUpperInvariant()
        if ($subject -cne $Config.subject) { "${name}: signer '$subject', expected '$($Config.subject)'" }
        if ($thumbprint -notin $Config.thumbprints) { "${name}: signer thumbprint $thumbprint is not an approved $Policy certificate" }
        if ($Policy -eq 'test') {
            $root = "$(Get-OmaProperty (& $ChainProvider $cert $Path) 'RootThumbprint')".ToUpperInvariant()
            if ($root -notin $Config.rootThumbprints) { "${name}: the certificate chain ends at $root, not at a pinned test root" }
        }
    }
    if ($null -eq (Get-OmaProperty $sig 'TimeStamperCertificate')) { "${name}: no timestamp (no timestamping certificate)" }
    $embedded = & $EmbeddedSignatureProvider $Path
    if (-not (Get-OmaProperty $embedded 'Embedded')) {
        "${name}: no embedded signature (a catalog signature is not accepted): $(Get-OmaProperty $embedded 'Detail')"
    } elseif (-not (Get-OmaProperty $embedded 'SignatureValid')) {
        "${name}: the embedded signature does not verify: $(Get-OmaProperty $embedded 'Detail')"
    }
    if (-not (Get-OmaProperty $embedded 'TimestampValid')) {
        "${name}: the timestamp is missing or does not verify: $(Get-OmaProperty $embedded 'Detail')"
    }
}

<#
.SYNOPSIS
  Checks the embedded Authenticode signature of one file against a policy; returns the problems.
.DESCRIPTION
  release: status Valid with ordinary Windows trust, an embedded (not catalog) signature that the
  Windows verifier accepts, signer subject equal to release.subject and thumbprint in
  release.thumbprints, a timestamping certificate and a timestamp verified by signtool.
  test: the same, against the test certificates, plus a chain ending at a pinned test root. The
  pinned root is trusted in Cert:\LocalMachine\Root only for this call and only on a declared
  isolated machine as administrator (plan L5); otherwise the check stops before any import.
  Provider exceptions become problems. -TrustStore replaces the certificate store (tests).
#>
function Test-OmaSignature {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$Path,
        [Parameter(Mandatory)] [ValidateSet('release', 'test')] [string]$Policy,
        [pscustomobject]$Certificates,
        [scriptblock]$SignatureProvider = $script:DefaultSignatureProvider,
        [scriptblock]$ChainProvider = $script:DefaultChainProvider,
        [scriptblock]$EmbeddedSignatureProvider = $script:DefaultEmbeddedSignatureProvider,
        [hashtable]$TrustStore = $script:DefaultTrustStore
    )
    $config = Get-OmaPolicyCertificates $Certificates $Policy
    if ($config.Problem) { return @($config.Problem) }
    $name = Split-Path -Leaf $Path
    try {
        if ($Policy -eq 'release') {
            return @(Get-OmaSignatureProblems $Path $Policy $config $SignatureProvider $ChainProvider $EmbeddedSignatureProvider)
        }
        $guard = Get-OmaIsolationProblem $TrustStore
        if ($guard) { return @($guard) }
        @(Invoke-OmaWithTestRoot $config $TrustStore {
                Get-OmaSignatureProblems $Path $Policy $config $SignatureProvider $ChainProvider $EmbeddedSignatureProvider
            })
    } catch {
        @("${name}: signature check failed: $($_.Exception.Message)")
    }
}

<#
.SYNOPSIS
  Checks ProductName, ProductVersion and FileVersion of a file against the current version.
.DESCRIPTION
  Formats recorded by the spike: ProductName 'OpenMonitor Advanced' and ProductVersion 'X.Y.Z'
  everywhere; FileVersion 'X.Y.Z' for the app, the uninstaller and the setup (Tauri/NSIS) and
  'X.Y.Z.0' for oma-service.exe (.NET); oma-overlay.exe has 'X.Y.Z' like the app (tauri-winres
  in crates/oma-overlay/build.rs). -Name is the file name used in the messages and to
  recognise the service. Returns the problems.
#>
function Test-OmaVersionInfo {
    [CmdletBinding()]
    param(
        [AllowNull()] $VersionInfo,
        [Parameter(Mandatory)] [string]$Version,
        [Parameter(Mandatory)] [string]$Name
    )
    if ($null -eq $VersionInfo) { return @("$Name has no version information") }
    $want = [ordered]@{
        ProductName    = $ProductName
        ProductVersion = $Version
        FileVersion    = if ($Name -eq 'oma-service.exe') { "$Version.0" } else { $Version }
    }
    foreach ($field in $want.Keys) {
        $found = [string](Get-OmaProperty $VersionInfo $field)
        if ($found -cne $want[$field]) {
            "$Name has $field '$found'$(if (-not $found) { ' (missing)' }), expected '$($want[$field])'"
        }
    }
}

function Read-OmaManifestFile([string]$Path, [string]$Version) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return [pscustomobject]@{ Problem = "manifest not found: $Path" } }
    try {
        $m = Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json -AsHashtable
    } catch {
        return [pscustomobject]@{ Problem = "manifest is not valid JSON: $Path" }
    }
    if ($m -isnot [Collections.IDictionary] -or $m['schema'] -ne 1) { return [pscustomobject]@{ Problem = "unsupported manifest schema in ${Path}" } }
    if ([string]$m['version'] -cne $Version) {
        return [pscustomobject]@{ Problem = "the manifest belongs to version '$($m['version'])', not $Version" }
    }
    foreach ($k in 'collect', 'apply') { $m[$k] = @($m[$k] | Where-Object { $null -ne $_ }) }
    if ($null -eq $m['signed']) { $m['signed'] = @{} }
    [pscustomobject]@{ Problem = $null; Manifest = $m }
}

# The single collect row of a role, or a problem.
function Get-OmaSingleEntry($List, [string]$Role, [string]$Pass) {
    $rows = @(Get-OmaEntries $List $Role)
    if ($rows.Count -ne 1) { return [pscustomobject]@{ Problem = "the $Pass pass of the manifest must record exactly one $Role, found $($rows.Count)" } }
    [pscustomobject]@{ Problem = $null; Entry = $rows[0] }
}

<#
.SYNOPSIS
  Verifies a setup against the signing manifest and a policy; returns the problems.
.DESCRIPTION
  All policies: non-empty setup, the manifest of this version, the 7-Zip archive listing and
  extraction (exit codes checked), exactly one oma-app.exe, oma-service.exe, oma-overlay.exe, PawnIO_setup.exe and
  PresentMon-2.6.0-x64.exe among the listed entries (so two entries at the same path fail),
  PawnIO with the pinned hash, status Valid and pinned signer (OmaPawnIoPins.psm1), PresentMon
  with the pinned hash, status Valid and the Intel signer (OmaPresentMonPins.psm1), and the
  product metadata of the setup and the payload. release|test: every product file carries a signature accepted by
  Test-OmaSignature and the extracted payload has the hashes of the imported signed copies;
  none: the payload has the hashes of the collect pass.
  uninstall.exe: 7-Zip does not list it (spike, 7-Zip 26.01). If it ever appears it is checked
  like the payload; otherwise release|test verify the imported signed copy, its hash and its
  replacement in the apply pass, none verifies the collect pass, and a note (information stream,
  tag OmaNote) says that the installed uninstaller's signature still needs the manual check.
  The temporary extraction folder is always removed; a failed removal is reported as a problem.
#>
function Test-OmaPayload {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$Setup,
        [Parameter(Mandatory)] [ValidateSet('release', 'test', 'none')] [string]$Policy,
        [Parameter(Mandatory)] [string]$Manifest,
        [Parameter(Mandatory)] [string]$Version,
        [pscustomobject]$Certificates,
        [scriptblock]$Extractor = $script:DefaultExtractor,
        [scriptblock]$SignatureProvider = $script:DefaultSignatureProvider,
        [scriptblock]$ChainProvider = $script:DefaultChainProvider,
        [scriptblock]$VersionInfoProvider = $script:DefaultVersionInfoProvider,
        [scriptblock]$EmbeddedSignatureProvider = $script:DefaultEmbeddedSignatureProvider,
        [hashtable]$TrustStore = $script:DefaultTrustStore,
        [pscustomobject]$PawnIoPins,
        [pscustomobject]$PresentMonPins,
        [scriptblock]$Lister = $script:DefaultLister
    )
    Assert-OmaVersionString $Version
    $signedPolicy = $Policy -ne 'none'
    $problems = [Collections.Generic.List[string]]::new()
    $sigArgs = @{
        Policy = $Policy; Certificates = $Certificates; SignatureProvider = $SignatureProvider
        ChainProvider = $ChainProvider; EmbeddedSignatureProvider = $EmbeddedSignatureProvider; TrustStore = $TrustStore
    }
    $checkFile = {
        param([string]$Path, [string]$Name)
        if ($signedPolicy) { foreach ($p in @(Test-OmaSignature -Path $Path @sigArgs)) { $problems.Add($p) } }
        foreach ($p in @(Test-OmaVersionInfo -VersionInfo (& $VersionInfoProvider $Path) -Version $Version -Name $Name)) { $problems.Add($p) }
    }

    if (-not (Test-Path -LiteralPath $Setup -PathType Leaf)) { $problems.Add("setup not found: $Setup") }
    elseif ((Get-Item -LiteralPath $Setup).Length -eq 0) { $problems.Add("the setup is empty: $Setup") }
    $read = Read-OmaManifestFile $Manifest $Version
    if ($read.Problem) { $problems.Add($read.Problem) }
    if ($signedPolicy) {
        $config = Get-OmaPolicyCertificates $Certificates $Policy
        if ($config.Problem) { $problems.Add($config.Problem) }
    }
    if ($problems.Count -gt 0) { return $problems.ToArray() }
    $m = $read.Manifest
    $stateDir = Split-Path -Parent (Resolve-OmaPath $Manifest)

    # What the extracted payload must match: the signed copies, or the collect pass when unsigned.
    $want = @{}
    foreach ($pair in @(@('app', 'oma-app.exe'), @('uninstaller', 'uninstall.exe'), @('service', 'oma-service.exe'), @('overlay', 'oma-overlay.exe'))) {
        if ($signedPolicy) {
            $sha = [string]$m['signed'][$pair[1]]
            if (-not $sha) { $problems.Add("the manifest has no imported signed copy of $($pair[1])") }
            $want[$pair[1]] = $sha
        } else {
            $c = Get-OmaSingleEntry $m['collect'] $pair[0] 'collect'
            if ($c.Problem) { $problems.Add($c.Problem); continue }
            if ($c.Entry['sha256'] -ne $c.Entry['after']) { $problems.Add("$($pair[1]) changed during the collect pass") }
            $want[$pair[1]] = [string]$c.Entry['sha256']
        }
    }

    & $checkFile $Setup (Split-Path -Leaf $Setup)

    # The listing counts every archive entry, so two entries at the same path (which `7z x -y`
    # would collapse into one file) fail the exactly-one rule.
    try {
        $listing = & $Lister $Setup
    } catch {
        $problems.Add("listing the setup failed: $($_.Exception.Message)")
        return $problems.ToArray()
    }
    if ($listing.ExitCode -ne 0) {
        $problems.Add("listing the setup with 7-Zip failed: exit code $($listing.ExitCode) $($listing.Output)")
        return $problems.ToArray()
    }
    $entryNames = @($listing.Paths | ForEach-Object { ("$_" -split '[\\/]')[-1] })
    $names = @($script:OwnPayloadNames) + 'PawnIO_setup.exe' + $script:PresentMonName + 'uninstall.exe'
    $listed = @{}
    foreach ($n in $names) {
        $listed[$n] = @($entryNames | Where-Object { $_ -ieq $n }).Count
        $optional = $n -eq 'uninstall.exe' -and $listed[$n] -eq 0
        if ($listed[$n] -ne 1 -and -not $optional) {
            $problems.Add("the setup must contain exactly one $n in the archive listing, found $($listed[$n])")
        }
    }

    $dest = Join-Path ([IO.Path]::GetTempPath()) ('oma-verify-' + [guid]::NewGuid().ToString('N'))
    # Runs in a child scope; `return` only leaves the block, so the cleanup below always runs.
    $inspect = {
        try {
            $x = & $Extractor $Setup $dest
        } catch {
            $problems.Add("extracting the setup failed: $($_.Exception.Message)")
            return
        }
        if ($x.ExitCode -ne 0) {
            $problems.Add("extracting the setup with 7-Zip failed: exit code $($x.ExitCode) $($x.Output)")
            return
        }
        $files = @(if (Test-Path -LiteralPath $dest) { Get-ChildItem -LiteralPath $dest -Recurse -File -Force })
        $found = @{}
        foreach ($n in $names) {
            $found[$n] = @($files | Where-Object { $_.Name -ieq $n })
            if ($listed[$n] -eq 1 -and $found[$n].Count -ne 1) {
                $problems.Add("the archive lists one $n but the extraction produced $($found[$n].Count)")
            }
        }
        # Only files listed and extracted exactly once are inspected further.
        $single = { param($n) $listed[$n] -eq 1 -and $found[$n].Count -eq 1 }

        foreach ($n in @($script:OwnPayloadNames) + 'uninstall.exe') {
            if (-not (& $single $n)) { continue }
            $path = $found[$n][0].FullName
            $sha = Get-OmaSha256 $path
            $source = if ($signedPolicy) { 'the signed copy' } else { 'the collect pass' }
            if ($sha -ne $want[$n]) { $problems.Add("$n in the setup has SHA-256 $sha, expected $($want[$n]) from $source") }
            & $checkFile $path $n
        }

        if (& $single 'PawnIO_setup.exe') {
            $pins = if ($PawnIoPins) { $PawnIoPins } else { Get-OmaPawnIoPins }
            $p = Test-OmaPawnIoSetup -Path $found['PawnIO_setup.exe'][0].FullName -Pins $pins -SignatureProvider $SignatureProvider
            if ($p) { $problems.Add("PawnIO_setup.exe: $p") }
        }

        if (& $single $script:PresentMonName) {
            $pins = if ($PresentMonPins) { $PresentMonPins } else { Get-OmaPresentMonPins }
            $p = Test-OmaPresentMonExe -Path $found[$script:PresentMonName][0].FullName -Pins $pins -SignatureProvider $SignatureProvider
            if ($p) { $problems.Add("${script:PresentMonName}: $p") }
        }

        if ($listed['uninstall.exe'] -eq 0) {
            $collected = Get-OmaSingleEntry $m['collect'] 'uninstaller' 'collect'
            if ($collected.Problem) { $problems.Add($collected.Problem) }
            elseif ($collected.Entry['sha256'] -ne $collected.Entry['after']) { $problems.Add('the uninstaller changed during the collect pass') }
            if ($signedPolicy) {
                $copy = Join-Path $stateDir 'signed\uninstall.exe'
                if (-not (Test-Path -LiteralPath $copy -PathType Leaf)) {
                    $problems.Add("the imported signed uninstall.exe is missing: $copy")
                } else {
                    $sha = Get-OmaSha256 $copy
                    if ($sha -ne $want['uninstall.exe']) {
                        $problems.Add("signed copy of uninstall.exe has SHA-256 $sha, not its imported hash $($want['uninstall.exe'])")
                    }
                    & $checkFile $copy 'uninstall.exe'
                }
                $applied = Get-OmaSingleEntry $m['apply'] 'uninstaller' 'apply'
                if ($applied.Problem) {
                    $problems.Add($applied.Problem)
                } elseif (-not $collected.Problem -and ($applied.Entry['sha256'] -ne $collected.Entry['sha256'] -or $applied.Entry['after'] -ne $want['uninstall.exe'])) {
                    $problems.Add('the uninstaller was not replaced with its signed copy in the apply pass')
                }
            }
            $checked = if ($signedPolicy) { 'the manifest and the imported signed copy' } else { 'the collect pass in the manifest' }
            Write-Information -Tags 'OmaNote' -MessageData ("installed uninstaller signature not verified: 7-Zip does not list uninstall.exe, so the check covered only $checked; " +
                "install the setup in Windows Sandbox or a VM and check C:\Program Files\$ProductName\uninstall.exe (spec §8.2)")
        }
    }
    $cleanup = $null
    try {
        $null = & $inspect
    } finally {
        if (Test-Path -LiteralPath $dest) {
            try {
                Remove-Item -LiteralPath $dest -Recurse -Force -ErrorAction Stop
            } catch {
                $cleanup = "cannot remove the temporary extraction folder ${dest}: $($_.Exception.Message)"
            }
        }
    }
    if ($cleanup) { $problems.Add($cleanup) }
    $problems.ToArray()
}

<#
.SYNOPSIS
  Verifies the signed files returned by SignPath before they are imported; returns the problems.
.DESCRIPTION
  The directory must hold exactly oma-app.exe, uninstall.exe, oma-service.exe and oma-overlay.exe. Each must pass
  Test-OmaSignature and Test-OmaVersionInfo. Hashes are not compared here: the signed copies are
  not in the manifest until import-signed records them.
#>
function Test-OmaSignedFiles {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$Directory,
        [Parameter(Mandatory)] [ValidateSet('release', 'test')] [string]$Policy,
        [Parameter(Mandatory)] [string]$Version,
        [pscustomobject]$Certificates,
        [scriptblock]$SignatureProvider = $script:DefaultSignatureProvider,
        [scriptblock]$ChainProvider = $script:DefaultChainProvider,
        [scriptblock]$VersionInfoProvider = $script:DefaultVersionInfoProvider,
        [scriptblock]$EmbeddedSignatureProvider = $script:DefaultEmbeddedSignatureProvider,
        [hashtable]$TrustStore = $script:DefaultTrustStore
    )
    Assert-OmaVersionString $Version
    $config = Get-OmaPolicyCertificates $Certificates $Policy
    if ($config.Problem) { return @($config.Problem) }
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) { return @("signed files directory not found: $Directory") }
    $problems = [Collections.Generic.List[string]]::new()
    $items = @(Get-ChildItem -LiteralPath $Directory -Force)
    $files = @($items | Where-Object { -not $_.PSIsContainer } | ForEach-Object Name)
    $missing = @($SignedNames | Where-Object { $_ -notin $files })
    $unexpected = @($items | Where-Object { $_.PSIsContainer -or $_.Name -notin $SignedNames } | ForEach-Object Name)
    if ($missing.Count -gt 0 -or $unexpected.Count -gt 0) {
        $problems.Add("the signed files directory must hold exactly $($SignedNames -join ', '); missing: $($missing -join ', '); unexpected: $($unexpected -join ', ')")
    }
    foreach ($n in $SignedNames | Where-Object { $_ -in $files }) {
        $path = Join-Path $Directory $n
        foreach ($p in @(Test-OmaSignature -Path $path -Policy $Policy -Certificates $Certificates -SignatureProvider $SignatureProvider `
                    -ChainProvider $ChainProvider -EmbeddedSignatureProvider $EmbeddedSignatureProvider -TrustStore $TrustStore)) { $problems.Add($p) }
        foreach ($p in @(Test-OmaVersionInfo -VersionInfo (& $VersionInfoProvider $path) -Version $Version -Name $n)) { $problems.Add($p) }
    }
    $problems.ToArray()
}

Export-ModuleMember -Function Initialize-OmaSigningState, Invoke-OmaSignShim, Register-OmaPayload,
    Import-OmaSignedFiles, Assert-OmaSigningPass, Get-OmaPluginPathPattern,
    Assert-OmaVersionString, Test-OmaVersionInfo, Test-OmaSignature, Test-OmaPayload, Test-OmaSignedFiles,
    Get-OmaSignToolPath, Get-OmaEmbeddedSignature, Get-OmaCertificateChain, Get-Oma7ZipPath, ConvertFrom-Oma7ZipListing,
    Get-OmaPolicyCertificates `
    -Variable UninstallerPathPattern
