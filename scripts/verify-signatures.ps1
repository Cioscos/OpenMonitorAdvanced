<#
.SYNOPSIS
  Verifies the signatures and the payload of the installer (spec M6a §5.1, plan L4/L5).
.DESCRIPTION
    pwsh scripts/verify-signatures.ps1 -Policy release|test|none -Version X.Y.Z -Setup <setup.exe> -Manifest <target\signing\manifest.json>
    pwsh scripts/verify-signatures.ps1 -Policy release|test -Version X.Y.Z -Files <dir with the three signed files>

  -Setup (the manifest is required with every policy): extracts the setup with 7-Zip and checks
  the payload, PawnIO, the product metadata and, for release|test, the signatures and the hashes
  of the imported signed copies; for none, the hashes of the collect pass. 7-Zip does not list
  the uninstaller: its imported copy and its replacement are checked in the manifest and the
  summary says that the installed uninstaller still needs the manual check.
  -Files: before import-signed, the directory must hold exactly oma-app.exe, uninstall.exe and
  oma-service.exe, each with an accepted signature and the expected metadata.

  release: ordinary Windows trust, signer from .signpath/certificates.json (exact subject and an
  approved thumbprint), embedded signature and timestamp verified by signtool (Windows SDK).
  test: the same against the test certificates; the pinned test root is trusted in
  Cert:\LocalMachine\Root only on a GitHub-hosted runner or with OMA_ISOLATED_TRUST=1, as
  administrator, and removed afterwards if this script added it.
  none: content checks only; writes "::warning::unsigned build".

  The certificates always come from .signpath/certificates.json and every check uses the real
  providers: no parameter disables or replaces them. Exit code 0 only when nothing failed.
#>
#Requires -Version 7

[CmdletBinding(DefaultParameterSetName = 'Setup')]
param(
    [Parameter(Mandatory)] [ValidateSet('release', 'test', 'none')] [string]$Policy,
    [Parameter(Mandatory)] [string]$Version,
    [Parameter(Mandatory, ParameterSetName = 'Setup')] [string]$Setup,
    [Parameter(Mandatory, ParameterSetName = 'Setup')] [string]$Manifest,
    [Parameter(Mandatory, ParameterSetName = 'Files')] [string]$Files
)

$ErrorActionPreference = 'Stop'

function Write-Problem([string]$Message) {
    if ($env:GITHUB_ACTIONS -eq 'true') { Write-Output "::error::$Message" } else { Write-Output "FAIL: $Message" }
}

try {
    if ($Policy -eq 'none') { Write-Output '::warning::unsigned build' }
    Import-Module (Join-Path $PSScriptRoot 'lib\OmaCommon.psm1') -Force
    Import-Module (Join-Path $PSScriptRoot 'lib\OmaSigning.psm1') -Force
    Assert-OmaVersionString $Version
    $repoRoot = Resolve-OmaPath (Join-Path $PSScriptRoot '..')
    $here = (Get-Location -PSProvider FileSystem).ProviderPath
    $certificates = $null
    if ($Policy -ne 'none') {
        $certificates = Get-Content -Raw -LiteralPath (Join-Path $repoRoot '.signpath\certificates.json') | ConvertFrom-Json
    }

    if ($PSCmdlet.ParameterSetName -eq 'Files') {
        if ($Policy -eq 'none') { throw '-Files checks signed files: use -Policy release or test' }
        $what = [IO.Path]::GetFullPath($Files, $here)
        $problems = @(Test-OmaSignedFiles -Directory $what -Policy $Policy -Version $Version -Certificates $certificates)
        $notes = @()
    } else {
        $what = [IO.Path]::GetFullPath($Setup, $here)
        $problems = @(Test-OmaPayload -Setup $what -Policy $Policy -Manifest ([IO.Path]::GetFullPath($Manifest, $here)) `
                -Version $Version -Certificates $certificates -InformationVariable notes -InformationAction SilentlyContinue)
    }

    foreach ($n in $notes) { Write-Output "NOTE: $n" }
    if ($problems.Count -gt 0) {
        foreach ($p in $problems) { Write-Problem $p }
        Write-Output "verify-signatures: $($problems.Count) problem(s) with policy $Policy in $what"
        exit 1
    }
    Write-Output "OK: $what passed the $Policy verification for version $Version"
    exit 0
} catch {
    Write-Problem $_.Exception.Message
    exit 1
}
