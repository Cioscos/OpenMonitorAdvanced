<#
.SYNOPSIS
  Prepares the window-mode budget measurement: makes the next start of
  OpenMonitor Advanced open the Advanced view on a given page and chart window.
.DESCRIPTION
  Starts the app with the WebView2 DevTools port enabled, writes the Advanced
  view state into the page's localStorage through the Chrome DevTools Protocol
  (Runtime.evaluate: no synthetic input, no UI Automation), and closes the app.
  It then starts the app a second time and checks that the page opens on the
  requested section, window and series, optionally saving a screenshot. The
  state lives in the WebView2 profile, so the next normal start (without the
  DevTools port, as in measure-footprint.ps1) opens on the same page.
  -CheckOnly skips the seeding and only checks what the next start opens.
  Every instance of the same executable must be closed first: with the single
  instance lock, a launch would only hand over to the running one.
.EXAMPLE
  ./scripts/seed-advanced-view.ps1 -Section 'gpu/pci-0000:01:00.0' -Window 3600 -Screenshot "$env:TEMP\oma-seed.png"
.EXAMPLE
  ./scripts/seed-advanced-view.ps1 -Section 'gpu/pci-0000:01:00.0' -Window 3600 -CheckOnly
#>
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\oma-app.exe'),
    [Parameter(Mandatory = $true)][string]$Section,
    [ValidateSet(60, 300, 1800, 3600)][int]$Window = 3600,
    # Sensor ids for the chart; empty keeps the page's default series.
    [string[]]$Series = @(),
    # Text that must appear on the page (for example the GPU name).
    [string]$ExpectText = '',
    [int]$Port = 9223,
    [string]$Screenshot = '',
    [switch]$CheckOnly
)

$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path $Exe).Path
$exeName = [IO.Path]::GetFileNameWithoutExtension($exePath)

if (Get-Process -Name $exeName -ErrorAction SilentlyContinue) {
    throw "Close every running $exeName.exe first (a new launch would hand over to it)."
}

function Get-Descendants([int]$RootId) {
    $processes = @(Get-CimInstance Win32_Process)
    $tree = [Collections.Generic.HashSet[int]]::new()
    [void]$tree.Add($RootId)
    do {
        $changed = $false
        foreach ($p in $processes) {
            if ($tree.Contains([int]$p.ParentProcessId) -and $tree.Add([int]$p.ProcessId)) { $changed = $true }
        }
    } while ($changed)
    return @($tree)
}

function Start-WithDevTools {
    $previous = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
    try {
        $proc = Start-Process -FilePath $exePath -WindowStyle Normal -PassThru
    }
    finally {
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $previous
    }
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 500
        $proc.Refresh()
        if ($proc.HasExited) { throw "$exeName.exe exited (another instance may be running)." }
        try {
            $page = @(Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json/list" -TimeoutSec 2 |
                Where-Object { $_.type -eq 'page' }) | Select-Object -First 1
        }
        catch { $page = $null }
        if ($page) {
            $state = @{ Process = $proc; Socket = $page.webSocketDebuggerUrl }
            while ((Get-Date) -lt $deadline) {
                if ((Invoke-Page $state 'document.readyState') -eq 'complete') { return $state }
                Start-Sleep -Milliseconds 500
            }
        }
    }
    throw "No WebView2 page on DevTools port $Port within 60 s."
}

function Invoke-Cdp($State, [string]$Method, [hashtable]$Params) {
    $socket = [Net.WebSockets.ClientWebSocket]::new()
    $none = [Threading.CancellationToken]::None
    $socket.ConnectAsync([Uri]$State.Socket, $none).GetAwaiter().GetResult()
    try {
        $request = @{ id = 1; method = $Method; params = $Params } | ConvertTo-Json -Depth 10 -Compress
        $bytes = [Text.Encoding]::UTF8.GetBytes($request)
        $socket.SendAsync([ArraySegment[byte]]::new($bytes), [Net.WebSockets.WebSocketMessageType]::Text, $true, $none).GetAwaiter().GetResult()
        $buffer = [byte[]]::new(65536)
        while ($true) {
            $message = [IO.MemoryStream]::new()
            do {
                $received = $socket.ReceiveAsync([ArraySegment[byte]]::new($buffer), $none).GetAwaiter().GetResult()
                $message.Write($buffer, 0, $received.Count)
            } until ($received.EndOfMessage)
            $reply = [Text.Encoding]::UTF8.GetString($message.ToArray()) | ConvertFrom-Json
            if ($reply.id -eq 1) {
                if ($reply.error) { throw "CDP $Method failed: $($reply.error.message)" }
                return $reply.result
            }
        }
    }
    finally {
        $socket.Dispose()
    }
}

