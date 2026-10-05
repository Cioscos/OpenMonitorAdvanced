#Requires -Version 7
# Pester 5 tests for the pure helpers of scripts/measure-footprint.ps1. The script itself starts
# the app, so only its function definitions are loaded (from the parsed script, never run):
# nothing is started, no counter or process is read.
#   Import-Module Pester -RequiredVersion 5.7.1
#   Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI

BeforeAll {
    $script:footprintScript = (Resolve-Path (Join-Path $PSScriptRoot '..\measure-footprint.ps1')).Path
    $tokens = $null
    $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($footprintScript, [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw "measure-footprint.ps1 does not parse: $($errors[0].Message)" }
    $functions = $ast.FindAll({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)
    foreach ($f in $functions) { . ([scriptblock]::Create($f.Extent.Text)) }

    function New-Proc([int]$Id, [int]$Parent, [string]$Name) {
        [pscustomobject]@{ ProcessId = $Id; ParentProcessId = $Parent; Name = $Name }
    }
}

Describe 'overlay measurement (spec M7 §11)' {
    It 'footprint reports the overlay child of the app' {
        # The overlay is the app's direct child named oma-overlay.exe; another app's overlay
        # and the app's WebView2 children do not count.
        $list = @(
            (New-Proc 100 1 'oma-app.exe'),
            (New-Proc 200 100 'oma-overlay.exe'),
            (New-Proc 300 100 'msedgewebview2.exe'),
            (New-Proc 400 999 'oma-overlay.exe')
        )
        Get-OverlayProcessId -AppPid 100 -ProcessProvider { $list } | Should -Be 200
        Get-OverlayProcessId -AppPid 100 -ProcessProvider { @($list[0], $list[2], $list[3]) } | Should -BeNullOrEmpty
        # Two at once (a restart caught half-way) is not a measurement.
        Get-OverlayProcessId -AppPid 100 -ProcessProvider { $list + @(New-Proc 201 100 'oma-overlay.exe') } | Should -BeNullOrEmpty
        Get-OverlayProcessId -AppPid $null -ProcessProvider { $list } | Should -BeNullOrEmpty

        $valid = [pscustomobject]@{ Valid = $true; ServicePid = 200; CpuPercent = 0.12; PrivateBytesMB = 21.4 }
        $f = ConvertTo-OverlayFields $valid
        @($f.Keys) | Should -Be @('OverlayValid', 'OverlayCpuPercent', 'OverlayPrivateBytesMB', 'OverlayInvalidReason')
        $f.OverlayValid | Should -BeTrue
        $f.OverlayCpuPercent | Should -Be 0.12
        $f.OverlayPrivateBytesMB | Should -Be 21.4
        $f.OverlayInvalidReason | Should -BeNullOrEmpty

        $f = ConvertTo-OverlayFields ([pscustomobject]@{ Valid = $false; Reason = 'not running' })
        $f.OverlayValid | Should -BeFalse
        $f.OverlayCpuPercent | Should -BeNullOrEmpty
        $f.OverlayPrivateBytesMB | Should -BeNullOrEmpty
        $f.OverlayInvalidReason | Should -Be 'not running'
        # No result at all (the sample never ran) is invalid too, never zero.
        (ConvertTo-OverlayFields $null).OverlayValid | Should -BeFalse
    }

    It 'an absent overlay keeps the sample window and reports not running' {
        $ran = @{ Count = 0 }
        $r = Measure-ChildSample -Label 'oma-overlay.exe' -PidProvider { Get-OverlayProcessId -AppPid 100 -ProcessProvider { @() } } `
            -SampleAction { $ran.Count++ }
        $ran.Count | Should -Be 1
        $r.Valid | Should -BeFalse
        $r.Reason | Should -Be 'not running'
    }
}
