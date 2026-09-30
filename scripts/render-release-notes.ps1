<#
.SYNOPSIS
  Renders the draft release notes from .github/release-notes-template.md.
.DESCRIPTION
  Without -Existing it writes a fresh body. With -Existing (the body of an existing draft) it
  keeps the hand-written changes and replaces only the generated block; missing, duplicate or
  misordered markers fail before anything is written (spec M6a §5.3). The output is UTF-8
  without BOM with LF endings.

  -Signed is mandatory and takes a value, so a forgotten flag can never mean "unsigned".
  Call it as -Signed:$true or -Signed:$false. From a workflow step with a string, pass
  -Signed:([bool]::Parse($env:SIGNED)), because a plain [bool] cast of the string 'false' is
  true; this script also accepts the strings 'true' and 'false' and rejects anything else.

    pwsh scripts/render-release-notes.ps1 -Version X.Y.Z -Signed:$true -Out notes.md [-Existing body.md]
#>
#Requires -Version 7

[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Version,
    [Parameter(Mandatory)] $Signed,
    [Parameter(Mandatory)] [string]$Out,
    [string]$Existing,
    [string]$Template = (Join-Path (Split-Path -Parent $PSScriptRoot) '.github\release-notes-template.md')
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'lib\OmaReleaseNotes.psm1') -Force

$signedValue = switch ($Signed) {
    { $_ -is [bool] } { $_; break }
    { $_ -is [System.Management.Automation.SwitchParameter] } { [bool]$_; break }
    { $_ -is [string] -and $_ -ieq 'true' } { $true; break }
    { $_ -is [string] -and $_ -ieq 'false' } { $false; break }
    default { throw "-Signed must be `$true or `$false, got '$Signed'." }
}

$utf8 = [Text.UTF8Encoding]::new($false)
if ($Existing) {
    $body = $utf8.GetString([IO.File]::ReadAllBytes($Existing)).TrimStart([char]0xFEFF)
    $text = Update-OmaReleaseNotes -ExistingBody $body -TemplatePath $Template -Version $Version -Signed $signedValue
} else {
    $text = New-OmaReleaseNotes -TemplatePath $Template -Version $Version -Signed $signedValue
}
[IO.File]::WriteAllBytes($Out, $utf8.GetBytes($text))
