<#
.SYNOPSIS
  Release preflight (spec M6a §4.1-4.2, plan L7): runs before any build or signing request.
.DESCRIPTION
    pwsh scripts/release-preflight.ps1 -EventName push|workflow_dispatch -Ref <github.ref> -Sha <github.sha>
        -Repo <owner/name> -HasToken true|false [-OrganizationId <id>] [-RequireSigning true|false]
        [-Tag vX.Y.Z] [-SignPathProjectSlug <slug>]

  Checks, in order: the run ref (a tag push refs/tags/vX.Y.Z, or a dispatch from refs/heads/main),
  the signing mode (both SignPath credentials or none; REQUIRE_SIGNING=true forbids none) and,
  with signing enabled, the SignPath project slug and the certificates of .signpath/certificates.json
  for the selected policy; then the CI gate on the same commit and, for a tag, that the release is
  not already published.

  -HasToken and -RequireSigning are strings ('true'/'false'; an empty -RequireSigning means false),
  converted explicitly because [bool]'false' is true in PowerShell. Unknown values fail.
  Writes signing=enabled|disabled, signpath-policy=release-signing|test-signing and
  verify-policy=release|test|none to $env:GITHUB_OUTPUT when it is set, and prints them.
  gh reads GitHub with the token in GH_TOKEN. Exit code 0 only when every check passed.
#>
#Requires -Version 7

[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$EventName,
    [Parameter(Mandatory)] [string]$Ref,
    [Parameter(Mandatory)] [string]$Sha,
    [Parameter(Mandatory)] [string]$Repo,
    [Parameter(Mandatory)] [string]$HasToken,
    [string]$OrganizationId = '',
    [string]$RequireSigning = '',
    [string]$Tag = '',
    [string]$SignPathProjectSlug = ''
)

$ErrorActionPreference = 'Stop'

function Write-Problem([string]$Message) {
    if ($env:GITHUB_ACTIONS -eq 'true') { Write-Output "::error::$Message" } else { Write-Output "FAIL: $Message" }
}

try {
    Import-Module (Join-Path $PSScriptRoot 'lib\OmaRelease.psm1') -Force
    $outputs = Invoke-OmaReleasePreflight -EventName $EventName -Ref $Ref -Sha $Sha -Repo $Repo -HasToken $HasToken `
        -OrganizationId $OrganizationId -RequireSigning $RequireSigning -Tag $Tag -SignPathProjectSlug $SignPathProjectSlug `
        -OutputPath $env:GITHUB_OUTPUT -InformationAction Continue
    foreach ($k in $outputs.Keys) { Write-Output "$k=$($outputs[$k])" }
    exit 0
} catch {
    Write-Problem $_.Exception.Message
    exit 1
}
