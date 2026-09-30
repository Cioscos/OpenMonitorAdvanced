#Requires -Version 7
# Pester 5 tests for scripts/lib/OmaSigning.psm1 and scripts/sign-shim.ps1 (spec §3.2).
# Every test builds its own fake repository under $TestDrive with files of a few bytes;
# TMP/TEMP point into $TestDrive, so the fake NSIS uninstallers (nstXXXX.tmp) never touch
# the real temp directory.
#   Import-Module Pester -RequiredVersion 5.7.1
#   Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI

BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaCommon.psm1') -Force -ErrorAction Stop
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaSigning.psm1') -Force -ErrorAction Stop
    $script:shim = (Resolve-Path (Join-Path $PSScriptRoot '..\sign-shim.ps1')).Path
    $script:pwshExe = (Get-Process -Id $PID).Path

    $script:savedTmp = $env:TMP
    $script:savedTemp = $env:TEMP
    $fakeTemp = Join-Path $TestDrive 'temp dir'
    New-Item -ItemType Directory -Force $fakeTemp | Out-Null
    $env:TMP = $fakeTemp
    $env:TEMP = $fakeTemp

    # The fake runs must not be compared with the real run when the tests themselves run in
    # GitHub Actions; the 'GitHub Actions context' tests set these explicitly.
    $script:githubVars = @('GITHUB_ACTIONS', 'GITHUB_SHA', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT')
    $script:savedGithub = @{}
    foreach ($n in $githubVars) {
        $savedGithub[$n] = [Environment]::GetEnvironmentVariable($n)
        Remove-Item "Env:$n" -ErrorAction SilentlyContinue
    }

    $script:pluginNames = @('NSISdl.dll', 'StartMenu.dll', 'System.dll', 'nsDialogs.dll', 'additional\nsis_tauri_utils.dll')

    function Write-Fake([string]$Path, [string]$Text) {
        New-Item -ItemType Directory -Force (Split-Path $Path) | Out-Null
        [IO.File]::WriteAllText($Path, $Text)
    }

    function New-Context([string]$RunId = '42', [string]$Commit = ('a' * 40)) {
        [pscustomobject]@{ Commit = $Commit; Version = '0.3.0'; RunId = $RunId; RunAttempt = '1' }
    }

    # A fake repository with the files Tauri would pass to signCommand, laid out as the spike recorded.
    function New-FakeRepo([string]$Name = 'repo') {
        $root = Join-Path (Join-Path $TestDrive ([guid]::NewGuid().ToString('N').Substring(0, 8))) $Name
        $rel = Join-Path $root 'target\release'
        $r = [pscustomobject]@{
            Root      = $root
            State     = Join-Path $root 'target\signing'
            App       = Join-Path $rel 'oma-app.exe'
            SetupDir  = Join-Path $rel 'bundle\nsis'
            Setup     = Join-Path $rel 'bundle\nsis\OpenMonitor Advanced_0.3.0_x64-setup.exe'
            # Tauri passes the setup with mixed separators.
            SetupArg  = (Join-Path $rel 'bundle') + '/nsis/OpenMonitor Advanced_0.3.0_x64-setup.exe'
            PluginDir = Join-Path $rel 'nsis\x64\Plugins\x86-unicode'
            Plugins   = @()
            Service   = Join-Path $root 'target\installer-payload\service\oma-service.exe'
            Context   = New-Context
        }
        Write-Fake $r.App 'app patched by tauri'
        Write-Fake $r.Setup 'setup of the first pass'
        Write-Fake $r.Service 'service unsigned'
        $r.Plugins = foreach ($p in $pluginNames) {
            $full = Join-Path $r.PluginDir $p
            Write-Fake $full "plugin $p"
            # Tauri passes additional/nsis_tauri_utils.dll with a forward slash.
            $full.Replace('additional\', 'additional/')
        }
        $r
    }

    # A fresh NSIS temporary uninstaller in GetTempPath, with the given content.
    function New-Uninstaller([string]$Text = 'uninstaller bytes', [string]$Dir = [IO.Path]::GetTempPath()) {
        do {
            $path = Join-Path $Dir ('nst{0:X4}.tmp' -f (Get-Random -Maximum 65536))
        } while (Test-Path -LiteralPath $path)
        Write-Fake $path $Text
        $path
    }

    function Initialize-Fake($r) {
        Initialize-OmaSigningState -StateRoot $r.State -RepoRoot $r.Root -Commit $r.Context.Commit `
            -Version $r.Context.Version -RunId $r.Context.RunId -RunAttempt $r.Context.RunAttempt
    }

    function Invoke-CollectPass($r, [switch]$SkipUninstaller) {
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App
        foreach ($p in $r.Plugins) { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $p }
        if (-not $SkipUninstaller) {
            Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path (New-Uninstaller)
        }
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.SetupArg
        Register-OmaService -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Service
    }

    function New-SignedDir($r, [string[]]$Names = @('oma-app.exe', 'uninstall.exe', 'oma-service.exe')) {
        $dir = Join-Path (Split-Path $r.Root) ('returned-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
        foreach ($n in $Names) { Write-Fake (Join-Path $dir $n) "signed $n" }
        $dir
    }

    function Import-FakeSigned($r) {
        Import-OmaSignedFiles -StateRoot $r.State -From (New-SignedDir $r)
    }

    # Second pass: Tauri produces the same app and uninstaller bytes again and a new setup.
    function Invoke-ApplyPass($r) {
        Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App
        foreach ($p in $r.Plugins) { Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $p }
        Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path (New-Uninstaller)
        Write-Fake $r.Setup 'setup of the second pass'
        Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.SetupArg
    }

    function Read-Manifest($r) {
        Get-Content -Raw -LiteralPath (Join-Path $r.State 'manifest.json') | ConvertFrom-Json
    }

    function Get-Sha([string]$Text) {
        (Get-FileHash -InputStream ([IO.MemoryStream]::new([Text.Encoding]::UTF8.GetBytes($Text))) -Algorithm SHA256).Hash.ToLowerInvariant()
    }

    # Runs sign-shim.ps1 in a separate pwsh, like Tauri and makensis do.
    function Invoke-ShimScript([string[]]$Arguments, [string]$WorkingDirectory = $TestDrive) {
        Invoke-OmaNative -FilePath $pwshExe -WorkingDirectory $WorkingDirectory -AllowFailure `
            -ArgumentList (@('-NoProfile', '-NonInteractive', '-File', $shim) + $Arguments)
    }
}

AfterAll {
    $env:TMP = $savedTmp
    $env:TEMP = $savedTemp
    foreach ($n in $githubVars) { [Environment]::SetEnvironmentVariable($n, $savedGithub[$n]) }
}

Describe 'Initialize-OmaSigningState' {
    It 'init_creates_empty_state_and_configs' {
        $r = New-FakeRepo
        Write-Fake (Join-Path $r.State 'unsigned\stale.exe') 'left over from another run'
        Initialize-Fake $r

        (Get-ChildItem -LiteralPath $r.State -Name | Sort-Object) |
            Should -Be @('manifest.json', 'signed', 'tauri.sign.apply.json', 'tauri.sign.collect.json', 'unsigned')
        Get-ChildItem -LiteralPath (Join-Path $r.State 'unsigned') | Should -BeNullOrEmpty
        Get-ChildItem -LiteralPath (Join-Path $r.State 'signed') | Should -BeNullOrEmpty

        $m = Read-Manifest $r
        $m.schema | Should -Be 1
        $m.commit | Should -Be $r.Context.Commit
        $m.version | Should -Be '0.3.0'
        $m.runId | Should -Be '42'
        $m.runAttempt | Should -Be '1'
        $m.expected.app | Should -Be $r.App
        $m.expected.setupDir | Should -Be $r.SetupDir
        $m.expected.setupName | Should -Be 'OpenMonitor Advanced_0.3.0_x64-setup.exe'
        @($m.collect).Count | Should -Be 0
        @($m.apply).Count | Should -Be 0

        foreach ($mode in 'collect', 'apply') {
            $cfg = Get-Content -Raw -LiteralPath (Join-Path $r.State "tauri.sign.$mode.json") | ConvertFrom-Json
            $sc = $cfg.bundle.windows.signCommand
            $sc.cmd | Should -Be 'pwsh'
            $a = @($sc.args)
            $a | Should -Contain '%1'
            $a[-1] | Should -BeExactly '%1'
            $a[-2] | Should -BeExactly '-Path'
            $a[0..2] | Should -Be @('-NoProfile', '-NonInteractive', '-File')
            [IO.Path]::IsPathFullyQualified($a[3]) | Should -BeTrue
            $a[3] | Should -Be $shim
            $a[$a.IndexOf('-Mode') + 1] | Should -BeExactly $mode
            $stateArg = $a[$a.IndexOf('-StateRoot') + 1]
            [IO.Path]::IsPathFullyQualified($stateArg) | Should -BeTrue
            $stateArg | Should -Be $r.State
            $a[$a.IndexOf('-Commit') + 1] | Should -Be $r.Context.Commit
            $a[$a.IndexOf('-Version') + 1] | Should -Be '0.3.0'
            $a[$a.IndexOf('-RunId') + 1] | Should -Be '42'
            $a[$a.IndexOf('-RunAttempt') + 1] | Should -Be '1'
        }
    }

    It 'init_rejects_repo_root_target_root_and_junctions' {
        $r = New-FakeRepo
        $ctx = $r.Context
        $init = {
            param($state)
            Initialize-OmaSigningState -StateRoot $state -RepoRoot $r.Root -Commit $ctx.Commit `
                -Version $ctx.Version -RunId $ctx.RunId -RunAttempt $ctx.RunAttempt
        }
        $target = Join-Path $r.Root 'target'
        $bad = @(
            $r.Root,
            $target,
            "$target\",
            [IO.Path]::GetPathRoot($r.Root),
            (Split-Path $r.Root),
            (Join-Path $target 'installer-payload'),
            (Join-Path $target 'installer-payload\service'),
            (Join-Path $target 'release'),
            (Join-Path $target 'release\bundle'),
            (Join-Path $target '..\elsewhere'),
            (Join-Path $TestDrive 'outside\signing')
        )
        foreach ($state in $bad) {
            { & $init $state } | Should -Throw -Because "state root $state must be refused"
        }
        { & $init 'target\signing' } | Should -Throw -ExpectedMessage '*absolute*'

        # A junction under target/ pointing elsewhere: nothing behind it may be deleted.
        $victim = Join-Path $TestDrive ('victim-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
        Write-Fake (Join-Path $victim 'signing\keep.txt') 'precious'
        New-Item -ItemType Junction -Path (Join-Path $target 'link') -Target $victim | Out-Null
        { & $init (Join-Path $target 'link\signing') } | Should -Throw -ExpectedMessage '*reparse*'
        # The state root itself as a junction.
        New-Item -ItemType Junction -Path (Join-Path $target 'signing-j') -Target (Join-Path $victim 'signing') | Out-Null
        { & $init (Join-Path $target 'signing-j') } | Should -Throw -ExpectedMessage '*reparse*'
        Get-Content -LiteralPath (Join-Path $victim 'signing\keep.txt') | Should -Be 'precious'

        # Nothing of the repository was deleted by the refused calls.
        Test-Path -LiteralPath $r.App | Should -BeTrue
        Test-Path -LiteralPath $r.Service | Should -BeTrue
    }

    It 'rejects context values that are not safe on the signCommand line' {
        $r = New-FakeRepo
        $ok = $r.Context
        $cases = @(
            @{ Commit = 'abc'; Version = $ok.Version; RunId = $ok.RunId; RunAttempt = $ok.RunAttempt },
            @{ Commit = $ok.Commit; Version = '0.3'; RunId = $ok.RunId; RunAttempt = $ok.RunAttempt },
            @{ Commit = $ok.Commit; Version = '0.3.70000'; RunId = $ok.RunId; RunAttempt = $ok.RunAttempt },
            @{ Commit = $ok.Commit; Version = $ok.Version; RunId = '4$2'; RunAttempt = $ok.RunAttempt },
            @{ Commit = $ok.Commit; Version = $ok.Version; RunId = $ok.RunId; RunAttempt = "1'" }
        )
        foreach ($c in $cases) {
            { Initialize-OmaSigningState -StateRoot $r.State -RepoRoot $r.Root @c } | Should -Throw
        }
    }
}

Describe 'Invoke-OmaSignShim collect' {
    BeforeEach {
        $r = New-FakeRepo
        Initialize-Fake $r
    }

    It 'collect_records_role_name_and_hash' {
        $appBefore = Get-OmaSha256 $r.App
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App
        $uninst = New-Uninstaller 'the uninstaller'
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $uninst
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.SetupArg

        $m = Read-Manifest $r
        $c = @($m.collect)
        $c.Count | Should -Be 3
        $c[0].role | Should -Be 'app'
        $c[0].name | Should -Be 'oma-app.exe'
        $c[0].path | Should -Be $r.App
        $c[0].sha256 | Should -BeExactly $appBefore
        $c[0].after | Should -BeExactly $appBefore
        $c[1].role | Should -Be 'uninstaller'
        $c[1].name | Should -Be 'uninstall.exe'
        $c[1].path | Should -Be $uninst
        $c[1].sha256 | Should -BeExactly (Get-Sha 'the uninstaller')
        $c[2].role | Should -Be 'setup'
        $c[2].name | Should -Be 'OpenMonitor Advanced_0.3.0_x64-setup.exe'
        $c[2].path | Should -Be $r.Setup
        $c[2].sha256 | Should -BeExactly (Get-OmaSha256 $r.Setup)

        # The received files stay as they were; only app and uninstaller are copied.
        Get-OmaSha256 $r.App | Should -BeExactly $appBefore
        Get-OmaSha256 (Join-Path $r.State 'unsigned\oma-app.exe') | Should -BeExactly $appBefore
        Get-OmaSha256 (Join-Path $r.State 'unsigned\uninstall.exe') | Should -BeExactly (Get-Sha 'the uninstaller')
        (Get-ChildItem -LiteralPath (Join-Path $r.State 'unsigned') -Name | Sort-Object) |
            Should -Be @('oma-app.exe', 'uninstall.exe')
    }

    It 'register_service_records_role_service' {
        Register-OmaService -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Service
        $c = @((Read-Manifest $r).collect)
        $c.Count | Should -Be 1
        $c[0].role | Should -Be 'service'
        $c[0].name | Should -Be 'oma-service.exe'
        $c[0].sha256 | Should -BeExactly (Get-Sha 'service unsigned')
        Get-OmaSha256 (Join-Path $r.State 'unsigned\oma-service.exe') | Should -BeExactly (Get-Sha 'service unsigned')

        { Register-OmaService -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Service } | Should -Throw -ExpectedMessage 'duplicate service call'
        $other = Join-Path $r.Root 'target\release\oma-service.exe'
        Write-Fake $other 'not the payload'
        { Register-OmaService -StateRoot $r.State -ExpectedContext $r.Context -Path $other } | Should -Throw
    }

    It 'plugins_are_left_intact' {
        foreach ($p in $r.Plugins) {
            $before = Get-OmaSha256 $p
            Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $p
            Get-OmaSha256 $p | Should -BeExactly $before
        }
        $c = @((Read-Manifest $r).collect)
        $c.Count | Should -Be 5
        $c.role | Should -Be @('plugin', 'plugin', 'plugin', 'plugin', 'plugin')
        foreach ($e in $c) { $e.after | Should -BeExactly $e.sha256 }
        Get-ChildItem -LiteralPath (Join-Path $r.State 'unsigned') | Should -BeNullOrEmpty

        # In apply a plugin must still match what collect saw.
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path (New-Uninstaller)
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.SetupArg
        Register-OmaService -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Service
        Import-FakeSigned $r
        Write-Fake (Join-Path $r.PluginDir 'System.dll') 'a different System.dll'
        { Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path (Join-Path $r.PluginDir 'System.dll') } |
            Should -Throw -ExpectedMessage '*System.dll*'
    }

    It 'rejects_an_unexpected_path' {
        $tmp = [IO.Path]::GetTempPath()
        $cases = @(
            (Join-Path $r.Root 'target\release\other.exe'),
            (Join-Path $r.PluginDir 'additional\evil.dll'),
            (Join-Path $r.PluginDir 'System.dll.bak'),
            (Join-Path $r.Root 'target\release\bundle\nsis\OpenMonitor Advanced_0.2.0_x64-setup.exe'),
            (Join-Path $tmp 'nst2A98.exe'),
            (Join-Path $tmp 'nstZZ.tmp'),
            (Join-Path $tmp 'nst2A98.tmp.bak'),
            (Join-Path $r.Root 'nst2A98.tmp')
        )
        foreach ($p in $cases) {
            Write-Fake $p 'anything'
            { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $p } |
                Should -Throw -ExpectedMessage "unexpected file passed to signCommand: *" -Because $p
        }
        # A trailing newline must not sneak past the \z anchors.
        foreach ($p in ($r.Plugins[2] + "`n"), ((New-Uninstaller) + "`n")) {
            { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $p } |
                Should -Throw -ExpectedMessage 'unexpected file passed to signCommand: *'
        }
        # A relative path is refused whatever the current directory.
        { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path 'oma-app.exe' } | Should -Throw
        @((Read-Manifest $r).collect).Count | Should -Be 0
    }

    It 'rejects_a_second_uninstaller' {
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path (New-Uninstaller)
        { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path (New-Uninstaller) } |
            Should -Throw -ExpectedMessage 'duplicate uninstaller call'
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App
        { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App } |
            Should -Throw -ExpectedMessage 'duplicate app call'
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.SetupArg
        { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Setup } |
            Should -Throw -ExpectedMessage 'duplicate setup call'
    }

    It 'recognises the uninstaller in whatever directory GetTempPath returns' {
        $other = Join-Path $TestDrive 'tmp-c'
        New-Item -ItemType Directory -Force $other | Out-Null
        $u = New-Uninstaller 'x' $other
        { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $u } |
            Should -Throw -ExpectedMessage 'unexpected file passed to signCommand: *'
        $env:TMP = $other
        try {
            Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $u
        } finally {
            $env:TMP = $env:TEMP
        }
        @((Read-Manifest $r).collect)[0].role | Should -Be 'uninstaller'
    }
}

Describe 'Invoke-OmaSignShim apply' {
    BeforeEach {
        $r = New-FakeRepo
        Initialize-Fake $r
        Invoke-CollectPass $r
        $unsignedApp = Get-OmaSha256 $r.App
    }

    It 'apply_replaces_with_the_signed_copy' {
        $collected = @((Read-Manifest $r).collect)
        Import-FakeSigned $r
        $u = New-Uninstaller
        Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App
        Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $u

        Get-OmaSha256 $r.App | Should -BeExactly (Get-Sha 'signed oma-app.exe')
        Get-OmaSha256 $u | Should -BeExactly (Get-Sha 'signed uninstall.exe')
        $m = Read-Manifest $r
        $m.signed.'oma-app.exe' | Should -BeExactly (Get-Sha 'signed oma-app.exe')
        $a = @($m.apply)
        $a.Count | Should -Be 2
        $a[0].role | Should -Be 'app'
        $a[0].sha256 | Should -BeExactly $unsignedApp
        $a[0].after | Should -BeExactly (Get-Sha 'signed oma-app.exe')
        $a[1].role | Should -Be 'uninstaller'
        $a[1].name | Should -Be 'uninstall.exe'
        $a[1].after | Should -BeExactly (Get-Sha 'signed uninstall.exe')
        # The collect rows are frozen.
        (@($m.collect) | ConvertTo-Json -Depth 5) | Should -Be ($collected | ConvertTo-Json -Depth 5)
    }

    It 'apply_fails_on_hash_mismatch' {
        Import-FakeSigned $r
        Write-Fake $r.App 'a different build'
        $received = Get-OmaSha256 $r.App
        { Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App } |
            Should -Throw -ExpectedMessage "hash mismatch for oma-app.exe: collected $unsignedApp, received $received"
        Get-OmaSha256 $r.App | Should -BeExactly $received
    }

    It 'apply_fails_when_signed_copy_missing' {
        { Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App } |
            Should -Throw -ExpectedMessage 'signed copy missing for oma-app.exe'
        Import-FakeSigned $r
        Remove-Item -LiteralPath (Join-Path $r.State 'signed\uninstall.exe')
        { Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path (New-Uninstaller) } |
            Should -Throw -ExpectedMessage 'signed copy missing for uninstall.exe'
        Get-OmaSha256 $r.App | Should -BeExactly $unsignedApp
    }

    It 'apply_rejects_tampered_signed_copy' {
        Import-FakeSigned $r
        Write-Fake (Join-Path $r.State 'signed\oma-app.exe') 'swapped after import'
        { Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App } |
            Should -Throw -ExpectedMessage '*signed copy of oma-app.exe*'
        Get-OmaSha256 $r.App | Should -BeExactly $unsignedApp
    }

    It 'apply_leaves_the_setup_untouched' {
        Import-FakeSigned $r
        Write-Fake $r.Setup 'setup of the second pass'
        $before = Get-OmaSha256 $r.Setup
        Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.SetupArg
        Get-OmaSha256 $r.Setup | Should -BeExactly $before
        $a = @((Read-Manifest $r).apply)
        $a[0].role | Should -Be 'setup'
        $a[0].sha256 | Should -BeExactly $before
        $a[0].after | Should -BeExactly $before
    }

    It 'refuses to collect or register once the signed files are imported' {
        Import-FakeSigned $r
        { Register-OmaService -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Service } | Should -Throw
        { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Plugins[0] } | Should -Throw
    }
}

Describe 'Import-OmaSignedFiles' {
    BeforeEach {
        $r = New-FakeRepo
        Initialize-Fake $r
        Invoke-CollectPass $r
    }

    It 'import_signed_rejects_extra_or_missing_files' {
        $missing = New-SignedDir $r @('oma-app.exe', 'oma-service.exe')
        { Import-OmaSignedFiles -StateRoot $r.State -From $missing } | Should -Throw -ExpectedMessage '*uninstall.exe*'
        $extra = New-SignedDir $r @('oma-app.exe', 'uninstall.exe', 'oma-service.exe', 'PawnIO_setup.exe')
        { Import-OmaSignedFiles -StateRoot $r.State -From $extra } | Should -Throw -ExpectedMessage '*PawnIO_setup.exe*'
        $nested = New-SignedDir $r
        New-Item -ItemType Directory (Join-Path $nested 'sub') | Out-Null
        { Import-OmaSignedFiles -StateRoot $r.State -From $nested } | Should -Throw -ExpectedMessage '*sub*'
        Get-ChildItem -LiteralPath (Join-Path $r.State 'signed') | Should -BeNullOrEmpty
        (Read-Manifest $r).signed.PSObject.Properties.Name | Should -BeNullOrEmpty

        Import-FakeSigned $r
        $s = (Read-Manifest $r).signed
        ($s.PSObject.Properties.Name | Sort-Object) | Should -Be @('oma-app.exe', 'oma-service.exe', 'uninstall.exe')
        $s.'oma-service.exe' | Should -BeExactly (Get-Sha 'signed oma-service.exe')
        Get-OmaSha256 (Join-Path $r.State 'signed\uninstall.exe') | Should -BeExactly (Get-Sha 'signed uninstall.exe')
        { Import-FakeSigned $r } | Should -Throw
    }
}

Describe 'Assert-OmaSigningPass' {
    BeforeEach {
        $r = New-FakeRepo
        Initialize-Fake $r
    }

    It 'check_fails_when_the_uninstaller_was_never_called' {
        Invoke-CollectPass $r -SkipUninstaller
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } |
            Should -Throw -ExpectedMessage '*uninstaller*'
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path (New-Uninstaller)
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } | Should -Not -Throw
    }

    It 'fails when the service was never registered or a plugin call is missing' {
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App
        foreach ($p in $r.Plugins | Select-Object -Skip 1) { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $p }
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path (New-Uninstaller)
        Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.SetupArg
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } |
            Should -Throw -ExpectedMessage '*service*'
        Register-OmaService -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Service
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } |
            Should -Throw -ExpectedMessage '*NSISdl.dll*'
    }

    It 'fails when the setup on disk is not the one the shim saw' {
        Invoke-CollectPass $r
        Write-Fake $r.Setup 'a stale setup from an older pass'
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } |
            Should -Throw -ExpectedMessage '*setup*'
    }

    It 'passes a complete apply pass and fails an incomplete one' {
        Invoke-CollectPass $r
        Import-FakeSigned $r
        Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass apply -ExpectedContext $r.Context } |
            Should -Throw -ExpectedMessage '*uninstaller*'
        foreach ($p in $r.Plugins) { Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $p }
        Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path (New-Uninstaller)
        Write-Fake $r.Setup 'setup of the second pass'
        Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.SetupArg
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass apply -ExpectedContext $r.Context } | Should -Not -Throw
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } | Should -Not -Throw
    }

    It 'check_fails_on_manifest_of_another_run' {
        Invoke-CollectPass $r
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext (New-Context -RunId '43') } |
            Should -Throw -ExpectedMessage '*runId*'
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext (New-Context -Commit ('b' * 40)) } |
            Should -Throw -ExpectedMessage '*commit*'
        $v = New-Context
        $v.Version = '0.3.1'
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $v } |
            Should -Throw -ExpectedMessage '*version*'
    }

    It 'check_uses_independent_run_context' {
        Invoke-CollectPass $r
        # A manifest rewritten to describe another run is not trusted: the context comes from the caller.
        $path = Join-Path $r.State 'manifest.json'
        $m = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
        $m.commit = 'c' * 40
        $m | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $path
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } |
            Should -Throw -ExpectedMessage '*commit*'
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect } |
            Should -Throw -ExpectedMessage '*run context*'

        # The shim calls compare the manifest with the context the config passes them, too.
        { Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.SetupArg } |
            Should -Throw -ExpectedMessage '*commit*'
        { Register-OmaService -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Service } |
            Should -Throw -ExpectedMessage '*commit*'

        # The script refuses check without an explicit context.
        $res = Invoke-ShimScript @('-Mode', 'check', '-Pass', 'collect', '-RepoRoot', $r.Root, '-StateRoot', $r.State)
        $res.ExitCode | Should -Not -Be 0
    }

    It 'outside GitHub Actions the explicit context is mandatory in every pass' {
        { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -Path $r.App } |
            Should -Throw -ExpectedMessage '*run context*'
        { Register-OmaService -StateRoot $r.State -Path $r.Service } |
            Should -Throw -ExpectedMessage '*run context*'
        @((Read-Manifest $r).collect).Count | Should -Be 0
        foreach ($mode in 'collect', 'apply', 'register-service') {
            $res = Invoke-ShimScript @('-Mode', $mode, '-StateRoot', $r.State, '-Path', $r.App)
            $res.ExitCode | Should -Not -Be 0
            $res.Stderr | Should -BeLike '*-Commit*'
        }
    }

    It 'check_verifies_repo_root_and_service_independently' {
        Invoke-CollectPass $r
        { Assert-OmaSigningPass -RepoRoot $r.Root.ToUpperInvariant() -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } |
            Should -Not -Throw
        $other = Join-Path (Split-Path $r.Root) 'other-repo'
        New-Item -ItemType Directory -Force $other | Out-Null
        { Assert-OmaSigningPass -RepoRoot $other -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } |
            Should -Throw -ExpectedMessage '*repoRoot*'
        { Assert-OmaSigningPass -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } |
            Should -Throw -ExpectedMessage '*repository root*'

        $path = Join-Path $r.State 'manifest.json'
        $original = Get-Content -Raw -LiteralPath $path
        $m = $original | ConvertFrom-Json
        $m.expected.service = Join-Path $r.Root 'target\release\oma-service.exe'
        $m | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $path
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } |
            Should -Throw -ExpectedMessage '*service*'

        # A manifest moved wholesale to another root (repoRoot and every expected path) is refused too.
        $m = $original.Replace(($r.Root | ConvertTo-Json).Trim('"'), ($other | ConvertTo-Json).Trim('"')) | ConvertFrom-Json
        $m.repoRoot | Should -Be $other
        $m | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $path
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } |
            Should -Throw -ExpectedMessage '*repoRoot*'
    }
}

Describe 'GitHub Actions context' {
    BeforeEach {
        $r = New-FakeRepo
        Initialize-Fake $r
        $env:GITHUB_ACTIONS = 'true'
        $env:GITHUB_SHA = $r.Context.Commit
        $env:GITHUB_RUN_ID = $r.Context.RunId
        $env:GITHUB_RUN_ATTEMPT = $r.Context.RunAttempt
    }

    AfterEach {
        foreach ($n in $githubVars) { Remove-Item "Env:$n" -ErrorAction SilentlyContinue }
    }

    It 'passes when the manifest matches the GitHub run' {
        Invoke-CollectPass $r
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass collect -ExpectedContext $r.Context } | Should -Not -Throw
        Import-FakeSigned $r
        Invoke-ApplyPass $r
        { Assert-OmaSigningPass -RepoRoot $r.Root -StateRoot $r.State -Pass apply -ExpectedContext $r.Context } | Should -Not -Throw
    }

    It 'fails every pass when the GitHub run differs from the manifest' -ForEach @(
        @{ Var = 'GITHUB_SHA'; Value = 'b' * 40 },
        @{ Var = 'GITHUB_RUN_ID'; Value = '43' },
        @{ Var = 'GITHUB_RUN_ATTEMPT'; Value = '2' }
    ) {
        Set-Item "Env:$Var" $Value
        { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App } |
            Should -Throw -ExpectedMessage "*$Var*"
        { Register-OmaService -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Service } |
            Should -Throw -ExpectedMessage "*$Var*"
        { Invoke-OmaSignShim -Mode apply -StateRoot $r.State -ExpectedContext $r.Context -Path $r.SetupArg } |
            Should -Throw -ExpectedMessage "*$Var*"
        @((Read-Manifest $r).collect).Count | Should -Be 0
        # The same through the script, which inherits the environment like the Tauri/makensis children.
        $res = Invoke-ShimScript @('-Mode', 'collect', '-StateRoot', $r.State, '-Path', $r.App,
            '-Commit', $r.Context.Commit, '-Version', '0.3.0', '-RunId', '42', '-RunAttempt', '1')
        $res.ExitCode | Should -Not -Be 0
        $res.Stderr | Should -BeLike "*$Var*"
    }

    It 'fails when a GitHub variable is missing in Actions' -ForEach @(
        @{ Var = 'GITHUB_SHA' }, @{ Var = 'GITHUB_RUN_ID' }, @{ Var = 'GITHUB_RUN_ATTEMPT' }
    ) {
        Remove-Item "Env:$Var"
        { Invoke-OmaSignShim -Mode collect -StateRoot $r.State -ExpectedContext $r.Context -Path $r.App } |
            Should -Throw -ExpectedMessage "*$Var*"
        { Register-OmaService -StateRoot $r.State -ExpectedContext $r.Context -Path $r.Service } |
            Should -Throw -ExpectedMessage "*$Var*"
    }
}

Describe 'sign-shim.ps1' {
    It 'works_from_another_directory_with_spaces' {
        $r = New-FakeRepo 'repo con spazi'
        $elsewhere = Join-Path $TestDrive 'another cwd'
        New-Item -ItemType Directory -Force $elsewhere | Out-Null
        $ctxArgs = @('-Commit', $r.Context.Commit, '-Version', '0.3.0', '-RunId', '42', '-RunAttempt', '1')

        $res = Invoke-ShimScript (@('-Mode', 'init', '-RepoRoot', $r.Root, '-StateRoot', $r.State) + $ctxArgs) $elsewhere
        $res.ExitCode | Should -Be 0 -Because $res.Stderr

        # Call the shim exactly as the generated config tells Tauri to, from app\src-tauri-like cwds.
        $sc = (Get-Content -Raw -LiteralPath (Join-Path $r.State 'tauri.sign.collect.json') | ConvertFrom-Json).bundle.windows.signCommand
        $files = @($r.App) + $r.Plugins + @((New-Uninstaller), $r.SetupArg)
        foreach ($f in $files) {
            $a = @($sc.args | ForEach-Object { if ($_ -eq '%1') { $f } else { $_ } })
            $x = Invoke-OmaNative -FilePath $sc.cmd -ArgumentList $a -WorkingDirectory $elsewhere -AllowFailure
            $x.ExitCode | Should -Be 0 -Because "$f -> $($x.Stderr)"
        }
        $res = Invoke-ShimScript (@('-Mode', 'register-service', '-StateRoot', $r.State, '-Path', $r.Service) + $ctxArgs) $elsewhere
        $res.ExitCode | Should -Be 0 -Because $res.Stderr
        $res = Invoke-ShimScript (@('-Mode', 'check', '-Pass', 'collect', '-RepoRoot', $r.Root, '-StateRoot', $r.State) + $ctxArgs) $elsewhere
        $res.ExitCode | Should -Be 0 -Because $res.Stderr

        $from = New-SignedDir $r
        $res = Invoke-ShimScript @('-Mode', 'import-signed', '-StateRoot', $r.State, '-From', $from) $elsewhere
        $res.ExitCode | Should -Be 0 -Because $res.Stderr
    }

    It 'script_exit_codes' {
        $r = New-FakeRepo
        $ctxArgs = @('-Commit', $r.Context.Commit, '-Version', '0.3.0', '-RunId', '42', '-RunAttempt', '1')
        $res = Invoke-ShimScript (@('-Mode', 'init', '-RepoRoot', $r.Root, '-StateRoot', $r.State) + $ctxArgs)
        $res.ExitCode | Should -Be 0 -Because $res.Stderr

        $res = Invoke-ShimScript (@('-Mode', 'collect', '-StateRoot', $r.State, '-Path', $r.App) + $ctxArgs)
        $res.ExitCode | Should -Be 0 -Because $res.Stderr

        $odd = Join-Path $r.Root 'odd.exe'
        Write-Fake $odd 'x'
        $res = Invoke-ShimScript (@('-Mode', 'collect', '-StateRoot', $r.State, '-Path', $odd) + $ctxArgs)
        $res.ExitCode | Should -Not -Be 0
        $res.Stderr | Should -BeLike '*unexpected file passed to signCommand:*odd.exe*'

        $res = Invoke-ShimScript (@('-Mode', 'collect', '-StateRoot', $r.State, '-Path', $r.App) + $ctxArgs)
        $res.ExitCode | Should -Not -Be 0
        $res.Stderr | Should -BeLike '*duplicate app call*'

        # The pass is incomplete (no uninstaller, setup or service).
        $res = Invoke-ShimScript (@('-Mode', 'check', '-Pass', 'collect', '-RepoRoot', $r.Root, '-StateRoot', $r.State) + $ctxArgs)
        $res.ExitCode | Should -Not -Be 0
        $res.Stderr | Should -BeLike '*uninstaller*'

        $res = Invoke-ShimScript @('-Mode', 'bogus', '-StateRoot', $r.State)
        $res.ExitCode | Should -Not -Be 0

        $res = Invoke-ShimScript @('-Mode', 'init', '-RepoRoot', $r.Root, '-StateRoot', $r.Root)
        $res.ExitCode | Should -Not -Be 0
    }
}
