<#
.SYNOPSIS
  Fails if `oma-service`'s trimmed publish produces a trim (ILLink) warning whose exact
  origin is not on the reviewed allowlist (service/trim-allowlist.txt).
.DESCRIPTION
  The Service.pubxml's WarningsNotAsErrors only demotes a handful of IL codes from build
  errors back to plain warnings, so MSBuild does not itself abort the publish (controller
  ruling R16 rejected that as the sole gate: it works at the IL-code level, so a *new*
  warning at an unlisted call site but with an already-demoted code would silently pass).
  This script is the real gate: it runs the publish, parses every ILxxxx warning out of the
  log as a (code, origin) pair, and compares each one against
  service/trim-allowlist.txt (one `CODE|origin` per line; '#' comments and blank lines
  ignored). Any warning whose pair is not listed fails the script (exit 1) and is printed.
  Allowlist entries that did not occur in this run are reported as informational only
  (not a failure) — the allowlist does not have to be reproduced by every machine/run.
  A failed `dotnet publish` (non-zero $LASTEXITCODE) also fails the script.
.PARAMETER AllowlistPath
  The allowlist file. Defaults to service/trim-allowlist.txt next to this script's repo root.
.PARAMETER PublishOutputDir
  Optional -p:PublishDir override, so the check can run without touching the normal
  bin\Publish output (e.g. a temp directory in CI).
.PARAMETER InputLog
  Parse this existing publish log instead of running `dotnet publish` again. Combine with
  -ParseOnly for a pure parsing/comparison run (used to unit-test the parser itself with a
  hand-written sample log, since a clean publish on this machine currently produces zero
  trim warnings and can't otherwise demonstrate a failing run).
.PARAMETER ParseOnly
  Skip running `dotnet publish` entirely; requires -InputLog.
.EXAMPLE
  ./scripts/check-trim-warnings.ps1
.EXAMPLE
  # Demonstrate a failure without a real unlisted warning at hand:
  ./scripts/check-trim-warnings.ps1 -ParseOnly -InputLog ./sample-with-new-warning.log
#>
param(
    [string]$AllowlistPath = (Join-Path $PSScriptRoot '..\service\trim-allowlist.txt'),
    [string]$PublishOutputDir,
    [string]$InputLog,
    [switch]$ParseOnly
)

$ErrorActionPreference = 'Stop'
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')

function Read-Allowlist([string]$Path) {
    $entries = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($line in Get-Content -Path $Path) {
        $trimmed = $line.Trim()
        if ($trimmed -eq '' -or $trimmed.StartsWith('#')) { continue }
        [void]$entries.Add($trimmed)
    }
    return $entries
}

<#
.SYNOPSIS
  Parses ILxxxx trim warnings out of a `dotnet publish` log's lines.
.OUTPUTS
  One [pscustomobject]@{ Code; Origin; Line } per warning found, in order (duplicates kept:
  the same (code, origin) pair can legitimately appear more than once, e.g. two call sites
  in the same method).
#>
function Get-TrimWarnings([string[]]$LogLines) {
    # MSBuild/ILLink prints e.g.:
    #   obj\...\linked\Link.semaphore : warning IL2075: LibreHardwareMonitor.Hardware.OpCode.Open(): ...
    # or, from csc-style diagnostics:
    #   Program.cs(12,3): warning IL2026: Program.Main(String[]): ...
    # In both cases the shape after "warning ILxxxx:" is "<origin>: <message>", so the origin
    # is everything up to the next ": " (origins here are member signatures, which do not
    # themselves contain ": ").
    $pattern = 'warning\s+(IL[23]\d{3}):\s*([^:]+(?:<[^>]*>)?[^:]*?)\s*:\s'
    $results = [System.Collections.Generic.List[pscustomobject]]::new()
    foreach ($line in $LogLines) {
        $match = [regex]::Match($line, $pattern)
        if ($match.Success) {
            $results.Add([pscustomobject]@{
                Code   = $match.Groups[1].Value
                Origin = $match.Groups[2].Value.Trim()
                Line   = $line.Trim()
            })
        }
    }
    return $results
}

if ($ParseOnly -and -not $InputLog) {
    throw '-ParseOnly requires -InputLog.'
}

if ($InputLog) {
    $logLines = Get-Content -Path $InputLog
} elseif (-not $ParseOnly) {
    $publishArgs = @(
        'publish', (Join-Path $repoRoot 'service\OpenMonitorAdvanced.Service'),
        '-c', 'Release',
        '-p:PublishProfile=Service',
        '-v', 'normal'
    )
    if ($PublishOutputDir) {
        $publishArgs += "-p:PublishDir=$PublishOutputDir"
    }

    Write-Host "Running: dotnet $($publishArgs -join ' ')"
    $logLines = & dotnet @publishArgs 2>&1 | ForEach-Object { $_.ToString() }
    $publishExitCode = $LASTEXITCODE
    $logLines | ForEach-Object { Write-Host $_ }

    if ($publishExitCode -ne 0) {
        Write-Error "dotnet publish failed with exit code $publishExitCode."
        exit 1
    }
} else {
    throw 'Either -InputLog or a real publish run is required.'
}

$allowlist = Read-Allowlist -Path $AllowlistPath
$warnings = Get-TrimWarnings -LogLines $logLines

$seenAllowlistKeys = [System.Collections.Generic.HashSet[string]]::new()
$violations = [System.Collections.Generic.List[pscustomobject]]::new()

foreach ($warning in $warnings) {
    $key = "$($warning.Code)|$($warning.Origin)"
    if ($allowlist.Contains($key)) {
        [void]$seenAllowlistKeys.Add($key)
    } else {
        $violations.Add($warning)
    }
}

Write-Host ''
Write-Host "Trim warnings found: $($warnings.Count)"
Write-Host "Allowlist entries used: $($seenAllowlistKeys.Count) / $($allowlist.Count)"

$unused = $allowlist | Where-Object { -not $seenAllowlistKeys.Contains($_) }
if ($unused) {
    Write-Host ''
    Write-Host 'Informational: allowlist entries that did not occur in this run (not a failure):'
    foreach ($entry in $unused) { Write-Host "  $entry" }
}

if ($violations.Count -gt 0) {
    Write-Host ''
    Write-Host 'FAIL: trim warning(s) not on the allowlist (service/trim-allowlist.txt):' -ForegroundColor Red
    foreach ($violation in $violations) {
        Write-Host "  $($violation.Code)|$($violation.Origin)" -ForegroundColor Red
        Write-Host "    $($violation.Line)"
    }

    exit 1
}

Write-Host ''
Write-Host 'OK: every trim warning is on the allowlist.' -ForegroundColor Green
exit 0
