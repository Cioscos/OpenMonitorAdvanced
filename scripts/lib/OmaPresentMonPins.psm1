#Requires -Version 7
# The pins of the official PresentMon console the installer ships with the service (M7b), shared
# by the payload build (scripts/build-installer-payload.ps1) and the release verifier
# (Test-OmaPayload in scripts/lib/OmaSigning.psm1), so both check the same hash and the same
# author. Importing this module runs nothing else, and nothing here ever runs PresentMon.
#
# PresentMon 2.6.0, official console. Provenance (spike of 2026-10-04, re-checked by the M7b
# Task B12 implementer on 2026-10-05):
# - URL: https://github.com/GameTechDev/PresentMon/releases/download/v2.6.0/PresentMon-2.6.0-x64.exe
#   (release tag v2.6.0 of GameTechDev/PresentMon, 980 320 bytes);
# - SHA-256 pinned in app/src-tauri/nsis/presentmon.sha256, also read by oma.nsh at compile time
#   and by PresentMonPin in the service;
# - Authenticode: Valid, signer "CN=Intel Corporation, O=Intel Corporation, S=California, C=US",
#   issued by "Sectigo Public Code Signing CA R36", valid until 2027-08-02 (thumbprint
#   BF07AF4995EFFDB3CAA3466D6AB616136A1762CC at the time of the check), timestamped by
#   "Sectigo Public Time Stamping Signer R37";
# - no version resource (ProductName and FileVersion are empty): the version is in the file name.
# Only the subject is pinned, not the thumbprint: the hash already fixes the file, and Intel
# renews its certificate. Never update these automatically. Moving to another PresentMon release
# means repeating the checks above by hand, recording them here and updating presentmon.sha256.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'OmaCommon.psm1')

$script:FileName = 'PresentMon-2.6.0-x64.exe'
$script:SignerSubject = 'CN=Intel Corporation, O=Intel Corporation, S=California, C=US'

<#
.SYNOPSIS
  The PresentMon pins: @{ Sha256 (upper-case hex, from presentmon.sha256); SignerSubject }.
#>
function Get-OmaPresentMonPins {
    [CmdletBinding()]
    param([string]$RepoRoot = (Join-Path $PSScriptRoot '..\..'))
    $file = Join-Path $RepoRoot 'app\src-tauri\nsis\presentmon.sha256'
    $sha = (Get-Content -Raw -LiteralPath $file).Trim()
    if ($sha -cnotmatch '^[0-9A-F]{64}$') { throw 'app/src-tauri/nsis/presentmon.sha256 must hold one upper-case SHA-256' }
    [pscustomobject]@{ Sha256 = $sha; SignerSubject = $script:SignerSubject }
}

<#
.SYNOPSIS
  Checks a PresentMon console against the pins: SHA-256, Authenticode status Valid and the pinned
  signer subject. Returns the first problem as a string, or $null. Never runs the file.
.PARAMETER SignatureProvider
  param($Path) -> an object shaped like Get-AuthenticodeSignature's result. Tests inject a fake.
#>
function Test-OmaPresentMonExe {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$Path,
        [pscustomobject]$Pins = (Get-OmaPresentMonPins),
        [scriptblock]$SignatureProvider = { param($Path) Get-AuthenticodeSignature -LiteralPath $Path }
    )
    $hash = (Get-OmaSha256 $Path).ToUpperInvariant()
    if ($hash -cne $Pins.Sha256.ToUpperInvariant()) { return "SHA-256 $hash, expected $($Pins.Sha256)" }
    $sig = & $SignatureProvider $Path
    if ("$($sig.Status)" -cne 'Valid') { return "Authenticode status $($sig.Status): $($sig.StatusMessage)" }
    $cert = $sig.SignerCertificate
    $subject = if ($cert) { "$($cert.Subject)" } else { '' }
    if ($subject -cne $Pins.SignerSubject) { return "signer '$subject', expected '$($Pins.SignerSubject)'" }
    $null
}

<#
.SYNOPSIS
  Stages the PresentMon console at -Destination: reuses a cached copy that passes the pins,
  otherwise downloads (https URL) or copies (local file) -Source to "<Destination>.partial",
  verifies it and moves it into place. Returns $true when the cached copy was reused, $false when
  it was fetched; throws "PresentMon-2.6.0-x64.exe rejected: <problem>" and leaves nothing
  behind when the fetched file fails the pins. A cached copy that fails them is removed first.
.PARAMETER SignatureProvider
  param($Path) -> an object shaped like Get-AuthenticodeSignature's result. Tests inject a fake.
#>
function Save-OmaPresentMon {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$Source,
        [Parameter(Mandatory)] [string]$Destination,
        [pscustomobject]$Pins = (Get-OmaPresentMonPins),
        [scriptblock]$SignatureProvider = { param($Path) Get-AuthenticodeSignature -LiteralPath $Path }
    )
    if (Test-Path -LiteralPath $Destination -PathType Leaf) {
        $problem = Test-OmaPresentMonExe -Path $Destination -Pins $Pins -SignatureProvider $SignatureProvider
        if ($null -eq $problem) {
            Write-Host "${script:FileName}: cached copy verified"
            return $true
        }
        Write-Host "${script:FileName}: cached copy rejected ($problem), fetching again"
        Remove-Item -LiteralPath $Destination -Force
    }
    New-Item -ItemType Directory -Force (Split-Path -Parent $Destination) | Out-Null
    $partial = "$Destination.partial"
    try {
        if ($Source -match '^https://') {
            Write-Host "Downloading $Source"
            Invoke-WebRequest -Uri $Source -OutFile $partial
        } else {
            Write-Host "Copying $Source"
            Copy-Item -LiteralPath $Source -Destination $partial -Force
        }
        $problem = Test-OmaPresentMonExe -Path $partial -Pins $Pins -SignatureProvider $SignatureProvider
        if ($null -ne $problem) { throw "$script:FileName rejected: $problem" }
        Move-Item -LiteralPath $partial -Destination $Destination -Force
    } finally {
        if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Force }
    }
    Write-Host "${script:FileName}: fetched and verified"
    $false
}

Export-ModuleMember -Function Get-OmaPresentMonPins, Test-OmaPresentMonExe, Save-OmaPresentMon
