<#
.SYNOPSIS
  Fails if `oma-service`'s trimmed publish produces a trim (ILLink) warning whose exact
  origin is not on the reviewed allowlist (service/trim-allowlist.txt).
.DESCRIPTION
  The Service.pubxml's WarningsNotAsErrors only demotes a handful of IL codes from build
  errors back to plain warnings, so MSBuild does not itself abort the publish (controller
  ruling R16 rejected that as the sole gate: it works at the IL-code level, so a *new*
  warning at an unlisted call site but with an already-demoted code would silently pass).
  This script is the real gate: it runs a clean publish, parses every ILxxxx warning out of
  the log as a (code, origin) pair, and compares each one against
  service/trim-allowlist.txt (one `CODE|origin` per line; '#' comments and blank lines
  ignored). Any warning whose pair is not listed fails the script (exit 1) and is printed.
  Allowlist entries that did not occur in this run are reported as informational only
  (not a failure) — the allowlist does not have to be reproduced by every machine/run.
  A failed `dotnet publish` (non-zero $LASTEXITCODE) also fails the script.

  Real dotnet/ILLink output has more than one shape (see
  scripts/testdata/trim-warnings-sample*.log, captured from the S1 spike):
    - `ILLink : Trim analysis warning ILxxxx: <origin>: <message> [proj.csproj]`
      (library/dependency code with no source available — what oma-service's own
      publish actually produces for its accepted warnings today).
    - `<path>(line,col): Trim analysis warning ILxxxx: <origin>: <message> [proj.csproj]`
      (source-mapped project code).
    - `<path>(line,col): warning ILxxxx: <message, no origin at all>` — the Roslyn
      compile-time trim analyzer's own diagnostic for the exact same call site, with NO
      origin member printed (the IDE attributes it by file/line instead). A line like this
      is treated as *unparseable*, never guessed at, and therefore always fails (fail
      closed) — see -ParseOnly below to exercise this against a real captured line.
  A warning line's origin, and every allowlist origin, is rejected outright (fail closed,
  with a clear message) if it contains a literal `|`, since that would make the `CODE|origin`
  split ambiguous.
.PARAMETER AllowlistPath
  The allowlist file. Defaults to service/trim-allowlist.txt next to this script's repo root.
.PARAMETER PublishOutputDir
  Optional -p:PublishDir override, so the check can run without touching the normal
  bin\Publish output (e.g. a temp directory in CI).
.PARAMETER InputLog
  Parse this existing publish log instead of running `dotnet publish` again. Combine with
  -ParseOnly for a pure parsing/comparison run — used both to validate the parser against
  real captured spike output (scripts/testdata/trim-warnings-sample.log) and to demonstrate
  a failure without a real unlisted warning at hand
  (scripts/testdata/trim-warnings-sample-unlisted.log).
.PARAMETER ParseOnly
  Skip running `dotnet publish` entirely; requires -InputLog.
.EXAMPLE
  ./scripts/check-trim-warnings.ps1
.EXAMPLE
  ./scripts/check-trim-warnings.ps1 -ParseOnly -InputLog scripts/testdata/trim-warnings-sample.log
.EXAMPLE
  ./scripts/check-trim-warnings.ps1 -ParseOnly -InputLog scripts/testdata/trim-warnings-sample-unlisted.log
#>
#Requires -Version 7

param(
    [string]$AllowlistPath = (Join-Path $PSScriptRoot '..\service\trim-allowlist.txt'),
    [string]$PublishOutputDir,
    [string]$InputLog,
    [switch]$ParseOnly
)

$ErrorActionPreference = 'Stop'
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')

<#
.SYNOPSIS
  Reads `CODE|origin` allowlist entries, fail-closed on anything ambiguous.
.DESCRIPTION
  Every non-comment, non-blank line must split into exactly two `|`-separated fields
  (code, origin); an origin containing a second `|` is rejected rather than guessed at,
  since it would make the split ambiguous with how warnings are matched.
