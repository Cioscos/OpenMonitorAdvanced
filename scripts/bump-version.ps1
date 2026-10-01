<#
.SYNOPSIS
  Bumps the product version to X.Y.Z in the five files and Cargo.lock (spec M6a §6.2).
.DESCRIPTION
  Refuses a version that is not canonical or not greater than the current one (numeric
  comparison), rewrites only the version substrings, runs `cargo update --workspace --offline`,
  rejects unrelated Cargo.lock changes and restores every file byte for byte on any failure.
  It creates no commit and no tag: it prints the next commands, and the push stays yours.

    pwsh scripts/bump-version.ps1 0.3.0
#>
#Requires -Version 7

[CmdletBinding()]
param(
    [Parameter(Mandatory, Position = 0)] [string]$Version,
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'lib\OmaVersion.psm1') -Force

try {
    Set-OmaVersion -RepoRoot $RepoRoot -Version $Version
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
Write-Output "Version bumped to $Version. Next:"
Write-Output "  git commit -am `"chore: release $Version`""
Write-Output "  git tag v$Version"
Write-Output "  git push origin main"
Write-Output "  # wait until the CI run of ci.yml on main is green for this commit, then:"
Write-Output "  git push origin v$Version"
exit 0
