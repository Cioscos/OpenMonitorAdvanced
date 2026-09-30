#Requires -Version 7
# Pester 5 tests for scripts/lib/OmaCommon.psm1.
#   Import-Module Pester -RequiredVersion 5.7.1
#   Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI

BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaCommon.psm1') -Force -ErrorAction Stop
    $script:pwshExe = (Get-Process -Id $PID).Path
}

Describe 'Invoke-OmaNative' {
    It 'returns_stdout' {
        $r = Invoke-OmaNative -FilePath $pwshExe -ArgumentList @(
            '-NoProfile', '-NonInteractive', '-Command',
            '[Console]::Out.Write(''hello out''); [Console]::Error.Write(''hello err'')')
        $r.Stdout | Should -BeExactly 'hello out'
        $r.Stderr | Should -BeExactly 'hello err'
        $r.ExitCode | Should -Be 0
    }

    It 'passes arguments with spaces and quotes intact' {
        $script = Join-Path $TestDrive 'echo-args.ps1'
        Set-Content -LiteralPath $script -Value '[Console]::Out.Write(($args -join ''|''))'
        $r = Invoke-OmaNative -FilePath $pwshExe -ArgumentList @(
            '-NoProfile', '-NonInteractive', '-File', $script, 'a "b" c d', 'e (f)')
        $r.Stdout | Should -BeExactly 'a "b" c d|e (f)'
    }

    It 'throws_on_nonzero_exit' {
        { Invoke-OmaNative -FilePath $pwshExe -ArgumentList @(
                '-NoProfile', '-NonInteractive', '-Command', '[Console]::Error.Write(''boom''); exit 3') } |
            Should -Throw -ExpectedMessage '*exited with code 3*boom*'
    }

    It 'returns the exit code with AllowFailure' {
        $r = Invoke-OmaNative -FilePath $pwshExe -AllowFailure -ArgumentList @(
            '-NoProfile', '-NonInteractive', '-Command', 'exit 3')
        $r.ExitCode | Should -Be 3
    }

    It 'runs in the given working directory' {
        $dir = New-Item -ItemType Directory (Join-Path $TestDrive 'work dir')
        $r = Invoke-OmaNative -FilePath $pwshExe -WorkingDirectory $dir.FullName -ArgumentList @(
            '-NoProfile', '-NonInteractive', '-Command', '[Console]::Out.Write([Environment]::CurrentDirectory)')
        $r.Stdout | Should -Be $dir.FullName
    }

    It 'throws when the command does not exist' {
        { Invoke-OmaNative -FilePath 'oma-no-such-command-xyz' -ArgumentList @() } | Should -Throw
    }
}

Describe 'Get-OmaSha256' {
    It 'returns the lowercase hex digest' {
        $f = Join-Path $TestDrive 'abc.bin'
        [IO.File]::WriteAllBytes($f, [Text.Encoding]::ASCII.GetBytes('abc'))
        Get-OmaSha256 -Path $f | Should -BeExactly 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
    }

    It 'throws for a missing file' {
        { Get-OmaSha256 -Path (Join-Path $TestDrive 'missing.bin') } | Should -Throw
    }
}

Describe 'Resolve-OmaPath' {
    It 'normalises mixed separators and dot segments' {
        Resolve-OmaPath -Path 'C:\a\b/c\..\d/e.exe' | Should -BeExactly 'C:\a\b\d\e.exe'
    }

    It 'removes a trailing separator' {
        Resolve-OmaPath -Path 'C:\a\b\' | Should -BeExactly 'C:\a\b'
        Resolve-OmaPath -Path 'C:\a\b/' | Should -BeExactly 'C:\a\b'
    }

    It 'preserves a drive root' {
        Resolve-OmaPath -Path 'C:\' | Should -BeExactly 'C:\'
        Resolve-OmaPath -Path 'C:/' | Should -BeExactly 'C:\'
    }

    It 'rejects a relative path' {
        { Resolve-OmaPath -Path 'target\signing' } | Should -Throw -ExpectedMessage '*absolute*'
        { Resolve-OmaPath -Path 'C:target' } | Should -Throw -ExpectedMessage '*absolute*'
        { Resolve-OmaPath -Path '\target' } | Should -Throw -ExpectedMessage '*absolute*'
    }
}
