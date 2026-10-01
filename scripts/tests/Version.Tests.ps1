#Requires -Version 7
# Pester 5 tests for scripts/lib/OmaVersion.psm1 (spec M6a §6). Every test works on a copy of
# scripts/tests/fixtures/version under $TestDrive, with a fake Cargo and a fake Git: nothing
# here touches the real repository or the network.
#   Import-Module Pester -RequiredVersion 5.7.1
#   Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI

BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaCommon.psm1') -Force -ErrorAction Stop
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaVersion.psm1') -Force -ErrorAction Stop
    $script:fixtures = (Resolve-Path (Join-Path $PSScriptRoot 'fixtures\version')).Path
    $script:utf8 = [Text.UTF8Encoding]::new($false)
    $script:sha = 'a' * 40
    $script:otherSha = 'b' * 40
    $script:files = 'Cargo.toml', 'Cargo.lock', 'app/package.json', 'app/src-tauri/tauri.conf.json', 'README.md', 'README.it.md'

    function script:New-Repo {
        $root = Join-Path $TestDrive ([guid]::NewGuid().ToString('N'))
        Copy-Item -LiteralPath $fixtures -Destination $root -Recurse
        $root
    }

    function script:Edit-File([string]$Root, [string]$Rel, [string]$From, [string]$To) {
        $p = Join-Path $Root $Rel
        $t = $utf8.GetString([IO.File]::ReadAllBytes($p))
        if (-not $t.Contains($From)) { throw "fixture edit: '$From' not found in $Rel" }
        [IO.File]::WriteAllBytes($p, $utf8.GetBytes($t.Replace($From, $To)))
    }

    function script:Read-All([string]$Root) {
        $files | ForEach-Object { , [IO.File]::ReadAllBytes((Join-Path $Root $_)) }
    }

    function script:Assert-Untouched([string]$Root, $Before) {
        $after = Read-All $Root
        for ($i = 0; $i -lt $files.Count; $i++) {
            [Convert]::ToHexString($after[$i]) | Should -BeExactly ([Convert]::ToHexString($Before[$i])) -Because $files[$i]
        }
    }

    # Fake cargo: `metadata` answers from $state, `update` rewrites the workspace packages of
    # Cargo.lock like cargo would (then runs $state.OnUpdate) or fails with $state.UpdateExit.
    function script:New-FakeCargo {
        $state = @{ MetadataExit = 0; UpdateExit = 0; OnUpdate = $null; FailMetadataAfterUpdate = $false; Calls = [Collections.Generic.List[string]]::new() }
        $state.Block = {
            param([string[]]$Arguments, [string]$WorkingDirectory)
            $state.Calls.Add(($Arguments -join ' '))
            $fail = { param($c, $e) [pscustomobject]@{ Stdout = ''; Stderr = $e; ExitCode = $c } }
            if ($Arguments[0] -eq 'metadata') {
                if ($state.MetadataExit -ne 0) { return & $fail $state.MetadataExit 'lock file needs to be updated' }
                if ($state.FailMetadataAfterUpdate -and $state.Calls.Contains('update --workspace --offline')) { return & $fail 101 'stale lock' }
                return [pscustomobject]@{ Stdout = '{"packages":[{"name":"oma-app"},{"name":"oma-core"}]}'; Stderr = ''; ExitCode = 0 }
            }
            if ($Arguments[0] -eq 'update') {
                if ($state.UpdateExit -ne 0) { return & $fail $state.UpdateExit 'no matching package' }
                $lock = Join-Path $WorkingDirectory 'Cargo.lock'
                $toml = [IO.File]::ReadAllText((Join-Path $WorkingDirectory 'Cargo.toml'))
                $new = [regex]::Match($toml, 'version = "(\d+\.\d+\.\d+)"').Groups[1].Value
                $text = [IO.File]::ReadAllText($lock)
                $text = [regex]::Replace($text, '(name = "oma-(?:app|core)"\nversion = ")[^"]*(")', "`${1}$new`$2")
                if ($state.OnUpdate) { $text = & $state.OnUpdate $text }
                [IO.File]::WriteAllBytes($lock, [Text.UTF8Encoding]::new($false).GetBytes($text))
                return [pscustomobject]@{ Stdout = ''; Stderr = ''; ExitCode = 0 }
            }
            & $fail 101 "unexpected cargo $Arguments"
        }.GetNewClosure()
        $state
    }

    # Fake git: rev-parse HEAD / refs/tags/<tag>^{commit}, fetch, merge-base --is-ancestor.
    function script:New-FakeGit {
        $state = @{ FetchExit = 0; Head = $sha; Tag = $sha; TagExit = 0; AncestorExit = 0; Calls = [Collections.Generic.List[string]]::new() }
        $state.Block = {
            param([string[]]$Arguments, [string]$WorkingDirectory)
            $state.Calls.Add(($Arguments -join ' '))
            $r = { param($o, $c, $e) [pscustomobject]@{ Stdout = $o; Stderr = $e; ExitCode = $c } }
            switch ($Arguments[0]) {
                'fetch' { return & $r '' $state.FetchExit 'network down' }
                'rev-parse' {
                    if ($Arguments[-1] -eq 'HEAD') { return & $r "$($state.Head)`n" 0 '' }
                    return & $r "$($state.Tag)`n" $state.TagExit 'unknown tag'
                }
                'merge-base' { return & $r '' $state.AncestorExit 'merge-base failed' }
            }
            & $r '' 128 "unexpected git $Arguments"
        }.GetNewClosure()
        $state
    }
}

