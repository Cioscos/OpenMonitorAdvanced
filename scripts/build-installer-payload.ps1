<#
.SYNOPSIS
  Builds the payload the NSIS installer embeds (spec §10): the published oma-service.exe, the
  overlay process oma-overlay.exe (M7c), the official PawnIO 2.2.0 setup and the official
  PresentMon 2.6.0 console, in target/installer-payload/.
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
  4. Checks that the publish output holds nothing but oma-service.exe (and its .pdb), since the
     installer copies only the exe, and that oma-service.exe exists and carries the expected
     metadata: ProductName "OpenMonitor Advanced", ProductVersion X.Y.Z, FileVersion X.Y.Z.0.
  4b. `cargo build --release --locked -p oma-overlay` and stages target/release/oma-overlay.exe as
     overlay/oma-overlay.exe (plan M7c DP7; oma.nsh installs it as $INSTDIR\oma-overlay.exe),
     after removing the previous copy, with the same metadata check as step 4 (FileVersion X.Y.Z,
     like the app). The logic lives in scripts/lib/OmaOverlayPayload.psm1 (Save-OmaOverlayExe).
  5. Downloads PawnIO_setup.exe 2.2.0 (or reuses the cached copy) and verifies its SHA-256
     against the pinned hash (app/src-tauri/nsis/pawnio.sha256, the single source also read by
     oma.nsh at compile time) and its Authenticode signature (Valid, pinned signer). The pins
     live in scripts/lib/OmaPawnIoPins.psm1 and are never updated automatically: a mismatch
     fails the build and removes the file.
  6. Downloads PresentMon-2.6.0-x64.exe (or reuses the cached copy) into presentmon/ and
     verifies its SHA-256 against app/src-tauri/nsis/presentmon.sha256 (also read by oma.nsh at
     compile time and by the service) and its Authenticode signature (Valid, signed by Intel).
     The pins live in scripts/lib/OmaPresentMonPins.psm1, which also holds the staging logic
     (Save-OmaPresentMon); a mismatch fails the build and leaves no file. PresentMon is never run.
  Exit code 0 only when all of the above succeeded.
.PARAMETER DotnetExe
  The dotnet command. Tests inject a fake here.
.PARAMETER ServiceProject
  The service project directory. Tests point it at a temp directory.
.PARAMETER OutputRoot
  The payload directory. Defaults to target/installer-payload, where oma.nsh looks for it.
.PARAMETER CargoExe
  The cargo command. Tests inject a fake here.
.PARAMETER CargoTargetDir
  Cargo's target directory: CARGO_TARGET_DIR when set, else target/ in the repository.
.PARAMETER OverlayOnly
  Only step 4b: leaves the service payload, PawnIO and PresentMon alone. For tests.
.PARAMETER PawnIoOnly
  Only step 5: leaves the service payload, the overlay and PresentMon alone. For tests.
.PARAMETER PawnIoSource
  Where to fetch PawnIO_setup.exe from: the official URL (default) or a local file (tests).
  Whatever the source, the file must match the pinned hash and signer.
.PARAMETER PresentMonOnly
  Only step 6: leaves the service payload, the overlay and PawnIO alone. For tests.
.PARAMETER PresentMonSource
  Where to fetch PresentMon-2.6.0-x64.exe from: the official v2.6.0 release URL (default) or a
  local file (tests). Whatever the source, the file must match the pinned hash and signer.
#>
#Requires -Version 7

param(
    [string]$DotnetExe = 'dotnet',
    [string]$ServiceProject,
    [string]$OutputRoot,
    [string]$CargoExe = 'cargo',
    [string]$CargoTargetDir,
    [switch]$OverlayOnly,
    [switch]$PawnIoOnly,
    [string]$PawnIoSource = 'https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe',
    [switch]$PresentMonOnly,
    [string]$PresentMonSource = 'https://github.com/GameTechDev/PresentMon/releases/download/v2.6.0/PresentMon-2.6.0-x64.exe'
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $ServiceProject) { $ServiceProject = Join-Path $repoRoot 'service\OpenMonitorAdvanced.Service' }
if (-not $OutputRoot) { $OutputRoot = Join-Path $repoRoot 'target\installer-payload' }
if (-not $CargoTargetDir) { $CargoTargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repoRoot 'target' } }

