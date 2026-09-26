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
  With -Service, the same warm-up/sample window also measures the
  `oma-service` Windows service (spec §1.2 service budget): CPU % normalised
  by the number of logical processors, and Private Bytes (not the private
  working set used for the app), both read from
  Win32_PerfFormattedData_PerfProc_Process matched by IDProcess — the PID the
  SCM reports for the service, never a locale-dependent PDH counter name or
  an ambiguous `oma-service#n` instance. The PID is checked again after
  sampling; if the service is not running, its PID changed, or its counters
  are missing, the service measurement is reported INVALID, never as zero.
  Reading another account's (LocalSystem) process counters can require an
  elevated PowerShell session on some machines; an invalid reading whose
  cause looks like a permissions issue says so.
.EXAMPLE
  ./scripts/measure-footprint.ps1                            # window open
  ./scripts/measure-footprint.ps1 -Minimized                 # tray only
  ./scripts/measure-footprint.ps1 -FillHistoryMinutes 61     # full history: tray, then window
  ./scripts/measure-footprint.ps1 -Service                   # also measure oma-service
#>
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\oma-app.exe'),
    [int]$WarmupSeconds = 15,
    [int]$SampleSeconds = 30,
    [switch]$Minimized,
    [int]$FillHistoryMinutes = 0,
    [switch]$Service,
    [string]$ServiceName = 'oma-service'
)

$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path $Exe).Path

# Returns the PID the SCM currently reports for a service, or $null when the
# service is not installed, not running, or has no PID (stopped/paused).
# Kept separate from the app's process object so it can be swapped for a
# fake provider in tests, without starting or touching any real service.
function Get-ServiceProcessId([string]$Name) {
    $svc = Get-CimInstance Win32_Service -Filter "Name='$Name'" -ErrorAction SilentlyContinue
    if (-not $svc -or -not $svc.ProcessId -or $svc.ProcessId -eq 0) { return $null }
    return [int]$svc.ProcessId
}