function Invoke-Page($State, [string]$Expression) {
    $result = Invoke-Cdp $State 'Runtime.evaluate' @{ expression = $Expression; returnByValue = $true }
    if ($result.exceptionDetails) { throw "Page script failed: $($result.exceptionDetails.text)" }
    return $result.result.value
}

function Stop-Gracefully($State) {
    $proc = $State.Process
    $tree = Get-Descendants $proc.Id
    $children = @($tree | Where-Object { $_ -ne $proc.Id })
    # WM_CLOSE destroys the window (the app stays in the tray), so WebView2
    # shuts down normally and flushes localStorage to the profile on disk.
    [void]$proc.CloseMainWindow()
    $deadline = (Get-Date).AddSeconds(20)
    while ($children.Count -and (Get-Date) -lt $deadline -and
        @(Get-Process -Id $children -ErrorAction SilentlyContinue).Count) {
        Start-Sleep -Milliseconds 500
    }
    Stop-Process -Id $proc.Id -ErrorAction SilentlyContinue
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and @(Get-Process -Id $tree -ErrorAction SilentlyContinue).Count) {
        Start-Sleep -Milliseconds 500
    }
}

$sectionJs = ConvertTo-Json -InputObject $Section -Compress
$seriesJs = ConvertTo-Json -InputObject @($Series) -Compress
$expectJs = ConvertTo-Json -InputObject $ExpectText -Compress
$seed = @"
(() => {
  const section = $sectionJs;
  const series = $seriesJs;
  localStorage.setItem('oma.view', 'advanced');
  localStorage.setItem('oma.advanced.section', section);
  localStorage.setItem('oma.advanced.window', '$Window');
  if (series.length) localStorage.setItem('oma.advanced.series.' + section, JSON.stringify(series));
  else localStorage.removeItem('oma.advanced.series.' + section);
  location.reload();
  return 'seeded';
})()
"@
$check = @"
JSON.stringify({
  view: localStorage.getItem('oma.view'),
  section: localStorage.getItem('oma.advanced.section'),
  window: localStorage.getItem('oma.advanced.window'),
  series: localStorage.getItem('oma.advanced.series.' + $sectionJs),
  charts: document.querySelectorAll('.uplot').length,
  legendSeries: document.querySelectorAll('.uplot .u-legend .u-series').length,
  expectedText: $expectJs === '' || document.body.innerText.includes($expectJs)
})
"@

# 1. Seed the state, then close the app so WebView2 writes it to disk.
if (-not $CheckOnly) {
    $app = Start-WithDevTools
    try {
        [void](Invoke-Page $app $seed)
        Start-Sleep -Seconds 10
    }
    finally {
        Stop-Gracefully $app
    }
}

# 2. Start again and check that the page opens on the seeded state.
$app = Start-WithDevTools
try {
    Start-Sleep -Seconds 10
    $state = Invoke-Page $app $check | ConvertFrom-Json
    if ($Screenshot) {
        $image = Invoke-Cdp $app 'Page.captureScreenshot' @{ format = 'png' }
        [IO.File]::WriteAllBytes($Screenshot, [Convert]::FromBase64String($image.data))
    }
}
finally {
    Stop-Gracefully $app
}

$state | Format-List
$problems = @()
if ($state.view -ne 'advanced') { $problems += "view is '$($state.view)'" }
if ($state.section -ne $Section) { $problems += "section is '$($state.section)'" }
if ($state.window -ne "$Window") { $problems += "window is '$($state.window)'" }
if ($state.charts -lt 1) { $problems += 'no uPlot chart on the page' }
if (-not $state.expectedText) { $problems += "text '$ExpectText' not on the page" }
if ($problems.Count) { throw "The Advanced view did not open as seeded: $($problems -join '; ')" }
'Seeded: the next start opens the Advanced view on the requested page.'
