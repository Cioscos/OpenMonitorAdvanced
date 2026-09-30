<#
.SYNOPSIS
  Two-pass signing shim for Tauri's bundle.windows.signCommand (spec M6a §3.2). It signs
  nothing: it collects the files Tauri would sign, then swaps in the copies SignPath signed.
.DESCRIPTION
  Modes (the logic lives in scripts/lib/OmaSigning.psm1):
    init              recreates the empty state under <repo>\target\ and writes manifest.json,
                      tauri.sign.collect.json and tauri.sign.apply.json (needs the run context);
    collect | apply   one signCommand call (-Path %1), from the generated Tauri configs;
    register-service  records target\installer-payload\service\oma-service.exe in collect;
    import-signed     copies exactly the three signed files from -From into signed/;
    check             gate after a bundle (-Pass collect|apply, needs the run context).

    pwsh scripts/sign-shim.ps1 -Mode init -StateRoot <abs>\target\signing -Commit <sha> -Version X.Y.Z -RunId <id> -RunAttempt <n>
    cd app; pnpm tauri build --bundles nsis --config ../target/signing/tauri.sign.collect.json -v '--' --locked
    pwsh scripts/sign-shim.ps1 -Mode register-service -StateRoot <abs>\target\signing -Path <abs>\target\installer-payload\service\oma-service.exe
    pwsh scripts/sign-shim.ps1 -Mode check -Pass collect -StateRoot <abs>\target\signing -Commit <sha> -Version X.Y.Z -RunId <id> -RunAttempt <n>

  The run context (-Commit, -Version, -RunId, -RunAttempt) comes from the caller, never from the
  manifest being checked; collect/apply compare it too when the config passes it. Paths must be
  absolute: Tauri runs the shim from app\src-tauri, makensis from target\release\nsis\x64.
  Exit code 0 only on success; the error goes to stderr.
#>
#Requires -Version 7

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('init', 'collect', 'apply', 'register-service', 'import-signed', 'check')]
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

function Get-RunContext([switch]$Required) {
    $given = @($Commit, $Version, $RunId, $RunAttempt | Where-Object { $_ })
    if ($given.Count -eq 0 -and -not $Required) { return $null }
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
            $ctx = Get-RunContext -Required
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
        'register-service' {
            Assert-Given 'Path' $Path
            $e = Register-OmaService -StateRoot $StateRoot -Path $Path
            Write-Output "sign-shim register-service: $($e.name) $($e.sha256)"
        }
        'import-signed' {
            Assert-Given 'From' $From
            Import-OmaSignedFiles -StateRoot $StateRoot -From $From | Out-Null
            Write-Output "sign-shim import-signed: signed files copied from $From"
        }
        'check' {
            Assert-Given 'Pass' $Pass
            Assert-OmaSigningPass -StateRoot $StateRoot -Pass $Pass -ExpectedContext (Get-RunContext -Required)
            Write-Output "sign-shim check: $Pass pass complete"
        }
    }
    exit 0
} catch {
    [Console]::Error.WriteLine("sign-shim ${Mode}: $($_.Exception.Message)")
    exit 1
}
