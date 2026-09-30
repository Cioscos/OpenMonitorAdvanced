#Requires -Version 7
# The pins of the official PawnIO setup the installer embeds, shared by the payload build
# (scripts/build-installer-payload.ps1) and the release verifier (scripts/verify-signatures.ps1),
# so both check the same hash and the same author. Importing this module runs nothing else.
#
# PawnIO 2.2.0, official setup. Provenance (recorded 2026-09-26 by the M4 Task 13 implementer):
# - URL: https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe
#   (release "Release 2.2.0", published 2026-03-15T15:49:08Z, 3 410 960 bytes);
# - SHA-256 (pinned in app/src-tauri/nsis/pawnio.sha256, also read by oma.nsh at compile time)
#   = the `digest` GitHub's release API reports for that asset;
# - Authenticode: Valid, signer "E=admin@namazso.eu, CN=namazso.eu, O=namazso, L=Debrecen,
#   C=HU", issued by "GLOBALTRUST 2015 CODESIGNING 1" (e-commerce monitoring GmbH), valid
#   2024-08-02..2027-08-05, timestamped by the Microsoft Public RSA Time Stamping Authority;
# - version info: PawnIO Setup 2.2.0.0, company namazso.
# Never update these automatically. Moving to another PawnIO release means repeating the checks
# above by hand and recording them here.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'OmaCommon.psm1')

$script:SignerSubject = 'E=admin@namazso.eu, CN=namazso.eu, O=namazso, L=Debrecen, C=HU'
$script:SignerThumbprint = 'F380DCC9F706E2756A5047B832FFE719E1BC35F5'

<#
.SYNOPSIS
  The PawnIO pins: @{ Sha256 (upper-case hex, from pawnio.sha256); SignerSubject; SignerThumbprint }.
#>
function Get-OmaPawnIoPins {
    [CmdletBinding()]
    param([string]$RepoRoot = (Join-Path $PSScriptRoot '..\..'))
    $file = Join-Path $RepoRoot 'app\src-tauri\nsis\pawnio.sha256'
    $sha = (Get-Content -Raw -LiteralPath $file).Trim()
    if ($sha -cnotmatch '^[0-9A-F]{64}$') { throw 'app/src-tauri/nsis/pawnio.sha256 must hold one upper-case SHA-256' }
    [pscustomobject]@{ Sha256 = $sha; SignerSubject = $script:SignerSubject; SignerThumbprint = $script:SignerThumbprint }
}

<#
.SYNOPSIS
  Checks a PawnIO setup against the pins: SHA-256, Authenticode status Valid and the pinned signer
  (thumbprint and subject). Returns the first problem as a string, or $null.
.PARAMETER SignatureProvider
  param($Path) -> an object shaped like Get-AuthenticodeSignature's result. Tests inject a fake.
#>
function Test-OmaPawnIoSetup {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$Path,
        [pscustomobject]$Pins = (Get-OmaPawnIoPins),
        [scriptblock]$SignatureProvider = { param($Path) Get-AuthenticodeSignature -LiteralPath $Path }
    )
    $hash = (Get-OmaSha256 $Path).ToUpperInvariant()
    if ($hash -cne $Pins.Sha256.ToUpperInvariant()) { return "SHA-256 $hash, expected $($Pins.Sha256)" }
    $sig = & $SignatureProvider $Path
    if ("$($sig.Status)" -cne 'Valid') { return "Authenticode status $($sig.Status): $($sig.StatusMessage)" }
    $cert = $sig.SignerCertificate
    if ($null -eq $cert -or "$($cert.Thumbprint)".ToUpperInvariant() -cne $Pins.SignerThumbprint) {
        return "signer thumbprint $(if ($cert) { $cert.Thumbprint }), expected $($Pins.SignerThumbprint)"
    }
    if ($cert.Subject -cne $Pins.SignerSubject) {
        return "signer '$($cert.Subject)', expected '$($Pins.SignerSubject)'"
    }
    $null
}

Export-ModuleMember -Function Get-OmaPawnIoPins, Test-OmaPawnIoSetup