Describe 'ConvertTo-OmaVersion' {
    It 'accepts_canonical' {
        (ConvertTo-OmaVersion '0.3.0') | Should -Be ([version]'0.3.0')
        $v = ConvertTo-OmaVersion '65535.0.1'
        $v.Major | Should -Be 65535
        $v.Build | Should -Be 1
    }
    It 'rejects_leading_zeros' { { ConvertTo-OmaVersion '0.03.0' } | Should -Throw }
    It 'rejects_above_pe_limit' { { ConvertTo-OmaVersion '0.65536.0' } | Should -Throw }
    It 'rejects_prerelease' { { ConvertTo-OmaVersion '0.3.0-rc.1' } | Should -Throw }
    It 'rejects two or four components' {
        { ConvertTo-OmaVersion '0.3' } | Should -Throw
        { ConvertTo-OmaVersion '0.3.0.1' } | Should -Throw
    }
}

Describe 'Get-OmaVersionSources' {
    It 'reads the five files' {
        $s = @(Get-OmaVersionSources (New-Repo))
        $s.Count | Should -Be 5
        ($s.Version | Select-Object -Unique) | Should -Be '0.2.0'
    }
    It 'reads_the_italian_readme' {
        $root = New-Repo
        Edit-File $root 'README.it.md' '(versione 0.2.0)' '(versione 0.9.1)'
        (Get-OmaVersionSources $root | Where-Object File -eq 'README.it.md').Version | Should -Be '0.9.1'
    }
    It 'ignores the nested nsis version in tauri.conf.json' {
        (Get-OmaVersionSources (New-Repo) | Where-Object File -like '*tauri.conf.json').Version | Should -Be '0.2.0'
    }
}

