<#
.SYNOPSIS
  Measures OpenMonitor Advanced against the performance budget (spec §1.2).
.DESCRIPTION
  Starts the release build, waits for warm-up, then reports the app's CPU
  usage and the private working set (Task Manager "Memory" column) of the app
  and of its WebView2 child processes.
.EXAMPLE
  ./scripts/measure-footprint.ps1              # window open
  ./scripts/measure-footprint.ps1 -Minimized   # tray only
#>
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\oma-app.exe'),
    [int]$WarmupSeconds = 15,
    [int]$SampleSeconds = 30,
    [switch]$Minimized
)

$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path $Exe).Path
$proc = if ($Minimized) {
    Start-Process -FilePath $exePath -ArgumentList '--minimized' -WindowStyle Hidden -PassThru
} else {
    # Controller ruling: a hidden window would likely stop WebView rendering,
    # invalidating the "window open" measurement, so window mode launches
    # normally. -WindowStyle Hidden is kept only for -Minimized.
    Start-Process -FilePath $exePath -WindowStyle Normal -PassThru
}

try {
    Start-Sleep -Seconds $WarmupSeconds
    $proc.Refresh()
    if ($proc.HasExited) { throw "The measured instance exited (another instance may already be running)." }
    $cpuStart = $proc.TotalProcessorTime
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds $SampleSeconds
    $proc.Refresh()
    $cpuEnd = $proc.TotalProcessorTime
    $cpuPercent = ($cpuEnd - $cpuStart).TotalMilliseconds / $elapsed.Elapsed.TotalMilliseconds / [Environment]::ProcessorCount * 100

    # Follow the actual process tree, including renderer grandchildren.
    $processes = @(Get-CimInstance Win32_Process)
    $descendants = [Collections.Generic.HashSet[int]]::new()
    [void]$descendants.Add($proc.Id)
    do {
        $changed = $false
        foreach ($child in $processes) {
            if ($descendants.Contains([int]$child.ParentProcessId)) {
                if ($descendants.Add([int]$child.ProcessId)) { $changed = $true }
            }
        }
    } while ($changed)
    $webviews = @($processes | Where-Object {
        $_.Name -eq 'msedgewebview2.exe' -and $descendants.Contains([int]$_.ProcessId)
    })
    $ids = @($proc.Id) + @($webviews | ForEach-Object { [int]$_.ProcessId })
    $perf = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process |
        Where-Object { $ids -contains [int]$_.IDProcess })
    if ($perf.Count -ne $ids.Count) { throw "Missing process memory counters; measurement is invalid." }
    $appPrivate = ($perf | Where-Object { [int]$_.IDProcess -eq $proc.Id }).WorkingSetPrivate
    $totalPrivate = ($perf | Measure-Object -Property WorkingSetPrivate -Sum).Sum

    [pscustomobject]@{
        Mode              = if ($Minimized) { 'tray' } else { 'window' }
        CorePercentCpu    = [math]::Round($cpuPercent, 2)
        AppPrivateMB      = [math]::Round($appPrivate / 1MB, 1)
        WebView2Processes = $webviews.Count
        TotalPrivateMB    = [math]::Round($totalPrivate / 1MB, 1)
    } | Format-List
}
finally {
    Stop-Process -Id $proc.Id -ErrorAction SilentlyContinue
}
