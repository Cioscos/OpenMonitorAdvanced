<#
.SYNOPSIS
  Two-pass signing shim for Tauri's bundle.windows.signCommand (spec M6a §3.2). It signs
  nothing: it collects the files Tauri would sign, then swaps in the copies SignPath signed.
.DESCRIPTION
  Modes (the logic lives in scripts/lib/OmaSigning.psm1):
    init              recreates the empty state under <repo>\target\ and writes manifest.json,
                      tauri.sign.collect.json and tauri.sign.apply.json (needs the run context);
    collect | apply   one signCommand call (-Path %1), from the generated Tauri configs;
    register-payload  records one payload exe in collect: target\installer-payload\service\oma-service.exe
                      or target\installer-payload\overlay\oma-overlay.exe (one call each);
    import-signed     copies exactly the four signed files from -From into signed/;
    check             gate after a bundle (-Pass collect|apply, needs the run context).

    pwsh scripts/sign-shim.ps1 -Mode init -StateRoot <abs>\target\signing -Commit <sha> -Version X.Y.Z -RunId <id> -RunAttempt <n>
    cd app; pnpm tauri build --bundles nsis --config ../target/signing/tauri.sign.collect.json -v '--' --locked
    pwsh scripts/sign-shim.ps1 -Mode register-payload -StateRoot <abs>\target\signing -Path <abs>\target\installer-payload\service\oma-service.exe -Commit <sha> -Version X.Y.Z -RunId <id> -RunAttempt <n>
    pwsh scripts/sign-shim.ps1 -Mode register-payload -StateRoot <abs>\target\signing -Path <abs>\target\installer-payload\overlay\oma-overlay.exe -Commit <sha> -Version X.Y.Z -RunId <id> -RunAttempt <n>
    pwsh scripts/sign-shim.ps1 -Mode check -Pass collect -StateRoot <abs>\target\signing -Commit <sha> -Version X.Y.Z -RunId <id> -RunAttempt <n>

  The run context (-Commit, -Version, -RunId, -RunAttempt) comes from the caller, never from the
  manifest being checked, and every mode but import-signed requires it (the generated configs
  pass it to collect/apply). In GitHub Actions (GITHUB_ACTIONS=true) collect, apply and
  register-payload also compare the manifest with GITHUB_SHA, GITHUB_RUN_ID and
  GITHUB_RUN_ATTEMPT, and fail if any is missing. check compares the manifest's repository root
  with -RepoRoot (default: the repository holding this script). Paths must be absolute: Tauri
  runs the shim from app\src-tauri, makensis from target\release\nsis\x64.
  Exit code 0 only on success; the error goes to stderr.
#>
#Requires -Version 7

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('init', 'collect', 'apply', 'register-payload', 'import-signed', 'check')]
    [string]$Mode,
    [string]$StateRoot,
    [string]$Path,
    [string]$From,
    [ValidateSet('collect', 'apply')]
    [string]$Pass,
    [string]$RepoRoot = (Join-Path $PSScriptRoot '..'),
    [string]$Commit,
    [string]$Version,
    [string]$RunId,
    [string]$RunAttempt
)

$ErrorActionPreference = 'Stop'

function Get-RunContext {
    $given = @($Commit, $Version, $RunId, $RunAttempt | Where-Object { $_ })
    if ($given.Count -ne 4) { throw "-Mode $Mode needs -Commit, -Version, -RunId and -RunAttempt" }
    [pscustomobject]@{ Commit = $Commit; Version = $Version; RunId = $RunId; RunAttempt = $RunAttempt }
}

function Assert-Given([string]$Name, [string]$Value) {
    if (-not $Value) { throw "-Mode $Mode needs -$Name" }
}

try {
    Import-Module (Join-Path $PSScriptRoot 'lib\OmaSigning.psm1') -Force
    Assert-Given 'StateRoot' $StateRoot
    switch ($Mode) {
        'init' {
            $ctx = Get-RunContext
            Initialize-OmaSigningState -StateRoot $StateRoot -RepoRoot $RepoRoot -Commit $ctx.Commit `
                -Version $ctx.Version -RunId $ctx.RunId -RunAttempt $ctx.RunAttempt
            Write-Output "sign-shim init: empty signing state in $StateRoot"
        }
        { $_ -in 'collect', 'apply' } {
            Assert-Given 'Path' $Path
            $e = Invoke-OmaSignShim -Mode $Mode -StateRoot $StateRoot -Path $Path -ExpectedContext (Get-RunContext)
            if ($e.role -eq 'plugin') {
                Write-Output "sign-shim ${Mode}: third-party plugin $($e.name) left intact ($($e.sha256))"
            } else {
                Write-Output "sign-shim ${Mode}: $($e.role) $($e.name) $($e.sha256) -> $($e.after)"
            }
        }
        'register-payload' {
            Assert-Given 'Path' $Path
            $e = Register-OmaPayload -StateRoot $StateRoot -Path $Path -ExpectedContext (Get-RunContext)
            Write-Output "sign-shim register-payload: $($e.name) $($e.sha256)"
        }
        'import-signed' {
            Assert-Given 'From' $From
            Import-OmaSignedFiles -StateRoot $StateRoot -From $From | Out-Null
            Write-Output "sign-shim import-signed: signed files copied from $From"
        }
        'check' {
            Assert-Given 'Pass' $Pass
            Assert-OmaSigningPass -RepoRoot $RepoRoot -StateRoot $StateRoot -Pass $Pass -ExpectedContext (Get-RunContext)
            Write-Output "sign-shim check: $Pass pass complete"
        }
    }
    exit 0
} catch {
    [Console]::Error.WriteLine("sign-shim ${Mode}: $($_.Exception.Message)")
    exit 1
}
