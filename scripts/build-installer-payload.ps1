<#
.SYNOPSIS
  Builds the payload the NSIS installer embeds (spec §10): the published oma-service.exe and
  the official PawnIO 2.2.0 setup, in target/installer-payload/.
.DESCRIPTION
  Run it before `pnpm tauri build` (app/src-tauri/nsis/oma.nsh refuses to compile without it):

    pwsh scripts/build-installer-payload.ps1
    cd app; pnpm tauri build --bundles nsis

  1. Deletes the previous service payload and the service's obj/bin, so a failed build can
     never leave a stale oma-service.exe for the installer to pick up, and so ILLink's trim
     analysis really reruns (same reason as scripts/check-trim-warnings.ps1).
  2. `dotnet publish service/OpenMonitorAdvanced.Service -c Release -p:PublishProfile=Service
     -o target/installer-payload/service`, stamped with the app version from
     app/src-tauri/tauri.conf.json. Fails on a non-zero exit code.
  3. Trim gate (controller ruling R16): the publish log goes through
     scripts/check-trim-warnings.ps1 -ParseOnly, so any trim warning whose (code, origin) is
     not in service/trim-allowlist.txt fails the payload build.
  4. Checks that oma-service.exe exists and carries the expected file version.
  5. Downloads PawnIO_setup.exe 2.2.0 (or reuses the cached copy) and verifies its SHA-256
     against the pinned hash (app/src-tauri/nsis/pawnio.sha256, the single source also read by
     oma.nsh at compile time) and its Authenticode signature (Valid, pinned signer). The pins
     are never updated automatically: a mismatch fails the build and removes the file.
  Exit code 0 only when all of the above succeeded.
.PARAMETER DotnetExe
  The dotnet command. Tests inject a fake here.
.PARAMETER ServiceProject
  The service project directory. Tests point it at a temp directory.
.PARAMETER OutputRoot
  The payload directory. Defaults to target/installer-payload, where oma.nsh looks for it.
.PARAMETER PawnIoOnly
  Only step 5: leaves the service payload alone. For tests.
.PARAMETER PawnIoSource
  Where to fetch PawnIO_setup.exe from: the official URL (default) or a local file (tests).
  Whatever the source, the file must match the pinned hash and signer.
#>
#Requires -Version 7

param(
    [string]$DotnetExe = 'dotnet',
    [string]$ServiceProject,
    [string]$OutputRoot,
    [switch]$PawnIoOnly,
    [string]$PawnIoSource = 'https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe'
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $ServiceProject) { $ServiceProject = Join-Path $repoRoot 'service\OpenMonitorAdvanced.Service' }
if (-not $OutputRoot) { $OutputRoot = Join-Path $repoRoot 'target\installer-payload' }

# PawnIO 2.2.0, official setup. Provenance (recorded 2026-09-26 by the Task 13 implementer):
# - URL: https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe
#   (release "Release 2.2.0", published 2026-03-15T15:49:08Z, 3 410 960 bytes);
# - SHA-256 (pinned in app/src-tauri/nsis/pawnio.sha256) = the `digest` GitHub's release API
#   reports for that asset;
# - Authenticode: Valid, signer "E=admin@namazso.eu, CN=namazso.eu, O=namazso, L=Debrecen,
#   C=HU", issued by "GLOBALTRUST 2015 CODESIGNING 1" (e-commerce monitoring GmbH), valid
#   2024-08-02..2027-08-05, timestamped by the Microsoft Public RSA Time Stamping Authority;
# - version info: PawnIO Setup 2.2.0.0, company namazso.
# Never update these automatically. Moving to another PawnIO release means repeating the
# checks above by hand and recording them here.
$PawnIoSha256 = (Get-Content -Raw (Join-Path $repoRoot 'app\src-tauri\nsis\pawnio.sha256')).Trim()
if ($PawnIoSha256 -cnotmatch '^[0-9A-F]{64}$') { throw "app/src-tauri/nsis/pawnio.sha256 must hold one upper-case SHA-256" }
$PawnIoSignerSubject = 'E=admin@namazso.eu, CN=namazso.eu, O=namazso, L=Debrecen, C=HU'
$PawnIoSignerThumbprint = 'F380DCC9F706E2756A5047B832FFE719E1BC35F5'

function Fail([string]$Message) {
    Write-Host "FAIL: $Message" -ForegroundColor Red
    exit 1
}

