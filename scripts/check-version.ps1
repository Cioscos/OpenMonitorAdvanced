<#
.SYNOPSIS
  Checks that the five version fields and Cargo.lock agree; with -Tag also that the tag is
  vX.Y.Z, equals the version, points at HEAD (and the run SHA) and is reachable from origin/main.
.DESCRIPTION
  Prints every problem on its own line and exits 1 if there is at least one (spec M6a §6.1).
  The release workflow always passes -ExpectedSha from the GitHub context.

    pwsh scripts/check-version.ps1 [-Tag vX.Y.Z] [-ExpectedSha <sha>]
#>
#Requires -Version 7

[CmdletBinding()]
param(
    [string]$Tag,
    [string]$ExpectedSha,
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'lib\OmaVersion.psm1') -Force

try {
    $problems = @(Test-OmaVersionConsistency -RepoRoot $RepoRoot -Tag $Tag -ExpectedSha $ExpectedSha)
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
if ($problems.Count -gt 0) {
    foreach ($p in $problems) { [Console]::Error.WriteLine($p) }
    exit 1
}
Write-Output 'version check: ok'
exit 0
