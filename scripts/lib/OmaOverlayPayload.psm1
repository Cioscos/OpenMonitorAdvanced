#Requires -Version 7
# Builds and stages a helper exe (oma-overlay, plan M7c DP7; oma-load, M8a1) for the installer:
# scripts/build-installer-payload.ps1 calls Save-OmaHelperExe, and app/src-tauri/nsis/oma.nsh installs
# the staged files as $INSTDIR\oma-overlay.exe and $INSTDIR\oma-load.exe. Kept in a module so the tests can run it in process with a fake cargo
# and a fake version reader.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'OmaCommon.psm1')
Import-Module (Join-Path $PSScriptRoot 'OmaSigning.psm1')

<#
.SYNOPSIS
  `cargo build --release --locked -p <Package>`, then copies the exe to -Destination.
.DESCRIPTION
  The previous staged copy is removed first, so a failed build never leaves a stale helper for
  the installer. The built exe must carry the product metadata the signing pipeline expects
  (Test-OmaVersionInfo: ProductName "OpenMonitor Advanced", ProductVersion and FileVersion X.Y.Z,
  from crates/<Package>/build.rs). The copy goes through a .partial file and is checked by hash.
  Throws on any failure; returns @{ Path; Sha256; FileVersion }.
#>
function Save-OmaHelperExe {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [ValidateSet('oma-overlay', 'oma-load')] [string]$Package,
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
        'build', '--release', '--locked', '-p', $Package,
        '--manifest-path', (Join-Path $RepoRoot 'Cargo.toml'),
        '--target-dir', $TargetDir
    )
    Write-Host "Running: $CargoExe $($cargoArgs -join ' ')"
    & $CargoExe @cargoArgs 2>&1 | ForEach-Object { Write-Host $_.ToString() }
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) { throw "cargo build of $Package failed with exit code $exitCode" }

    $built = Join-Path $TargetDir "release\$Package.exe"
    if (-not (Test-Path -LiteralPath $built -PathType Leaf)) { throw "$($Package).exe is missing from $(Split-Path $built) after the build" }
    $info = & $VersionInfoProvider $built
    $problems = @(Test-OmaVersionInfo -VersionInfo $info -Version $Version -Name "$Package.exe")
    if ($problems.Count -gt 0) { throw ($problems -join '; ') }

    New-Item -ItemType Directory -Force (Split-Path $Destination) | Out-Null
    $sha = Get-OmaSha256 $built
    Copy-Item -LiteralPath $built -Destination $partial
    if ((Get-OmaSha256 $partial) -ne $sha) {
        Remove-Item -LiteralPath $partial -Force
        throw "the staged copy of $Package.exe does not match the build"
    }
    Move-Item -LiteralPath $partial -Destination $Destination -Force
    [pscustomobject]@{ Path = $Destination; Sha256 = $sha; FileVersion = [string]$info.FileVersion }
}

Export-ModuleMember -Function Save-OmaHelperExe