# The PawnIO pins (hash from app/src-tauri/nsis/pawnio.sha256, pinned signer, provenance) live in
# scripts/lib/OmaPawnIoPins.psm1, shared with scripts/verify-signatures.ps1. Never update them
# automatically. The service metadata rule is Test-OmaVersionInfo, shared with the verifier too.
Import-Module (Join-Path $PSScriptRoot 'lib\OmaPawnIoPins.psm1') -Force
# Same for PresentMon (scripts/lib/OmaPresentMonPins.psm1, pinned hash in presentmon.sha256).
Import-Module (Join-Path $PSScriptRoot 'lib\OmaPresentMonPins.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'lib\OmaSigning.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'lib\OmaOverlayPayload.psm1') -Force
$PawnIoPins = Get-OmaPawnIoPins -RepoRoot $repoRoot

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

    # --- 3b. nothing but the exe and its symbols ---------------------------------------------------
    # The installer copies only oma-service.exe: anything else the single file might need next to
    # it (a native DLL, a config file) would be missing from the installed service.
    $unexpected = @(Get-ChildItem -LiteralPath $serviceOut -Force -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -notin @('oma-service.exe', 'oma-service.pdb') } |
            ForEach-Object Name)
    if ($unexpected.Count -gt 0) {
        Remove-Item -Recurse -Force $serviceOut -ErrorAction SilentlyContinue
        Fail "unexpected files in the publish output: $($unexpected -join ', ') (the installer copies only oma-service.exe)"
    }

    # --- 4. the exe we just built -----------------------------------------------------------------
    if (-not (Test-Path -PathType Leaf $serviceExe)) { Fail "oma-service.exe is missing from $serviceOut after publish" }
    $exeInfo = Get-Item $serviceExe
    if ($exeInfo.LastWriteTime -lt $started.AddSeconds(-5)) {
        Fail "oma-service.exe is older than this build ($($exeInfo.LastWriteTime))"
    }
    # ProductName 'OpenMonitor Advanced', ProductVersion X.Y.Z, FileVersion X.Y.Z.0 (the formats the
    # signed-file metadata of SignPath and verify-signatures.ps1 expect).
    $metadataProblems = @(Test-OmaVersionInfo -VersionInfo $exeInfo.VersionInfo -Version $appVersion -Name 'oma-service.exe')
    if ($metadataProblems.Count -gt 0) { Fail ($metadataProblems -join '; ') }
    Write-Host ("oma-service.exe {0}, {1:n1} MB" -f $exeInfo.VersionInfo.FileVersion, ($exeInfo.Length / 1MB))
}

New-Item -ItemType Directory -Force $OutputRoot | Out-Null
$only = @(@{ Overlay = $OverlayOnly; PawnIO = $PawnIoOnly; PresentMon = $PresentMonOnly }.GetEnumerator() |
        Where-Object { $_.Value } | ForEach-Object Key)
if ($only.Count -gt 1) { Fail '-OverlayOnly, -PawnIoOnly and -PresentMonOnly exclude each other' }
$all = $only.Count -eq 0
if ($all) {
    Build-ServicePayload
} else {
    Write-Host "$($only[0]) only: the other parts of the payload are left as they are"
}

# --- 4b. overlay process ----------------------------------------------------------------------
if ($all -or $OverlayOnly) {
    $appVersion = (Get-Content -Raw (Join-Path $repoRoot 'app\src-tauri\tauri.conf.json') | ConvertFrom-Json).version
    try {
        $overlay = Save-OmaOverlayExe -CargoExe $CargoExe -RepoRoot $repoRoot -TargetDir $CargoTargetDir `
            -Destination (Join-Path $OutputRoot 'overlay\oma-overlay.exe') -Version $appVersion
    } catch {
        Fail $_.Exception.Message
    }
    Write-Host ("oma-overlay.exe {0}, {1:n1} MB" -f $overlay.FileVersion, ((Get-Item -LiteralPath $overlay.Path).Length / 1MB))
}

# --- 5. PawnIO setup --------------------------------------------------------------------------
if ($all -or $PawnIoOnly) {
    $pawnIo = Join-Path $OutputRoot 'PawnIO_setup.exe'
    $reused = $false
    if (Test-Path -PathType Leaf $pawnIo) {
        $problem = Test-OmaPawnIoSetup -Path $pawnIo -Pins $PawnIoPins
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
        $problem = Test-OmaPawnIoSetup -Path $partial -Pins $PawnIoPins
        if ($null -ne $problem) {
            Remove-Item -Force $partial
            Fail "downloaded PawnIO_setup.exe rejected: $problem"
        }
        Move-Item -Force $partial $pawnIo
        Write-Host 'PawnIO_setup.exe: fetched and verified'
    }
}

# --- 6. PresentMon console --------------------------------------------------------------------
# Where oma.nsh takes it from; installed as $INSTDIR\service\presentmon\PresentMon-2.6.0-x64.exe.
if ($all -or $PresentMonOnly) {
    try {
        $null = Save-OmaPresentMon -Source $PresentMonSource -Destination (Join-Path $OutputRoot 'presentmon\PresentMon-2.6.0-x64.exe')
    } catch {
        Fail $_.Exception.Message
    }
}

Write-Host ''
Write-Host "OK: installer payload ready in $OutputRoot" -ForegroundColor Green
exit 0