# Steps 1-4: the service exe. Fail exits the whole script.
function Build-ServicePayload {
    # --- 1. no stale payload, fresh trim analysis -------------------------------------------------
    $serviceOut = Join-Path $OutputRoot 'service'
    $serviceExe = Join-Path $serviceOut 'oma-service.exe'
    foreach ($stale in @($serviceOut, (Join-Path $ServiceProject 'obj'), (Join-Path $ServiceProject 'bin'))) {
        if (Test-Path $stale) {
            Write-Host "Removing $stale"
            Remove-Item -Recurse -Force $stale
        }
    }

    # --- 2. publish -------------------------------------------------------------------------------
    $appVersion = (Get-Content -Raw (Join-Path $repoRoot 'app\src-tauri\tauri.conf.json') | ConvertFrom-Json).version
    if ($appVersion -notmatch '^\d+\.\d+\.\d+$') { Fail "unexpected app version '$appVersion' in tauri.conf.json" }
    $expectedFileVersion = "$appVersion.0"

    $log = Join-Path $OutputRoot 'publish.log'
    $publishArgs = @(
        'publish', $ServiceProject,
        '-c', 'Release',
        '-p:PublishProfile=Service',
        "-p:Version=$appVersion",
        '-o', $serviceOut,
        '-v', 'normal'
    )
    Write-Host "Running: $DotnetExe $($publishArgs -join ' ')  (log: $log)"
    $started = Get-Date
    & $DotnetExe @publishArgs 2>&1 | ForEach-Object { $_.ToString() } | Set-Content -Path $log -Encoding utf8
    $publishExitCode = $LASTEXITCODE
    Write-Host ("dotnet publish finished in {0:n0} s" -f ((Get-Date) - $started).TotalSeconds)
    if ($publishExitCode -ne 0) {
        Get-Content $log -Tail 40 | ForEach-Object { Write-Host $_ }
        Remove-Item -Recurse -Force $serviceOut -ErrorAction SilentlyContinue
        Fail "dotnet publish failed with exit code $publishExitCode (full log: $log)"
    }

    # --- 3. per-origin trim gate (R16) ------------------------------------------------------------
    $pwsh = (Get-Process -Id $PID).Path
    & $pwsh -NoProfile -NonInteractive -File (Join-Path $PSScriptRoot 'check-trim-warnings.ps1') -ParseOnly -InputLog $log
    if ($LASTEXITCODE -ne 0) {
        Remove-Item -Recurse -Force $serviceOut -ErrorAction SilentlyContinue
        Fail 'trim warning gate failed (see above; the allowlist is service/trim-allowlist.txt)'
    }

    # --- 4. the exe we just built -----------------------------------------------------------------
    if (-not (Test-Path -PathType Leaf $serviceExe)) { Fail "oma-service.exe is missing from $serviceOut after publish" }
    $exeInfo = Get-Item $serviceExe
    if ($exeInfo.LastWriteTime -lt $started.AddSeconds(-5)) {
        Fail "oma-service.exe is older than this build ($($exeInfo.LastWriteTime))"
    }
    if ($exeInfo.VersionInfo.FileVersion -ne $expectedFileVersion) {
        Fail "oma-service.exe has file version '$($exeInfo.VersionInfo.FileVersion)', expected '$expectedFileVersion'"
    }
    Write-Host ("oma-service.exe {0}, {1:n1} MB" -f $exeInfo.VersionInfo.FileVersion, ($exeInfo.Length / 1MB))
}

New-Item -ItemType Directory -Force $OutputRoot | Out-Null
if ($PawnIoOnly) {
    Write-Host 'PawnIO only: the service payload is left as it is'
} else {
    Build-ServicePayload
}

# --- 5. PawnIO setup --------------------------------------------------------------------------
function Test-PawnIoSetup([string]$Path) {
    $hash = (Get-FileHash -Algorithm SHA256 $Path).Hash
    if ($hash -ne $PawnIoSha256) { return "SHA-256 $hash, expected $PawnIoSha256" }
    $sig = Get-AuthenticodeSignature $Path
    if ($sig.Status -ne 'Valid') { return "Authenticode status $($sig.Status): $($sig.StatusMessage)" }
    if ($sig.SignerCertificate.Thumbprint -ne $PawnIoSignerThumbprint) {
        return "signer thumbprint $($sig.SignerCertificate.Thumbprint), expected $PawnIoSignerThumbprint"
    }
    if ($sig.SignerCertificate.Subject -ne $PawnIoSignerSubject) {
        return "signer '$($sig.SignerCertificate.Subject)', expected '$PawnIoSignerSubject'"
    }
    return $null
}

$pawnIo = Join-Path $OutputRoot 'PawnIO_setup.exe'
$reused = $false
if (Test-Path -PathType Leaf $pawnIo) {
    $problem = Test-PawnIoSetup $pawnIo
    if ($null -eq $problem) {
        $reused = $true
        Write-Host 'PawnIO_setup.exe: cached copy verified'
    } else {
        Write-Host "PawnIO_setup.exe: cached copy rejected ($problem), downloading again"
        Remove-Item -Force $pawnIo
    }
}
if (-not $reused) {
    $partial = "$pawnIo.partial"
    if ($PawnIoSource -match '^https://') {
        Write-Host "Downloading $PawnIoSource"
        Invoke-WebRequest -Uri $PawnIoSource -OutFile $partial
    } else {
        Write-Host "Copying $PawnIoSource"
        Copy-Item -LiteralPath $PawnIoSource -Destination $partial
    }
    $problem = Test-PawnIoSetup $partial
    if ($null -ne $problem) {
        Remove-Item -Force $partial
        Fail "downloaded PawnIO_setup.exe rejected: $problem"
    }
    Move-Item -Force $partial $pawnIo
    Write-Host 'PawnIO_setup.exe: fetched and verified'
}

Write-Host ''
Write-Host "OK: installer payload ready in $OutputRoot" -ForegroundColor Green
exit 0
