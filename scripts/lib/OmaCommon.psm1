#Requires -Version 7
# Helpers shared by the release scripts (plan M6a, L1): native commands with separate
# stdout/stderr/exit code, SHA-256 and absolute path normalisation. No state, no Windows APIs
# beyond what .NET offers.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

<#
.SYNOPSIS
  Runs a native command and returns @{ Stdout; Stderr; ExitCode } without mixing the streams.
.DESCRIPTION
  Arguments are passed through ProcessStartInfo.ArgumentList, so each element reaches the
  program as one argument whatever spaces or quotes it contains. A nonzero exit code throws,
  with the stderr text in the message, unless -AllowFailure is given: callers that treat a
  nonzero code as an answer (git predicates, API lookups that may 404) check .ExitCode
  themselves. The working directory defaults to the current PowerShell location.
#>
function Invoke-OmaNative {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$FilePath,
        [string[]]$ArgumentList = @(),
        [string]$WorkingDirectory,
        [switch]$AllowFailure
    )
    $exe = $FilePath
    if (-not [IO.Path]::IsPathFullyQualified($FilePath)) {
        $cmd = Get-Command -Name $FilePath -CommandType Application -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if (-not $cmd) { throw "command not found: $FilePath" }
        $exe = $cmd.Source
    }
    if (-not $WorkingDirectory) { $WorkingDirectory = (Get-Location -PSProvider FileSystem).ProviderPath }

    $psi = [Diagnostics.ProcessStartInfo]::new($exe)
    foreach ($a in $ArgumentList) { $psi.ArgumentList.Add($a) }
    $psi.WorkingDirectory = $WorkingDirectory
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardInput = $true
    $psi.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $psi.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $psi.CreateNoWindow = $true

    $p = [Diagnostics.Process]::Start($psi)
    try {
        $p.StandardInput.Close()
        # Both streams are drained concurrently, so a chatty stderr cannot block the child.
        $out = $p.StandardOutput.ReadToEndAsync()
        $err = $p.StandardError.ReadToEndAsync()
        $p.WaitForExit()
        $result = [pscustomobject]@{
            Stdout   = $out.GetAwaiter().GetResult()
            Stderr   = $err.GetAwaiter().GetResult()
            ExitCode = $p.ExitCode
        }
    } finally {
        $p.Dispose()
    }
    if ($result.ExitCode -ne 0 -and -not $AllowFailure) {
        throw "$FilePath exited with code $($result.ExitCode): $($result.Stderr.Trim())"
    }
    $result
}

<#
.SYNOPSIS
  SHA-256 of a file as a lowercase hex string.
#>
function Get-OmaSha256 {
    [CmdletBinding()]
    param([Parameter(Mandatory, Position = 0)] [string]$Path)
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)).ToLowerInvariant()
    } finally {
        $stream.Dispose()
    }
}

<#
.SYNOPSIS
  Normalises an absolute path with [IO.Path]::GetFullPath, without a trailing separator.
.DESCRIPTION
  Mixed separators and `.`/`..` segments are resolved; a root (`C:\`, `\\server\share\`) keeps
  its separator. Relative paths, including drive-relative (`C:x`) and rooted-without-drive
  (`\x`) ones, are refused: they would depend on the current directory, which differs between
  the callers (app\src-tauri for Tauri, target\release\nsis\x64 for makensis).
#>
function Resolve-OmaPath {
    [CmdletBinding()]
    param([Parameter(Mandatory, Position = 0)] [string]$Path)
    if (-not [IO.Path]::IsPathFullyQualified($Path)) { throw "path must be absolute: $Path" }
    $full = [IO.Path]::GetFullPath($Path)
    $root = [IO.Path]::GetPathRoot($full)
    if ($full.Length -gt $root.Length) { $full = $full.TrimEnd('\', '/') }
    $full
}

Export-ModuleMember -Function Invoke-OmaNative, Get-OmaSha256, Resolve-OmaPath
