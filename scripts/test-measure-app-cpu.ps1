# Unit test the production function without executing the script's app-launching body.
$ErrorActionPreference = 'Stop'
$path = Join-Path $PSScriptRoot 'measure-footprint.ps1'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw "measure-footprint.ps1 has parser errors" }
foreach ($name in @('Measure-AppCpuSample', 'Measure-ServiceSample', 'Measure-ChildSample', 'Get-PresentMonProcessId')) {
    $function = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name }, $true)
    if (-not $function) { throw "$name is missing from measure-footprint.ps1" }
    . ([scriptblock]::Create($function.Extent.Text))
}

function Assert-Equal($Actual, $Expected, [string]$Label) {
    if ($Actual -ne $Expected) { throw "$Label`: expected $Expected, got $Actual" }
}

$tree = @(
    [pscustomobject]@{ ProcessId = 10; ParentProcessId = 0; Name = 'oma-app.exe'; CreationDate = 'A' },
    [pscustomobject]@{ ProcessId = 20; ParentProcessId = 10; Name = 'msedgewebview2.exe'; CreationDate = 'B' }
)
$startCounters = @(
    [pscustomobject]@{ IDProcess = 10; PercentProcessorTime = 1000000; Timestamp_Sys100NS = 1000000000 },
    [pscustomobject]@{ IDProcess = 20; PercentProcessorTime = 10000000; Timestamp_Sys100NS = 1000000000 }
)
$endCounters = @(
    [pscustomobject]@{ IDProcess = 10; PercentProcessorTime = 1800000; Timestamp_Sys100NS = 1100000000 },
    [pscustomobject]@{ IDProcess = 20; PercentProcessorTime = 18000000; Timestamp_Sys100NS = 1100000000 }
)
$script:reads = 0
$script:counterReads = 0
$script:samples = 0
$result = Measure-AppCpuSample -RootProcessId 10 -LogicalProcessors 16 `
    -ProcessProvider { $script:reads++; $tree } `
    -CounterProvider { $script:counterReads++; if ($script:counterReads -eq 1) { $startCounters } else { $endCounters } } `
    -SampleAction { $script:samples++ }
Assert-Equal $result.Valid $true 'aggregate validity'
Assert-Equal $result.CpuPercent 0.55 'host plus WebView2 machine CPU percent'
Assert-Equal $result.ProcessCount 2 'measured process count'
Assert-Equal $script:samples 1 'sample action count'

$script:counterReads = 0
$missing = Measure-AppCpuSample -RootProcessId 10 -LogicalProcessors 16 `
    -ProcessProvider { $tree } `
    -CounterProvider { $script:counterReads++; if ($script:counterReads -eq 1) { $startCounters } else { @($endCounters[0]) } } `
    -SampleAction { }
Assert-Equal $missing.Valid $false 'missing renderer counter validity'
Assert-Equal $null $missing.CpuPercent 'missing renderer counter result'

$script:reads = 0
$changedTree = @($tree[0], [pscustomobject]@{ ProcessId = 21; ParentProcessId = 10; Name = 'msedgewebview2.exe'; CreationDate = 'C' })
$turnover = Measure-AppCpuSample -RootProcessId 10 -LogicalProcessors 16 `
    -ProcessProvider { $script:reads++; if ($script:reads -eq 1) { $tree } else { $changedTree } } `
    -CounterProvider { $startCounters } `
    -SampleAction { }
Assert-Equal $turnover.Valid $false 'renderer turnover validity'
Assert-Equal $null $turnover.CpuPercent 'renderer turnover result'

$hostOnly = @($tree[0])
$hostOnlyCounters = @($startCounters[0])
$noRenderer = Measure-AppCpuSample -RootProcessId 10 -LogicalProcessors 16 -RequireWebView `
    -ProcessProvider { $hostOnly } `
    -CounterProvider { $hostOnlyCounters } `
    -SampleAction { }
