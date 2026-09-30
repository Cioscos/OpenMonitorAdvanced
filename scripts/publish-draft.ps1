<#
.SYNOPSIS
  Creates or updates the draft release of a tag and verifies its assets (spec M6a §5.3).
.DESCRIPTION
    pwsh scripts/publish-draft.ps1 -Repo <owner/name> -Tag vX.Y.Z -Version X.Y.Z
        -Setup <OpenMonitor.Advanced_X.Y.Z_x64-setup.exe> -Sums <SHA256SUMS.txt> -Signed true|false
        [-TemplatePath .github/release-notes-template.md]

  No release: gh release create --draft --verify-tag with both assets. A draft: its hand-written
  changes are kept, the generated block is rewritten, then the assets are replaced with
  --clobber; isDraft is re-read before every write. A published release is never touched.
  At the end the draft must list exactly the two assets, uploaded, with the local sizes and
  SHA-256. On any failure: do not publish the draft, re-run the workflow.

  -Signed takes $true/$false or the strings 'true'/'false'; anything else fails.
  gh writes to GitHub with the token in GH_TOKEN. Exit code 0 only when the draft is verified.
#>
#Requires -Version 7

[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Repo,
    [Parameter(Mandatory)] [string]$Tag,
    [Parameter(Mandatory)] [string]$Version,
    [Parameter(Mandatory)] [string]$Setup,
    [Parameter(Mandatory)] [string]$Sums,
    [Parameter(Mandatory)] $Signed,
    [string]$TemplatePath = (Join-Path (Split-Path -Parent $PSScriptRoot) '.github\release-notes-template.md')
)

$ErrorActionPreference = 'Stop'

function Write-Problem([string]$Message) {
    if ($env:GITHUB_ACTIONS -eq 'true') { Write-Output "::error::$Message" } else { Write-Output "FAIL: $Message" }
}

try {
    Import-Module (Join-Path $PSScriptRoot 'lib\OmaRelease.psm1') -Force
    $signedValue = if ($Signed -is [bool]) { $Signed } else { ConvertFrom-OmaBoolText -Name 'Signed' -Text "$Signed" }
    $here = (Get-Location -PSProvider FileSystem).ProviderPath
    $result = Publish-OmaDraft -Repo $Repo -Tag $Tag -Version $Version -Setup ([IO.Path]::GetFullPath($Setup, $here)) `
        -Sums ([IO.Path]::GetFullPath($Sums, $here)) -Signed $signedValue -TemplatePath ([IO.Path]::GetFullPath($TemplatePath, $here)) `
        -InformationAction Continue
    Write-Output "OK: draft $($result.Tag) $($result.Action); remote assets $($result.Setup) and SHA256SUMS.txt verified"
    exit 0
} catch {
    Write-Problem $_.Exception.Message
    exit 1
}
