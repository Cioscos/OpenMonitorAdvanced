<#
.SYNOPSIS
  Measures OpenMonitor Advanced against the performance budget (spec §1.2).
.DESCRIPTION
  Starts the release build, waits for warm-up, then reports the app's CPU
  usage and the private working set (Task Manager "Memory" column) of the app
  and of its WebView2 child processes. VendorModules lists the GPU vendor
  libraries loaded in the app, so a measurement taken in safe mode (or on a
  machine without a vendor driver) is recognisable.
  With -FillHistoryMinutes the app first runs in the tray for that long, so
  the one-hour history is full; the tray is measured, then (unless
  -Minimized) a second launch hands over to the running instance, which opens
  its window, and the window is measured. The window opens on the view saved
  in the WebView2 profile (see seed-advanced-view.ps1).
.EXAMPLE
  ./scripts/measure-footprint.ps1                            # window open
  ./scripts/measure-footprint.ps1 -Minimized                 # tray only
  ./scripts/measure-footprint.ps1 -FillHistoryMinutes 61     # full history: tray, then window
#>
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\oma-app.exe'),
    [int]$WarmupSeconds = 15,
    [int]$SampleSeconds = 30,
    [switch]$Minimized,
    [int]$FillHistoryMinutes = 0
)

$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path $Exe).Path

function Measure-Process([Diagnostics.Process]$Proc, [string]$Mode) {
    Start-Sleep -Seconds $WarmupSeconds
    $Proc.Refresh()
    if ($Proc.HasExited) { throw "The measured instance exited (another instance may already be running)." }
    $cpuStart = $Proc.TotalProcessorTime
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds $SampleSeconds
    $Proc.Refresh()
    $cpuEnd = $Proc.TotalProcessorTime
    $cpuPercent = ($cpuEnd - $cpuStart).TotalMilliseconds / $elapsed.Elapsed.TotalMilliseconds / [Environment]::ProcessorCount * 100

    # Follow the actual process tree, including renderer grandchildren.
    $processes = @(Get-CimInstance Win32_Process)
    $descendants = [Collections.Generic.HashSet[int]]::new()
    [void]$descendants.Add($Proc.Id)
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
    $ids = @($Proc.Id) + @($webviews | ForEach-Object { [int]$_.ProcessId })
    $perf = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process |
        Where-Object { $ids -contains [int]$_.IDProcess })
    if ($perf.Count -ne $ids.Count) { throw "Missing process memory counters; measurement is invalid." }
    $appPrivate = ($perf | Where-Object { [int]$_.IDProcess -eq $Proc.Id }).WorkingSetPrivate
    $vendorDlls = @('nvml.dll', 'nvapi64.dll', 'atiadlxx.dll', 'ControlLib.dll')
    $vendorModules = @($Proc.Modules | Where-Object { $vendorDlls -contains $_.ModuleName } |
        ForEach-Object { $_.ModuleName } | Sort-Object -Unique)
    $totalPrivate = ($perf | Measure-Object -Property WorkingSetPrivate -Sum).Sum

    [pscustomobject]@{
        Mode              = $Mode
        HistoryMinutes    = $FillHistoryMinutes
        CorePercentCpu    = [math]::Round($cpuPercent, 2)
        AppPrivateMB      = [math]::Round($appPrivate / 1MB, 1)
        WebView2Processes = $webviews.Count
        TotalPrivateMB    = [math]::Round($totalPrivate / 1MB, 1)
        VendorModules     = if ($vendorModules.Count) { $vendorModules -join ', ' } else { '(none)' }
    }
}

$fill = $FillHistoryMinutes -gt 0
$proc = if ($Minimized -or $fill) {
    Start-Process -FilePath $exePath -ArgumentList '--minimized' -WindowStyle Hidden -PassThru
} else {
    # Controller ruling: a hidden window would likely stop WebView rendering,
    # invalidating the "window open" measurement, so window mode launches
    # normally. -WindowStyle Hidden is kept only for -Minimized.
    Start-Process -FilePath $exePath -WindowStyle Normal -PassThru
}

try {
    if ($fill) {
        Start-Sleep -Seconds 5
        $proc.Refresh()
        if ($proc.HasExited) { throw "The measured instance exited (another instance may already be running)." }
        Start-Sleep -Seconds ($FillHistoryMinutes * 60)
        Measure-Process $proc 'tray' | Format-List
        if (-not $Minimized) {
            # Single instance: the second launch asks the running app to open its window, then exits.
            $second = Start-Process -FilePath $exePath -WindowStyle Normal -PassThru
            if (-not $second.WaitForExit(15000)) { throw "The second launch did not hand over to the running instance." }
            Measure-Process $proc 'window' | Format-List
        }
    } elseif ($Minimized) {
        Measure-Process $proc 'tray' | Format-List
    } else {
        Measure-Process $proc 'window' | Format-List
    }
}
finally {
    Stop-Process -Id $proc.Id -ErrorAction SilentlyContinue
}
