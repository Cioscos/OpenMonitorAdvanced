# Unit test the production function without executing the script's app-launching body.
$ErrorActionPreference = 'Stop'
$path = Join-Path $PSScriptRoot 'measure-footprint.ps1'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw "measure-footprint.ps1 has parser errors" }
$function = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Measure-AppCpuSample' }, $true)
if (-not $function) { throw 'Measure-AppCpuSample is missing: aggregate app and WebView2 CPU cannot be measured' }
. ([scriptblock]::Create($function.Extent.Text))

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

Write-Output 'measure-app-cpu: 12 cases passed'