#>
function Read-Allowlist([string]$Path) {
    $entries = [System.Collections.Generic.HashSet[string]]::new()
    $lineNumber = 0
    foreach ($line in Get-Content -Path $Path) {
        $lineNumber++
        $trimmed = $line.Trim()
        if ($trimmed -eq '' -or $trimmed.StartsWith('#')) { continue }

        $parts = $trimmed.Split('|')
        if ($parts.Count -ne 2) {
            throw "Refusing to load '$Path': line $lineNumber ('$trimmed') must be exactly one 'CODE|origin' pair (found $($parts.Count) '|'-separated field(s)). An origin must never contain a literal '|'."
        }

        [void]$entries.Add($trimmed)
    }

    return $entries
}

<#
.SYNOPSIS
  Parses ILxxxx trim warnings out of a `dotnet publish` log's lines.
.DESCRIPTION
  Real ILLink/MSBuild output always contains the literal text "warning IL" followed by 4
  digits; every such occurrence is treated as a trim warning that must be accounted for.
  The origin, when present, is a member signature immediately after "warning ILxxxx: "
  (or "... Trim analysis warning ILxxxx: "), of the form `Type.Member(Args): ` — matched
  non-greedily up to the first `): ` so nested generic argument lists (`Dictionary<String,
  List<Double>>`) and array brackets (`Byte[]`) inside the parameter list are not mistaken
  for the end of the signature. When that shape is not found immediately after the code —
  notably the Roslyn compile-time trim analyzer's own warnings, which print no origin member
  at all — the warning is reported with `Origin = $null` (Parsed = $false): the caller must
  treat that as unlisted, not attempt a best-effort guess.
.OUTPUTS
  One [pscustomobject]@{ Code; Origin; Parsed; Line } per warning found, in order
  (duplicates kept: the same (code, origin) pair can legitimately appear more than once,
  e.g. two call sites in the same method).
#>
function Get-TrimWarnings([string[]]$LogLines) {
    $originPattern = '(?:Trim analysis )?warning\s+(IL[23]\d{3}):\s*([\w.<>]+\([^)]*\)):\s'
    $anyWarningPattern = 'warning\s+IL[23]\d{3}\b'

    $results = [System.Collections.Generic.List[pscustomobject]]::new()
    foreach ($line in $LogLines) {
        $withOrigin = [regex]::Match($line, $originPattern)
        if ($withOrigin.Success) {
            $results.Add([pscustomobject]@{
                Code   = $withOrigin.Groups[1].Value
                Origin = $withOrigin.Groups[2].Value.Trim()
                Parsed = $true
                Line   = $line.Trim()
            })
            continue
        }

        $anyWarning = [regex]::Match($line, $anyWarningPattern)
        if ($anyWarning.Success) {
            # A trim warning is present but no origin member could be extracted immediately
            # after it (e.g. the Roslyn compile-time analyzer's origin-less shape). Fail
            # closed: report it as an unparsed warning rather than silently drop it.
            $code = [regex]::Match($line, 'IL[23]\d{3}').Value
            $results.Add([pscustomobject]@{
                Code   = $code
                Origin = $null
                Parsed = $false
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
    $serviceProject = Join-Path $repoRoot 'service\OpenMonitorAdvanced.Service'

    # A stale incremental publish can under-report: the linked-assembly analysis only reruns
    # when ILLink's own up-to-date check decides it must, so a previously-clean obj/bin can
    # hide a newly introduced warning. Force a real trim analysis every time this check runs.
    foreach ($stale in @((Join-Path $serviceProject 'obj'), (Join-Path $serviceProject 'bin'))) {
        if (Test-Path $stale) {
            Write-Host "Removing stale $stale before publishing"
            Remove-Item -Recurse -Force $stale
        }
    }

    $publishArgs = @(
        'publish', $serviceProject,
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
    if (-not $warning.Parsed) {
        $violations.Add($warning)
        continue
    }

    if ($warning.Origin.Contains('|')) {
        # Fail closed rather than build an ambiguous "CODE|origin" key.
        $violations.Add([pscustomobject]@{
            Code   = $warning.Code
            Origin = $warning.Origin
            Parsed = $false
            Line   = "$($warning.Line)  [rejected: origin contains a literal '|']"
        })
        continue
    }

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
        $label = if ($violation.Parsed) { "$($violation.Code)|$($violation.Origin)" } else { "$($violation.Code)|<unparseable origin>" }
        Write-Host "  $label" -ForegroundColor Red
        Write-Host "    $($violation.Line)"
    }

    exit 1
}

Write-Host ''
Write-Host 'OK: every trim warning is on the allowlist.' -ForegroundColor Green
exit 0