Assert-Equal $noRenderer.Valid $false 'window without discovered WebView2 validity'
Assert-Equal $null $noRenderer.CpuPercent 'window without discovered WebView2 result'

foreach ($field in @('PercentProcessorTime', 'Timestamp_Sys100NS')) {
    foreach ($phase in @('start', 'end')) {
        foreach ($mode in @('null', 'missing')) {
            $rendererStart = [pscustomobject]@{ IDProcess = 20; PercentProcessorTime = 10000000; Timestamp_Sys100NS = 1000000000 }
            $rendererEnd = [pscustomobject]@{ IDProcess = 20; PercentProcessorTime = 18000000; Timestamp_Sys100NS = 1100000000 }
            $target = if ($phase -eq 'start') { $rendererStart } else { $rendererEnd }
            if ($mode -eq 'null') { $target.$field = $null }
            else { [void]$target.PSObject.Properties.Remove($field) }
            $badStart = @($startCounters[0], $rendererStart)
            $badEnd = @($endCounters[0], $rendererEnd)
            $script:counterReads = 0
            $invalidField = Measure-AppCpuSample -RootProcessId 10 -LogicalProcessors 16 `
                -ProcessProvider { $tree } `
                -CounterProvider { $script:counterReads++; if ($script:counterReads -eq 1) { $badStart } else { $badEnd } } `
                -SampleAction { }
            Assert-Equal $invalidField.Valid $false "$mode $field at $phase validity"
            Assert-Equal $null $invalidField.CpuPercent "$mode $field at $phase CPU"
        }
    }
}

# PresentMon, the service's child (M7b): found by name under the service PID only.
$pmName = 'PresentMon-2.6.0-x64.exe'
$services = @(
    [pscustomobject]@{ ProcessId = 500; ParentProcessId = 4; Name = 'oma-service.exe' },
    [pscustomobject]@{ ProcessId = 501; ParentProcessId = 500; Name = $pmName },
    [pscustomobject]@{ ProcessId = 777; ParentProcessId = 9; Name = $pmName }
)
Assert-Equal (Get-PresentMonProcessId -ServicePid 500 -ProcessProvider { $services }) 501 'PresentMon child of the service'
Assert-Equal $null (Get-PresentMonProcessId -ServicePid 9999 -ProcessProvider { $services }) 'no PresentMon under another PID'
Assert-Equal $null (Get-PresentMonProcessId -ServicePid $null -ProcessProvider { $services }) 'no service, no PresentMon'

# Not running: invalid with the reason "not running", and the sample window still elapses once.
$script:pmSamples = 0
$notRunning = Measure-ChildSample -PidProvider { $null } -SampleAction { $script:pmSamples++ } -Label $pmName
Assert-Equal $notRunning.Valid $false 'PresentMon not running validity'
Assert-Equal $notRunning.Reason 'not running' 'PresentMon not running reason'
Assert-Equal $script:pmSamples 1 'sample action count when PresentMon is not running'

# Running but without counters (a PID no process has): invalid before sampling, window still once.
$script:pmSamples = 0
$noCounters = Measure-ChildSample -PidProvider { 2147483000 } -SampleAction { $script:pmSamples++ } -Label $pmName
Assert-Equal $noCounters.Valid $false 'PresentMon without counters validity'
Assert-Equal ($noCounters.Reason -like 'No perf counters*') $true 'PresentMon without counters reason'
Assert-Equal $script:pmSamples 1 'sample action count without counters'

# A real process (this one): valid, measured over exactly one sample window.
$script:pmSamples = 0
$self = Measure-ChildSample -PidProvider { $PID } -SampleAction { $script:pmSamples++ } -Label 'pwsh'
Assert-Equal $self.Valid $true 'measured child validity'
Assert-Equal ($self.CpuPercent -ge 0) $true 'measured child CPU percent'
Assert-Equal ($self.PrivateBytesMB -gt 0) $true 'measured child private bytes'
Assert-Equal $script:pmSamples 1 'sample action count when measured'

Write-Output 'measure-app-cpu: 21 cases passed'