# Samples CPU % (normalised by logical processor count) and Private Bytes for
# a service across the interval that $SampleAction runs (the same
# warm-up-then-sample window used for the app). $PidProvider is called before
# and after $SampleAction so a service that is not running, that stops, or
# whose PID changes mid-sample is caught and reported invalid, never as a
# zero reading. Both parameters are script blocks so this function can be
# unit-tested without starting or stopping oma-service, or anything else.
function Measure-ServiceSample {
    param(
        [Parameter(Mandatory)][scriptblock]$PidProvider,
        [Parameter(Mandatory)][scriptblock]$SampleAction,
        [string]$ServiceLabel = 'oma-service',
        [int]$LogicalProcessors = [Environment]::ProcessorCount
    )

    $pidStart = & $PidProvider
    if (-not $pidStart) {
        return [pscustomobject]@{ Valid = $false; Reason = "$ServiceLabel is not running (the SCM reported no PID); measurement is INVALID." }
    }

    # Prime the formatted counter: WMI's perf provider computes a rate from
    # the previous raw sample it has cached for this PID, so the very first
    # read after the process started (or after this script's first query)
    # can reflect its whole lifetime rather than a short interval. Discard it
    # before running the timed sample.
    [void](Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$pidStart" -ErrorAction SilentlyContinue)

    & $SampleAction

    $pidEnd = & $PidProvider
    if (-not $pidEnd) {
        return [pscustomobject]@{ Valid = $false; Reason = "$ServiceLabel stopped during sampling; measurement is INVALID." }
    }
    if ($pidEnd -ne $pidStart) {
        return [pscustomobject]@{ Valid = $false; Reason = "$ServiceLabel PID changed during sampling ($pidStart -> $pidEnd); measurement is INVALID." }
    }

    $perf = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$pidEnd" -ErrorAction SilentlyContinue)
    if ($perf.Count -eq 0) {
        return [pscustomobject]@{ Valid = $false; Reason = "No perf counters for PID $pidEnd; measurement is INVALID (an elevated PowerShell session may be required to read a LocalSystem process's counters on this machine)." }
    }
    if ($perf.Count -gt 1) {
        return [pscustomobject]@{ Valid = $false; Reason = "Ambiguous perf-counter match for PID $pidEnd (more than one instance); measurement is INVALID." }
    }

    [pscustomobject]@{
        Valid          = $true
        ServicePid     = $pidEnd
        CpuPercent     = [math]::Round($perf[0].PercentProcessorTime / $LogicalProcessors, 2)
        PrivateBytesMB = [math]::Round($perf[0].PrivateBytes / 1MB, 1)
    }
}

function Measure-Process([Diagnostics.Process]$Proc, [string]$Mode, [switch]$MeasureService) {
    Start-Sleep -Seconds $WarmupSeconds
    $Proc.Refresh()
    if ($Proc.HasExited) { throw "The measured instance exited (another instance may already be running)." }
    $cpuStart = $Proc.TotalProcessorTime

    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    $svcResult = $null
    if ($MeasureService) {
        # Same interval as the app: $SampleAction below is the only sleep,
        # and the stopwatch wraps it exactly as it would without -Service.
        $svcResult = Measure-ServiceSample -PidProvider { Get-ServiceProcessId -Name $ServiceName } `
            -SampleAction { Start-Sleep -Seconds $SampleSeconds } -ServiceLabel $ServiceName
    } else {
        Start-Sleep -Seconds $SampleSeconds
    }
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

    $result = [ordered]@{
        Mode              = $Mode
        HistoryMinutes    = $FillHistoryMinutes
        CorePercentCpu    = [math]::Round($cpuPercent, 2)
        AppPrivateMB      = [math]::Round($appPrivate / 1MB, 1)
        WebView2Processes = $webviews.Count
        TotalPrivateMB    = [math]::Round($totalPrivate / 1MB, 1)
        VendorModules     = if ($vendorModules.Count) { $vendorModules -join ', ' } else { '(none)' }
    }
    if ($MeasureService) {
        if ($svcResult.Valid) {
            $result['ServiceValid']         = $true
            $result['ServicePid']           = $svcResult.ServicePid
            $result['ServiceCorePercentCpu'] = $svcResult.CpuPercent
            $result['ServicePrivateBytesMB'] = $svcResult.PrivateBytesMB
            $result['ServiceInvalidReason']  = $null
        } else {
            $result['ServiceValid']          = $false
            $result['ServicePid']            = $null
            $result['ServiceCorePercentCpu'] = $null
            $result['ServicePrivateBytesMB'] = $null
            $result['ServiceInvalidReason']  = $svcResult.Reason
        }
    }
    [pscustomobject]$result
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

if ($Service) {
    Write-Host "Also measuring service '$ServiceName': CPU normalised by $([Environment]::ProcessorCount) logical processors, Private Bytes via Win32_PerfFormattedData_PerfProc_Process by IDProcess. Reading a LocalSystem process's counters can require an elevated PowerShell session on some machines; an invalid reading whose cause looks like a permissions issue will say so."
}

try {
    if ($fill) {
        Start-Sleep -Seconds 5
        $proc.Refresh()
        if ($proc.HasExited) { throw "The measured instance exited (another instance may already be running)." }
        Start-Sleep -Seconds ($FillHistoryMinutes * 60)
        Measure-Process $proc 'tray' -MeasureService:$Service | Format-List
        if (-not $Minimized) {
            # Single instance: the second launch asks the running app to open its window, then exits.
            $second = Start-Process -FilePath $exePath -WindowStyle Normal -PassThru
            if (-not $second.WaitForExit(15000)) { throw "The second launch did not hand over to the running instance." }
            Measure-Process $proc 'window' -MeasureService:$Service | Format-List
        }
    } elseif ($Minimized) {
        Measure-Process $proc 'tray' -MeasureService:$Service | Format-List
    } else {
        Measure-Process $proc 'window' -MeasureService:$Service | Format-List
    }
}
finally {
    Stop-Process -Id $proc.Id -ErrorAction SilentlyContinue
}