Describe 'Test-OmaVersionConsistency' {
    BeforeEach {
        $script:root = New-Repo
        $script:cargo = New-FakeCargo
        $script:git = New-FakeGit
    }

    It 'all_aligned_has_no_problems' {
        @(Test-OmaVersionConsistency -RepoRoot $root -Cargo $cargo.Block -Git $git.Block).Count | Should -Be 0
    }

    It 'one_file_different_is_reported (<rel>)' -ForEach @(
        @{ rel = 'Cargo.toml'; from = 'version = "0.2.0"'; to = 'version = "0.3.0"' }
        @{ rel = 'app/package.json'; from = '"version": "0.2.0"'; to = '"version": "0.3.0"' }
        @{ rel = 'app/src-tauri/tauri.conf.json'; from = '"version": "0.2.0"'; to = '"version": "0.3.0"' }
        @{ rel = 'README.md'; from = '(version 0.2.0)'; to = '(version 0.3.0)' }
        @{ rel = 'README.it.md'; from = '(versione 0.2.0)'; to = '(versione 0.3.0)' }
    ) {
        Edit-File $root $rel $from $to
        $p = @(Test-OmaVersionConsistency -RepoRoot $root -Cargo $cargo.Block -Git $git.Block)
        $p.Count | Should -Be 1
        $p[0] | Should -BeLike "$rel*"
    }

    It 'missing_field_is_reported' {
        Edit-File $root 'README.md' '(version 0.2.0)' '(release 0.2.0)'
        $p = @(Test-OmaVersionConsistency -RepoRoot $root -Cargo $cargo.Block -Git $git.Block)
        $p.Count | Should -Be 1
        $p[0] | Should -BeLike 'README.md*missing*'
    }

    It 'a missing file is reported as a missing field' {
        Remove-Item (Join-Path $root 'app/package.json')
        $p = @(Test-OmaVersionConsistency -RepoRoot $root -Cargo $cargo.Block -Git $git.Block)
        $p.Count | Should -Be 1
        $p[0] | Should -BeLike 'app/package.json*missing*'
    }

    It 'every_problem_is_listed' {
        Edit-File $root 'README.md' '(version 0.2.0)' '(version 0.3.0)'
        Edit-File $root 'app/package.json' '"version": "0.2.0"' '"version": "0.1.0"'
        @(Test-OmaVersionConsistency -RepoRoot $root -Cargo $cargo.Block -Git $git.Block).Count | Should -Be 2
    }

    It 'a non canonical version is reported even when every file agrees' {
        Edit-File $root 'README.md' '(version 0.2.0)' '(version 0.02.0)'
        Edit-File $root 'README.it.md' '(versione 0.2.0)' '(versione 0.02.0)'
        Edit-File $root 'Cargo.toml' 'version = "0.2.0"' 'version = "0.02.0"'
        Edit-File $root 'app/package.json' '"version": "0.2.0"' '"version": "0.02.0"'
        Edit-File $root 'app/src-tauri/tauri.conf.json' '"version": "0.2.0"' '"version": "0.02.0"'
        $p = @(Test-OmaVersionConsistency -RepoRoot $root -Cargo $cargo.Block -Git $git.Block)
        $p.Count | Should -Be 5
        $p[0] | Should -BeLike '*0.02.0*'
    }

    It 'locked_metadata_failure_is_reported' {
        $cargo.MetadataExit = 101
        $p = @(Test-OmaVersionConsistency -RepoRoot $root -Cargo $cargo.Block -Git $git.Block)
        $p.Count | Should -Be 1
        $p[0] | Should -BeLike '*Cargo.lock*'
        $cargo.Calls[0] | Should -Be 'metadata --locked --format-version 1'
    }

    It 'stale_lockfile_is_reported_by_real_cargo' {
        # Real cargo on a dependency-free workspace (no network): with --no-deps cargo skips the
        # resolution and accepts a stale Cargo.lock, so the fake above cannot prove this.
        $toml = "[workspace]`nresolver = `"2`"`nmembers = [`"crates/oma-core`"]`n`n[workspace.package]`nversion = `"0.2.0`"`nedition = `"2021`"`n"
        [IO.File]::WriteAllBytes((Join-Path $root 'Cargo.toml'), $utf8.GetBytes($toml))
        $crate = Join-Path $root 'crates/oma-core'
        $null = New-Item -ItemType Directory -Path (Join-Path $crate 'src')
        [IO.File]::WriteAllBytes((Join-Path $crate 'Cargo.toml'), $utf8.GetBytes("[package]`nname = `"oma-core`"`nversion.workspace = true`nedition.workspace = true`n"))
        [IO.File]::WriteAllBytes((Join-Path $crate 'src/lib.rs'), [byte[]]@())
        Remove-Item (Join-Path $root 'Cargo.lock')
        $null = Invoke-OmaNative -FilePath 'cargo' -ArgumentList @('generate-lockfile', '--offline') -WorkingDirectory $root

        @(Test-OmaVersionConsistency -RepoRoot $root -Git $git.Block).Count | Should -Be 0

        Edit-File $root 'Cargo.lock' 'version = "0.2.0"' 'version = "0.1.0"'
        $p = @(Test-OmaVersionConsistency -RepoRoot $root -Git $git.Block)
        $p.Count | Should -Be 1
        $p[0] | Should -BeLike '*Cargo.lock*'
    }

    It 'metadata failure does not hide a version mismatch' {
        $cargo.MetadataExit = 101
        Edit-File $root 'README.md' '(version 0.2.0)' '(version 0.3.0)'
        @(Test-OmaVersionConsistency -RepoRoot $root -Cargo $cargo.Block -Git $git.Block).Count | Should -Be 2
    }

    Context 'with a tag' {
        It 'accepts a matching tag on HEAD reachable from origin/main' {
            @(Test-OmaVersionConsistency -RepoRoot $root -Tag 'v0.2.0' -Cargo $cargo.Block -Git $git.Block -ExpectedSha $sha).Count | Should -Be 0
            ($git.Calls | Where-Object { $_ -like 'fetch*' }) | Should -Be 'fetch origin +refs/heads/main:refs/remotes/origin/main'
            ($git.Calls | Where-Object { $_ -like 'merge-base*' }) | Should -Be "merge-base --is-ancestor $sha origin/main"
        }

        It 'tag_mismatch_fails' {
            $p = @(Test-OmaVersionConsistency -RepoRoot $root -Tag 'v0.3.0' -Cargo $cargo.Block -Git $git.Block)
            $p.Count | Should -Be 1
            $p[0] | Should -BeLike '*v0.3.0*0.2.0*'
        }

        It 'non_numeric_tag_fails (<tag>)' -ForEach @(@{ tag = 'v0.3' }, @{ tag = 'v0.3.0-rc1' }, @{ tag = '0.2.0' }, @{ tag = 'v01.2.3' }) {
            $p = @(Test-OmaVersionConsistency -RepoRoot $root -Tag $tag -Cargo $cargo.Block -Git $git.Block)
            $p.Count | Should -BeGreaterThan 0
            $p[0] | Should -BeLike "*$tag*"
        }

        It 'annotated_tag_resolves_to_its_commit' {
            # An annotated tag object has its own SHA; git returns the commit through ^{commit},
            # so the script must ask for that form and compare the commit with HEAD.
            @(Test-OmaVersionConsistency -RepoRoot $root -Tag 'v0.2.0' -Cargo $cargo.Block -Git $git.Block).Count | Should -Be 0
            ($git.Calls | Where-Object { $_ -like 'rev-parse*' -and $_ -notlike '*HEAD' }) | Should -Be 'rev-parse --verify refs/tags/v0.2.0^{commit}'
        }

        It 'tag_not_head_fails' {
            $git.Tag = $otherSha
            $p = @(Test-OmaVersionConsistency -RepoRoot $root -Tag 'v0.2.0' -Cargo $cargo.Block -Git $git.Block)
            $p.Count | Should -Be 1
            $p[0] | Should -BeLike '*HEAD*'
        }

        It 'tag_not_on_main_fails' {
            $git.AncestorExit = 1
            $p = @(Test-OmaVersionConsistency -RepoRoot $root -Tag 'v0.2.0' -Cargo $cargo.Block -Git $git.Block)
            $p.Count | Should -Be 1
            $p[0] | Should -BeLike '*not reachable*origin/main*'
        }

        It 'reports an operational git error (exit above 1) differently from a false predicate' {
            $git.AncestorExit = 128
            $p = @(Test-OmaVersionConsistency -RepoRoot $root -Tag 'v0.2.0' -Cargo $cargo.Block -Git $git.Block)
            $p.Count | Should -Be 1
            $p[0] | Should -BeLike '*merge-base failed*'
            $p[0] | Should -Not -BeLike '*not reachable*'
        }

        It 'a failed fetch is reported and the other checks still run' {
            $git.FetchExit = 128
            $git.Tag = $otherSha
            $p = @(Test-OmaVersionConsistency -RepoRoot $root -Tag 'v0.2.0' -Cargo $cargo.Block -Git $git.Block)
            $p.Count | Should -Be 2
            @($p -like '*fetch*').Count | Should -Be 1
            @($p -like '*HEAD*').Count | Should -Be 1
        }

        It 'requires HEAD and the tag to match the run SHA' {
            $p = @(Test-OmaVersionConsistency -RepoRoot $root -Tag 'v0.2.0' -Cargo $cargo.Block -Git $git.Block -ExpectedSha $otherSha)
            $p.Count | Should -BeGreaterThan 0
            ($p -join "`n") | Should -BeLike "*$otherSha*"
        }

        It 'reports an unresolvable tag' {
            $git.TagExit = 128
            $p = @(Test-OmaVersionConsistency -RepoRoot $root -Tag 'v0.2.0' -Cargo $cargo.Block -Git $git.Block)
            $p.Count | Should -Be 1
            $p[0] | Should -BeLike '*v0.2.0*'
        }
    }
}

Describe 'Set-OmaVersion' {
    BeforeEach {
        $script:root = New-Repo
        $script:cargo = New-FakeCargo
        $script:git = New-FakeGit
    }

    It 'bump_updates_every_file_with_lf' {
        Set-OmaVersion -RepoRoot $root -Version '0.3.0' -Cargo $cargo.Block -Git $git.Block
        @(Test-OmaVersionConsistency -RepoRoot $root -Cargo $cargo.Block -Git $git.Block).Count | Should -Be 0
        (Get-OmaVersionSources $root).Version | Select-Object -Unique | Should -Be '0.3.0'
        foreach ($f in $files) {
            [IO.File]::ReadAllBytes((Join-Path $root $f)) | Should -Not -Contain 13
        }
        # Only the version substring changed: the nested nsis version and the dependencies stay.
        (Get-Content -Raw (Join-Path $root 'app/src-tauri/tauri.conf.json')) | Should -BeLike '*"version": "1.0.0"*'
        (Get-Content -Raw (Join-Path $root 'Cargo.lock')) | Should -BeLike '*name = "serde"*version = "1.0.228"*'
        $cargo.Calls | Should -Contain 'update --workspace --offline'
    }

    It 'bump_rejects_downgrade_and_same (<v>)' -ForEach @(@{ v = '0.1.9' }, @{ v = '0.2.0' }) {
        $before = Read-All $root
        { Set-OmaVersion -RepoRoot $root -Version $v -Cargo $cargo.Block -Git $git.Block } | Should -Throw
        Assert-Untouched $root $before
        $cargo.Calls | Should -Not -Contain 'update --workspace --offline'
    }

    It 'bump_compares_numerically' {
        Set-OmaVersion -RepoRoot $root -Version '0.9.0' -Cargo $cargo.Block -Git $git.Block
        Set-OmaVersion -RepoRoot $root -Version '0.10.0' -Cargo $cargo.Block -Git $git.Block
        (Get-OmaVersionSources $root).Version | Select-Object -Unique | Should -Be '0.10.0'
        { Set-OmaVersion -RepoRoot $root -Version '0.9.5' -Cargo $cargo.Block -Git $git.Block } | Should -Throw
    }

    It 'rejects a non canonical new version' {
        { Set-OmaVersion -RepoRoot $root -Version '0.3.0-rc.1' -Cargo $cargo.Block -Git $git.Block } | Should -Throw
    }

    It 'refuses to start from an unaligned tree' {
        Edit-File $root 'README.md' '(version 0.2.0)' '(version 0.1.0)'
        $before = Read-All $root
        { Set-OmaVersion -RepoRoot $root -Version '0.3.0' -Cargo $cargo.Block -Git $git.Block } | Should -Throw
        Assert-Untouched $root $before
    }

    It 'bump_rolls_back_on_cargo_failure' {
        $cargo.UpdateExit = 101
        $before = Read-All $root
        { Set-OmaVersion -RepoRoot $root -Version '0.3.0' -Cargo $cargo.Block -Git $git.Block } | Should -Throw
        Assert-Untouched $root $before
    }

    It 'rolls back when the final check fails' {
        $cargo.FailMetadataAfterUpdate = $true
        $before = Read-All $root
        { Set-OmaVersion -RepoRoot $root -Version '0.3.0' -Cargo $cargo.Block -Git $git.Block } | Should -Throw
        Assert-Untouched $root $before
    }

    It 'bump_rejects_unrelated_lock_changes (<name>)' -ForEach @(
        @{ name = 'external dependency update'; edit = { param($t) $t.Replace('version = "1.0.228"', 'version = "1.0.229"') } }
        @{ name = 'changed checksum'; edit = { param($t) $t.Replace('3333333333333333333333333333333333333333333333333333333333333333', ('4' * 64)) } }
        @{ name = 'new package'; edit = { param($t) $t + "`n[[package]]`nname = `"evil`"`nversion = `"1.0.0`"`n" } }
        @{ name = 'changed source'; edit = { param($t) $t.Replace('source = "registry+https://github.com/rust-lang/crates.io-index"' + "`n" + 'checksum = "3333', 'source = "git+https://example.com/serde"' + "`n" + 'checksum = "3333') } }
        @{ name = 'changed external dependency list'; edit = { param($t) $t.Replace(' "foo 1.0.0",', ' "foo 2.0.0",') } }
        @{ name = 'renamed package'; edit = { param($t) $t.Replace('name = "serde"', 'name = "serde2"') } }
        @{ name = 'local version set to a third value'; edit = { param($t) $t.Replace('name = "oma-core"' + "`n" + 'version = "0.3.0"', 'name = "oma-core"' + "`n" + 'version = "0.7.0"') } }
    ) {
        $cargo.OnUpdate = $edit
        $before = Read-All $root
        { Set-OmaVersion -RepoRoot $root -Version '0.3.0' -Cargo $cargo.Block -Git $git.Block } | Should -Throw
        Assert-Untouched $root $before
    }

    It 'allows internal workspace references to change' {
        # A local reference that becomes disambiguated ("oma-core 0.3.0") after the bump.
        $cargo.OnUpdate = { param($t) $t.Replace(' "oma-core",', ' "oma-core 0.3.0",') }
        Set-OmaVersion -RepoRoot $root -Version '0.3.0' -Cargo $cargo.Block -Git $git.Block
        (Get-OmaVersionSources $root).Version | Select-Object -Unique | Should -Be '0.3.0'
    }
}
