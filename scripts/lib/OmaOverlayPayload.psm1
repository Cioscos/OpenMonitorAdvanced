#Requires -Version 7
# Builds and stages oma-overlay.exe for the installer (plan M7c DP7): scripts/build-installer-payload.ps1
# calls Save-OmaOverlayExe, and app/src-tauri/nsis/oma.nsh installs the staged file as
# $INSTDIR\oma-overlay.exe. Kept in a module so the tests can run it in process with a fake cargo
# and a fake version reader.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'OmaCommon.psm1')
Import-Module (Join-Path $PSScriptRoot 'OmaSigning.psm1')

<#
.SYNOPSIS
  `cargo build --release --locked -p oma-overlay`, then copies the exe to -Destination.
.DESCRIPTION
  The previous staged copy is removed first, so a failed build never leaves a stale overlay for
  the installer. The built exe must carry the product metadata the signing pipeline expects
  (Test-OmaVersionInfo: ProductName "OpenMonitor Advanced", ProductVersion and FileVersion X.Y.Z,
  from crates/oma-overlay/build.rs). The copy goes through a .partial file and is checked by hash.
  Throws on any failure; returns @{ Path; Sha256; FileVersion }.
#>
function Save-OmaOverlayExe {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$CargoExe,
        [Parameter(Mandatory)] [string]$RepoRoot,
        [Parameter(Mandatory)] [string]$TargetDir,
        [Parameter(Mandatory)] [string]$Destination,
        [Parameter(Mandatory)] [string]$Version,
        [scriptblock]$VersionInfoProvider = { param($Path) [Diagnostics.FileVersionInfo]::GetVersionInfo($Path) }
    )
    $partial = "$Destination.partial"
    foreach ($stale in $Destination, $partial) {
        if (Test-Path -LiteralPath $stale) { Remove-Item -LiteralPath $stale -Force }
    }

    $cargoArgs = @(
        'build', '--release', '--locked', '-p', 'oma-overlay',
        '--manifest-path', (Join-Path $RepoRoot 'Cargo.toml'),
        '--target-dir', $TargetDir
    )
    Write-Host "Running: $CargoExe $($cargoArgs -join ' ')"
    & $CargoExe @cargoArgs 2>&1 | ForEach-Object { Write-Host $_.ToString() }
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) { throw "cargo build of oma-overlay failed with exit code $exitCode" }

    $built = Join-Path $TargetDir 'release\oma-overlay.exe'
    if (-not (Test-Path -LiteralPath $built -PathType Leaf)) { throw "oma-overlay.exe is missing from $(Split-Path $built) after the build" }
    $info = & $VersionInfoProvider $built
    $problems = @(Test-OmaVersionInfo -VersionInfo $info -Version $Version -Name 'oma-overlay.exe')
    if ($problems.Count -gt 0) { throw ($problems -join '; ') }

    New-Item -ItemType Directory -Force (Split-Path $Destination) | Out-Null
    $sha = Get-OmaSha256 $built
    Copy-Item -LiteralPath $built -Destination $partial
    if ((Get-OmaSha256 $partial) -ne $sha) {
        Remove-Item -LiteralPath $partial -Force
        throw 'the staged copy of oma-overlay.exe does not match the build'
    }
    Move-Item -LiteralPath $partial -Destination $Destination -Force
    [pscustomobject]@{ Path = $Destination; Sha256 = $sha; FileVersion = [string]$info.FileVersion }
}

Export-ModuleMember -Function Save-OmaOverlayExe
