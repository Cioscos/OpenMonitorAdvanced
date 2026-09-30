#Requires -Version 7
# Pester 5 tests for scripts/lib/OmaRelease.psm1, scripts/release-preflight.ps1 and
# scripts/publish-draft.ps1 (spec M6a §4, §5.3; plan L3, L4, L7). GitHub is always a fake:
# the -Gh adapter is a scriptblock that records its calls and answers from prepared state; the
# entry-point test of the preflight puts a fake gh.cmd first on PATH. Nothing here writes to
# GitHub or reads it.
#   Import-Module Pester -RequiredVersion 5.7.1
#   Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI

BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaRelease.psm1') -Force -ErrorAction Stop
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaReleaseNotes.psm1') -Force -ErrorAction Stop
    $script:template = (Resolve-Path (Join-Path $PSScriptRoot '..\..\.github\release-notes-template.md')).Path
    $script:preflightScript = (Resolve-Path (Join-Path $PSScriptRoot '..\release-preflight.ps1')).Path
    $script:publishScript = (Resolve-Path (Join-Path $PSScriptRoot '..\publish-draft.ps1')).Path
    $script:utf8 = [Text.UTF8Encoding]::new($false)
    $script:repo = 'example/oma'
    $script:sha = 'a' * 40
    $script:attribution = 'Free code signing provided by SignPath.io, certificate by SignPath Foundation'
    $script:unsignedLine = 'The installer is not code-signed yet, so Windows SmartScreen may warn you: choose *More info* → *Run anyway*.'
    $script:mismatch = 'remote assets do not match; do not publish this draft, re-run the workflow'

    function script:Result([string]$Stdout = '', [int]$ExitCode = 0, [string]$Stderr = '') {
        [pscustomobject]@{ Stdout = $Stdout; Stderr = $Stderr; ExitCode = $ExitCode }
    }

    # --- CI gate fixtures (plan L7) ----------------------------------------------------------

    function script:New-Run([long]$Id = 1, [int]$Number = 1, [int]$Attempt = 1, [string]$Status = 'completed',
        [string]$Conclusion = 'success', [string]$Event = 'push', [string]$Branch = 'main', [string]$HeadSha = $sha,
        [string]$HeadRepo = $repo) {
        [ordered]@{
            id = $Id; run_number = $Number; run_attempt = $Attempt; status = $Status; conclusion = $Conclusion
            event = $Event; head_branch = $Branch; head_sha = $HeadSha; path = '.github/workflows/ci.yml'
            repository = @{ full_name = $repo }; head_repository = @{ full_name = $HeadRepo }
        }
    }

    function script:New-Job([string]$Name, [int]$Attempt = 1, [string]$Conclusion = 'success', [string]$Status = 'completed') {
        [ordered]@{ name = $Name; run_attempt = $Attempt; status = $Status; conclusion = $Conclusion }
    }

    function script:All-Jobs([int]$Attempt = 1) {
        foreach ($n in 'checks', 'service', 'installer', 'scripts', 'actionlint') { New-Job $n $Attempt }
    }

    # Fake gh for the CI gate. RunPages / JobPages are lists of pages, answered as `--slurp` would
    # (a JSON array of page objects); JobPages is keyed by "<run id>/<attempt>".
    function script:New-FakeCi {
        param($RunPages = @(, @(New-Run)), [hashtable]$JobPages = @{ '1/1' = @(, @(All-Jobs)) },
            [int]$RunsExit = 0, $RunsStdout = $null)
        # GetNewClosure does not see the test's script: functions and variables: local copies.
        $mk = ${function:script:Result}; $digestOf = ${function:script:Get-Digest}; $utf8 = $script:utf8; $repo = $script:repo
        $state = @{ RunPages = $RunPages; JobPages = $JobPages; RunsExit = $RunsExit; RunsStdout = $RunsStdout
            Calls = [Collections.Generic.List[object]]::new() }
        $state.Block = {
            param([string[]]$Arguments)
            $state.Calls.Add(@($Arguments))
            $path = @($Arguments | Where-Object { $_ -like 'repos/*' })[0]
            if ($path -like '*/actions/workflows/ci.yml/runs') {
                if ($state.RunsExit -ne 0) { return & $mk '' $state.RunsExit 'gh: Bad credentials (HTTP 401)' }
                if ($null -ne $state.RunsStdout) { return & $mk $state.RunsStdout }
                $pages = @(foreach ($p in $state.RunPages) { @{ total_count = 0; workflow_runs = @($p) } })
                return & $mk (ConvertTo-Json -InputObject $pages -Depth 20 -Compress)
            }
            if ($path -match '/actions/runs/(\d+)/attempts/(\d+)/jobs$') {
                $key = "$($Matches[1])/$($Matches[2])"
                if (-not $state.JobPages.ContainsKey($key)) { return & $mk '' 1 'gh: Not Found (HTTP 404)' }
                $pages = @(foreach ($p in $state.JobPages[$key]) { @{ total_count = 0; jobs = @($p) } })
                return & $mk (ConvertTo-Json -InputObject $pages -Depth 20 -Compress)
            }
            & $mk '' 1 "unexpected gh $Arguments"
        }.GetNewClosure()
        $state
    }

    # --- release fixtures --------------------------------------------------------------------

    function script:New-Setup([string]$Version = '1.2.3', [string]$Content = 'installer bytes') {
        $dir = Join-Path $TestDrive ([guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        $setup = Join-Path $dir "OpenMonitor.Advanced_$($Version)_x64-setup.exe"
        [IO.File]::WriteAllBytes($setup, $utf8.GetBytes($Content))
        $sums = Join-Path $dir 'SHA256SUMS.txt'
        Write-OmaSha256Sums -Setup $setup -Out $sums
        [pscustomobject]@{ Setup = $setup; Sums = $sums }
    }

    function script:Get-Digest([byte[]]$Bytes) {
        'sha256:' + [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant()
    }

    # A fake GitHub release store behind gh. State: Exists, IsDraft, Body, Assets (name -> bytes,
    # state, digest). Knobs: FlipToPublishedAtView (the n-th `release view` sees isDraft=false),
    # NullDigest, UploadFailAfter (upload stores that many files then fails), ViewExit/ViewStderr,
    # ViewStdout (raw answer), CorruptUpload (stores other bytes for the setup).
    function script:New-FakeRelease {
        param([bool]$Exists = $false, [bool]$IsDraft = $true, [string]$Body = '', [string]$Tag = 'v1.2.3')
        # GetNewClosure does not see the test's script: functions and variables: local copies.
        $mk = ${function:script:Result}; $digestOf = ${function:script:Get-Digest}; $utf8 = $script:utf8; $repo = $script:repo
        $state = @{
            Exists = $Exists; IsDraft = $IsDraft; Body = $Body; Tag = $Tag; Assets = [ordered]@{}; Views = 0
            FlipToPublishedAtView = 0; NullDigest = $false; UploadFailAfter = -1; ViewExit = 0; ViewStderr = ''
            ViewStdout = $null; CorruptUpload = $false; NotesSeen = [Collections.Generic.List[string]]::new()
            Calls = [Collections.Generic.List[object]]::new()
        }
        $state.Block = {
            param([string[]]$Arguments)
            $state.Calls.Add(@($Arguments))
            $valueOf = { param($flag) $i = [array]::IndexOf($Arguments, $flag); if ($i -ge 0) { $Arguments[$i + 1] } }
            $store = {
                param([string]$File)
                $bytes = [IO.File]::ReadAllBytes($File)
                $name = [IO.Path]::GetFileName($File)
                if ($state.CorruptUpload -and $name -like '*-setup.exe') { $bytes = $utf8.GetBytes('tampered') }
                $state.Assets[$name] = @{ Bytes = $bytes; State = 'uploaded' }
            }
            if ($Arguments[0] -eq '--version') { return & $mk "gh version 2.40.0 (2023-12-01)`n" }
            if ($Arguments[0] -ne 'release') { return & $mk '' 1 "unexpected gh $Arguments" }
            switch ($Arguments[1]) {
                'view' {
                    $state.Views++
                    if ($state.FlipToPublishedAtView -gt 0 -and $state.Views -ge $state.FlipToPublishedAtView) { $state.IsDraft = $false }
                    if ($state.ViewExit -ne 0) { return & $mk '' $state.ViewExit $state.ViewStderr }
                    if ($null -ne $state.ViewStdout) { return & $mk $state.ViewStdout }
                    if (-not $state.Exists) { return & $mk '' 1 "release not found`n" }
                    $o = [ordered]@{}
                    foreach ($f in (& $valueOf '--json') -split ',') {
                        switch ($f) {
                            'isDraft' { $o.isDraft = $state.IsDraft }
                            'tagName' { $o.tagName = $state.Tag }
                            'body' { $o.body = $state.Body }
                            'assets' {
                                $o.assets = @(foreach ($k in $state.Assets.Keys) {
                                        $a = $state.Assets[$k]
                                        [ordered]@{ name = $k; size = $a.Bytes.Length; state = $a.State
                                            digest = if ($state.NullDigest) { $null } else { & $digestOf $a.Bytes }
                                            apiUrl = "https://api.github.com/repos/$repo/releases/assets/1" }
                                    })
                            }
                        }
                    }
                    return & $mk (ConvertTo-Json -InputObject $o -Depth 10 -Compress)
                }
                'create' {
                    $state.NotesSeen.Add([IO.File]::ReadAllText((& $valueOf '--notes-file')))
                    $state.Exists = $true; $state.IsDraft = $true
                    $state.Body = [IO.File]::ReadAllText((& $valueOf '--notes-file'))
                    foreach ($f in $Arguments | Where-Object { $_ -like '*\*' -and (Test-Path -LiteralPath $_ -PathType Leaf) -and $_ -ne (& $valueOf '--notes-file') }) { & $store $f }
                    return & $mk "https://github.com/$repo/releases/tag/untagged-1`n"
                }
                'edit' {
                    $state.NotesSeen.Add([IO.File]::ReadAllText((& $valueOf '--notes-file')))
                    $state.Body = [IO.File]::ReadAllText((& $valueOf '--notes-file'))
                    return & $mk ''
                }
                'upload' {
                    $files = @($Arguments | Where-Object { $_ -like '*\*' -and (Test-Path -LiteralPath $_ -PathType Leaf) })
                    $n = 0
                    foreach ($f in $files) {
                        if ($state.UploadFailAfter -ge 0 -and $n -ge $state.UploadFailAfter) {
                            return & $mk '' 1 'HTTP 502: Bad Gateway (upload)'
                        }
                        & $store $f; $n++
                    }
                    return & $mk ''
                }
                'download' {
                    $name = & $valueOf '--pattern'
                    $dir = & $valueOf '--dir'
                    if (-not $state.Assets.Contains($name)) { return & $mk '' 1 'no assets match the file pattern' }
                    [IO.File]::WriteAllBytes((Join-Path $dir $name), $state.Assets[$name].Bytes)
                    return & $mk ''
                }
            }
            & $mk '' 1 "unexpected gh $Arguments"
        }.GetNewClosure()
        $state
    }

    function script:Get-Verbs($State) { @($State.Calls | ForEach-Object { if ($_[0] -eq 'release') { $_[1] } else { $_[0] } }) }

    function script:Publish($State, $Files, [bool]$Signed = $true) {
        Publish-OmaDraft -Repo $repo -Tag 'v1.2.3' -Version '1.2.3' -Setup $Files.Setup -Sums $Files.Sums `
            -Signed $Signed -TemplatePath $template -Gh $State.Block
    }

    # --- preflight fixtures ------------------------------------------------------------------

    function script:New-Certificates([switch]$Complete) {
        $p = Join-Path $TestDrive ([guid]::NewGuid().ToString('N') + '.json')
        $root = Join-Path $TestDrive 'test-root.cer'
        [IO.File]::WriteAllBytes($root, [byte[]](1, 2, 3))
        $c = if ($Complete) {
            @{
                release = @{ subject = 'CN=SignPath Foundation, O=SignPath Foundation, C=US'; thumbprints = @('A' * 40) }
                test    = @{ subject = 'CN=Test'; thumbprints = @('B' * 40); rootThumbprints = @('C' * 40); rootCertificatePath = $root }
            }
        } else {
            @{ release = @{ subject = ''; thumbprints = @() }; test = @{ subject = ''; thumbprints = @(); rootThumbprints = @(); rootCertificatePath = '' } }
        }
        [IO.File]::WriteAllText($p, (ConvertTo-Json $c -Depth 5))
        $p
    }

    function script:Preflight {
        param([string]$EventName = 'push', [string]$Ref = 'refs/tags/v1.2.3', [string]$HasToken = 'false',
            [string]$OrganizationId = '', [string]$RequireSigning = '', [string]$Tag, [string]$Slug = '',
            [string]$Certificates, [string]$Output, $Gh)
        if (-not $Certificates) { $Certificates = New-Certificates }
        if (-not $Output) { $Output = Join-Path $TestDrive ([guid]::NewGuid().ToString('N') + '.out') }
        if (-not $Gh) {
            $ci = New-FakeCi
            $rel = New-FakeRelease
            $Gh = { param([string[]]$Arguments) if ($Arguments[0] -eq 'api') { & $ci.Block $Arguments } else { & $rel.Block $Arguments } }.GetNewClosure()
        }
        Invoke-OmaReleasePreflight -EventName $EventName -Ref $Ref -Sha $sha -Repo $repo -HasToken $HasToken `
            -OrganizationId $OrganizationId -RequireSigning $RequireSigning -Tag $Tag -SignPathProjectSlug $Slug `
            -CertificatesPath $Certificates -OutputPath $Output -Gh $Gh
    }
}

Describe 'Get-OmaSigningMode' {
    It 'disabled_when_both_missing' {
        Get-OmaSigningMode -HasToken $false -OrganizationId '' -RequireSigning '' | Should -Be 'disabled'
        Get-OmaSigningMode -HasToken $false -OrganizationId '  ' -RequireSigning 'false' | Should -Be 'disabled'
    }

    It 'enabled_when_both_present' {
        Get-OmaSigningMode -HasToken $true -OrganizationId 'org-1' -RequireSigning 'true' | Should -Be 'enabled'
    }

    It 'partial_configuration_fails' {
        { Get-OmaSigningMode -HasToken $true -OrganizationId '' -RequireSigning '' } | Should -Throw '*SIGNPATH_ORGANIZATION_ID*'
        { Get-OmaSigningMode -HasToken $false -OrganizationId 'org-1' -RequireSigning '' } | Should -Throw '*SIGNPATH_API_TOKEN*'
    }

    It 'required_signing_without_credentials_fails' {
        { Get-OmaSigningMode -HasToken $false -OrganizationId '' -RequireSigning 'true' } | Should -Throw '*REQUIRE_SIGNING*'
    }

    It 'invalid_boolean_configuration_fails' {
        foreach ($v in 'yes', '1', 'on', ' true') {
            { Get-OmaSigningMode -HasToken $true -OrganizationId 'org-1' -RequireSigning $v } | Should -Throw '*REQUIRE_SIGNING*'
        }
    }
}

Describe 'ConvertFrom-OmaBoolText' {
    It 'false_env_string_is_not_true' {
        ConvertFrom-OmaBoolText -Name 'HasToken' -Text 'false' | Should -BeFalse
        ConvertFrom-OmaBoolText -Name 'HasToken' -Text 'true' | Should -BeTrue
        ConvertFrom-OmaBoolText -Name 'RequireSigning' -Text '' -EmptyMeans $false | Should -BeFalse
        { ConvertFrom-OmaBoolText -Name 'HasToken' -Text '' } | Should -Throw '*HasToken*'
        { ConvertFrom-OmaBoolText -Name 'HasToken' -Text 'False ' } | Should -Throw '*HasToken*'
    }
}

Describe 'Assert-OmaRunRef' {
    It 'accepts_a_version_tag_push' {
        $r = Assert-OmaRunRef -EventName 'push' -Ref 'refs/tags/v1.2.3'
        $r.Tag | Should -Be 'v1.2.3'
        $r.Version | Should -Be '1.2.3'
    }

    It 'accepts_a_dispatch_from_main' {
        $r = Assert-OmaRunRef -EventName 'workflow_dispatch' -Ref 'refs/heads/main'
        $r.Tag | Should -BeNullOrEmpty
    }

    It 'dispatch_from_other_branch_fails' {
        { Assert-OmaRunRef -EventName 'workflow_dispatch' -Ref 'refs/heads/feature' } | Should -Throw '*refs/heads/main*'
        { Assert-OmaRunRef -EventName 'workflow_dispatch' -Ref 'refs/tags/v1.2.3' } | Should -Throw '*refs/heads/main*'
    }

    It 'push_of_a_branch_fails' {
        { Assert-OmaRunRef -EventName 'push' -Ref 'refs/heads/main' } | Should -Throw '*refs/tags/v*'
    }

    It 'rejects_a_malformed_tag' {
        foreach ($t in 'refs/tags/v1.2', 'refs/tags/v01.2.3', 'refs/tags/v1.2.3-rc.1', 'refs/tags/1.2.3', 'refs/tags/v1.65536.0', "refs/tags/v1.2.3`n") {
            { Assert-OmaRunRef -EventName 'push' -Ref $t } | Should -Throw '*tag*'
        }
    }

    It 'rejects_other_events' {
        { Assert-OmaRunRef -EventName 'pull_request' -Ref 'refs/heads/main' } | Should -Throw '*pull_request*'
    }
}

Describe 'Assert-OmaCiGreen' {
    It 'passes_with_every_job_green' {
        $ci = New-FakeCi
        { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Not -Throw
    }

    It 'queries_the_runs_of_ci_yml_for_the_sha' {
        $ci = New-FakeCi
        Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block
        $a = $ci.Calls[0]
        $a[0] | Should -Be 'api'
        $a | Should -Contain "repos/$repo/actions/workflows/ci.yml/runs"
        $a[([array]::IndexOf($a, '--method') + 1)] | Should -Be 'GET'
        $a | Should -Contain '--paginate'
        $a | Should -Contain '--slurp'
        $a | Should -Contain 'event=push'
        $a | Should -Contain 'branch=main'
        $a | Should -Contain "head_sha=$sha"
        $ci.Calls[1] | Should -Contain "repos/$repo/actions/runs/1/attempts/1/jobs"
        $ci.Calls[1] | Should -Contain '--paginate'
    }

    It 'ci_gate_requires_every_job' {
        $jobs = @(All-Jobs | Where-Object { $_.name -ne 'actionlint' }) + @(New-Job 'actionlint' 1 'skipped')
        $ci = New-FakeCi -JobPages @{ '1/1' = @(, $jobs) }
        { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Throw '*actionlint*skipped*'

        $missing = @(All-Jobs | Where-Object { $_.name -ne 'scripts' })
        $ci = New-FakeCi -JobPages @{ '1/1' = @(, $missing) }
        { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Throw '*scripts*'
    }

    It 'ci_gate_fails_without_completed_run' {
        $ci = New-FakeCi -RunPages @(, @())
        { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Throw "*no CI run*$sha*"

        $ci = New-FakeCi -RunPages @(, @(New-Run -Status 'in_progress' -Conclusion $null))
        { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Throw '*in_progress*'

        $ci = New-FakeCi -RunsExit 1
        { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Throw '*HTTP 401*'

        $ci = New-FakeCi -RunsStdout 'not json'
        { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Throw
    }

    It 'ci_gate_rejects_old_green_when_latest_attempt_failed' {
        # Run 7 was green; run 9 (same SHA, e.g. a later push of the same commit) is on its second
        # attempt, which failed. Attempt 1 of run 9 had green jobs: they must not count.
        $runs = @((New-Run -Id 7 -Number 7), (New-Run -Id 9 -Number 9 -Attempt 2 -Conclusion 'failure'))
        $ci = New-FakeCi -RunPages @(, $runs) -JobPages @{ '7/1' = @(, @(All-Jobs)); '9/1' = @(, @(All-Jobs)) }
        { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Throw '*failure*'

        # Latest run green overall but its latest attempt still has an old-attempt job list only.
        $ci = New-FakeCi -RunPages @(, @(New-Run -Id 9 -Number 9 -Attempt 2)) -JobPages @{ '9/2' = @(, @(All-Jobs 1)) }
        { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Throw '*checks*'
    }

    It 'ci_gate_requires_main_push_of_same_repo' {
        foreach ($run in @((New-Run -HeadRepo 'fork/oma'), (New-Run -Event 'pull_request'), (New-Run -Branch 'dev'), (New-Run -HeadSha ('b' * 40)))) {
            $ci = New-FakeCi -RunPages @(, @($run))
            { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Throw '*no CI run*'
        }
    }

    It 'ci_gate_reads_paginated_jobs' {
        $all = @(All-Jobs)
        $ci = New-FakeCi -RunPages @(@(New-Run -Id 2 -Number 2), @(New-Run -Id 5 -Number 5)) `
            -JobPages @{ '5/1' = @(@($all[0..2]), @($all[3..4])) }
        { Assert-OmaCiGreen -Repo $repo -Sha $sha -Gh $ci.Block } | Should -Not -Throw
        $ci.Calls[1] | Should -Contain "repos/$repo/actions/runs/5/attempts/1/jobs"
    }

    It 'rejects_bad_inputs' {
        $ci = New-FakeCi
        { Assert-OmaCiGreen -Repo 'x' -Sha $sha -Gh $ci.Block } | Should -Throw '*repository*'
        { Assert-OmaCiGreen -Repo $repo -Sha 'abc' -Gh $ci.Block } | Should -Throw '*SHA*'
        $ci.Calls.Count | Should -Be 0
    }
}

Describe 'Get-OmaReleaseState' {
    It 'reports_absent_draft_and_published' {
        $rel = New-FakeRelease
        Get-OmaReleaseState -Repo $repo -Tag 'v1.2.3' -Gh $rel.Block | Should -Be 'absent'
        $rel = New-FakeRelease -Exists $true
        Get-OmaReleaseState -Repo $repo -Tag 'v1.2.3' -Gh $rel.Block | Should -Be 'draft'
        $rel = New-FakeRelease -Exists $true -IsDraft $false
        Get-OmaReleaseState -Repo $repo -Tag 'v1.2.3' -Gh $rel.Block | Should -Be 'published'
        $rel.Calls[0][0..2] | Should -Be @('release', 'view', 'v1.2.3')
        $rel.Calls[0] | Should -Contain '--repo'
        $rel.Calls[0][([array]::IndexOf($rel.Calls[0], '--repo') + 1)] | Should -Be $repo
    }

    It 'api_failure_is_not_absent_release' {
        foreach ($err in 'HTTP 401: Bad credentials (https://api.github.com/graphql)', 'HTTP 403: Resource not accessible by integration',
            'HTTP 502: Bad Gateway', 'error connecting to api.github.com', 'release not found; HTTP 500') {
            $rel = New-FakeRelease
            $rel.ViewExit = 1; $rel.ViewStderr = $err
            { Get-OmaReleaseState -Repo $repo -Tag 'v1.2.3' -Gh $rel.Block } | Should -Throw "*$($err.Substring(0, 8))*"
        }
        $rel = New-FakeRelease -Exists $true
        $rel.ViewStdout = '{not json'
        { Get-OmaReleaseState -Repo $repo -Tag 'v1.2.3' -Gh $rel.Block } | Should -Throw
        $rel.ViewStdout = '{"tagName":"v1.2.3"}'
        { Get-OmaReleaseState -Repo $repo -Tag 'v1.2.3' -Gh $rel.Block } | Should -Throw '*isDraft*'
        $rel.ViewStdout = '{"tagName":"v9.9.9","isDraft":true}'
        { Get-OmaReleaseState -Repo $repo -Tag 'v1.2.3' -Gh $rel.Block } | Should -Throw '*v9.9.9*'
    }
}

Describe 'Get-OmaFinalSetupName' {
    It 'final_setup_name_uses_dots' {
        Get-OmaFinalSetupName -Version '1.2.3' | Should -BeExactly 'OpenMonitor.Advanced_1.2.3_x64-setup.exe'
        { Get-OmaFinalSetupName -Version '1.2' } | Should -Throw
    }
}

Describe 'Write-OmaSha256Sums' {
    It 'sums_format_is_sha256sum' {
        $f = New-Setup
        $bytes = [IO.File]::ReadAllBytes($f.Sums)
        $expected = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($utf8.GetBytes('installer bytes'))).ToLowerInvariant()
        $utf8.GetString($bytes) | Should -BeExactly "$expected  OpenMonitor.Advanced_1.2.3_x64-setup.exe`n"
        $bytes | Should -Not -Contain 13
        ($bytes[0] -eq 0xEF) | Should -BeFalse
    }
}

Describe 'Publish-OmaDraft' {
    It 'creates_a_new_draft' {
        $rel = New-FakeRelease
        $f = New-Setup
        Publish $rel $f -Signed $false
        $create = @($rel.Calls | Where-Object { $_[0] -eq 'release' -and $_[1] -eq 'create' })
        $create.Count | Should -Be 1
        $c = $create[0]
        $c[2] | Should -Be 'v1.2.3'
        $c | Should -Contain '--draft'
        $c | Should -Contain '--verify-tag'
        $c[([array]::IndexOf($c, '--title') + 1)] | Should -BeExactly 'OpenMonitor Advanced 1.2.3'
        $c[([array]::IndexOf($c, '--repo') + 1)] | Should -Be $repo
        $c[-2] | Should -Be $f.Setup
        $c[-1] | Should -Be $f.Sums
        $rel.NotesSeen[0] | Should -Match ([regex]::Escape($unsignedLine))
        $rel.NotesSeen[0] | Should -Not -Match "`r"
        $rel.Assets.Keys | Should -Be @('OpenMonitor.Advanced_1.2.3_x64-setup.exe', 'SHA256SUMS.txt')
        (Get-Verbs $rel) | Should -Not -Contain 'edit'
        (Get-Verbs $rel)[-1] | Should -Be 'view'
    }

    It 'writes_the_notes_to_a_temporary_file_and_removes_it' {
        $rel = New-FakeRelease
        Publish $rel (New-Setup)
        $notes = @($rel.Calls | Where-Object { $_[1] -eq 'create' })[0]
        $path = $notes[([array]::IndexOf($notes, '--notes-file') + 1)]
        Test-Path -LiteralPath $path | Should -BeFalse
    }

    It 'updates_an_existing_draft_keeping_changes' {
        $old = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $false
        $old = $old.Replace('<!-- Write the changes here -->', 'Hand-written news')
        $rel = New-FakeRelease -Exists $true -Body $old
        $rel.Assets['OpenMonitor.Advanced_1.2.3_x64-setup.exe'] = @{ Bytes = $utf8.GetBytes('old'); State = 'uploaded' }
        $f = New-Setup
        Publish $rel $f -Signed $true
        $rel.Body | Should -Match 'Hand-written news'
        $rel.Body | Should -Match ([regex]::Escape($attribution))
        $rel.Body | Should -Not -Match ([regex]::Escape($unsignedLine))
        $verbs = Get-Verbs $rel
        $verbs | Should -Not -Contain 'create'
        $upload = @($rel.Calls | Where-Object { $_[1] -eq 'upload' })[0]
        $upload | Should -Contain '--clobber'
        $upload[([array]::IndexOf($upload, '--repo') + 1)] | Should -Be $repo
        $upload | Should -Contain $f.Setup
        $upload | Should -Contain $f.Sums
        $rel.IsDraft | Should -BeTrue
        # edit, then upload, each preceded by an isDraft check.
        $i = [array]::IndexOf($verbs, 'edit')
        $verbs[$i - 1] | Should -Be 'view'
        $verbs[$i + 1] | Should -Be 'view'
        $verbs[$i + 2] | Should -Be 'upload'
        $utf8.GetString($rel.Assets['OpenMonitor.Advanced_1.2.3_x64-setup.exe'].Bytes) | Should -Be 'installer bytes'
    }

    It 'refuses_a_published_release' {
        $rel = New-FakeRelease -Exists $true -IsDraft $false -Body 'published notes'
        { Publish $rel (New-Setup) } | Should -Throw 'release v1.2.3 is already published'
        (Get-Verbs $rel) | Should -Be @('view')
        $rel.Body | Should -Be 'published notes'
    }

    It 'rechecks_draft_before_writing' {
        $body = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $false
        # Views: 1 state, 2 body, 3 recheck before edit -> published by someone meanwhile.
        $rel = New-FakeRelease -Exists $true -Body $body
        $rel.FlipToPublishedAtView = 3
        { Publish $rel (New-Setup) } | Should -Throw '*v1.2.3*published*'
        (Get-Verbs $rel) | Should -Not -Contain 'edit'
        (Get-Verbs $rel) | Should -Not -Contain 'upload'

        # 4 = recheck before upload: the notes were edited, the assets are not touched.
        $rel = New-FakeRelease -Exists $true -Body $body
        $rel.FlipToPublishedAtView = 4
        { Publish $rel (New-Setup) } | Should -Throw '*v1.2.3*published*'
        (Get-Verbs $rel) | Should -Contain 'edit'
        (Get-Verbs $rel) | Should -Not -Contain 'upload'
    }

    It 'invalid_markers_fail_before_upload' {
        $rel = New-FakeRelease -Exists $true -Body 'a body someone rewrote without markers'
        { Publish $rel (New-Setup) } | Should -Throw '*marker*'
        (Get-Verbs $rel) | Should -Be @('view', 'view')
        $rel.Body | Should -Be 'a body someone rewrote without markers'
    }

    It 'verifies_remote_assets_after_upload' {
        $rel = New-FakeRelease
        $rel.CorruptUpload = $true
        { Publish $rel (New-Setup) } | Should -Throw "*$mismatch*"

        # An extra remote asset (a leftover from an older run) is a mismatch too.
        $body = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $true
        $rel = New-FakeRelease -Exists $true -Body $body
        $rel.Assets['OpenMonitor Advanced_1.2.3_x64-setup.exe'] = @{ Bytes = $utf8.GetBytes('x'); State = 'uploaded' }
        { Publish $rel (New-Setup) } | Should -Throw "*$mismatch*"
        (Get-Verbs $rel) | Should -Not -Contain 'delete-asset'

        # A remote asset not yet in state uploaded.
        $rel = New-FakeRelease -Exists $true -Body $body
        $f = New-Setup
        Publish $rel $f
        $rel.Assets['SHA256SUMS.txt'].State = 'starter'
        { Assert-OmaRemoteAssets -Repo $repo -Tag 'v1.2.3' -Setup $f.Setup -Sums $f.Sums -Gh $rel.Block } | Should -Throw "*$mismatch*"
    }

    It 'null_digest_downloads_and_hashes_both_assets' {
        $rel = New-FakeRelease
        $rel.NullDigest = $true
        $f = New-Setup
        Publish $rel $f
        $downloads = @($rel.Calls | Where-Object { $_[1] -eq 'download' })
        $downloads.Count | Should -Be 2
        foreach ($d in $downloads) {
            $d[2] | Should -Be 'v1.2.3'
            $d[([array]::IndexOf($d, '--repo') + 1)] | Should -Be $repo
        }
        @($downloads | ForEach-Object { $_[([array]::IndexOf($_, '--pattern') + 1)] }) |
            Should -Be @('OpenMonitor.Advanced_1.2.3_x64-setup.exe', 'SHA256SUMS.txt')
        (Get-Verbs $rel) | Should -Contain '--version'

        # With a null digest a tampered remote file still fails.
        $rel = New-FakeRelease
        $rel.NullDigest = $true
        $rel.CorruptUpload = $true
        { Publish $rel (New-Setup) } | Should -Throw "*$mismatch*"
    }

    It 'partial_upload_is_reported' {
        $body = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $true
        $rel = New-FakeRelease -Exists $true -Body $body
        $rel.UploadFailAfter = 1
        { Publish $rel (New-Setup) } | Should -Throw '*do not publish this draft, re-run the workflow*'
        (Get-Verbs $rel) | Should -Not -Contain 'delete-asset'
        (Get-Verbs $rel) | Should -Not -Contain 'delete'
    }

    It 'rejects_local_files_that_do_not_match' {
        $rel = New-FakeRelease
        $f = New-Setup -Version '1.2.4'
        { Publish $rel $f } | Should -Throw '*OpenMonitor.Advanced_1.2.3_x64-setup.exe*'
        $f = New-Setup
        [IO.File]::WriteAllText($f.Sums, "$('0' * 64)  OpenMonitor.Advanced_1.2.3_x64-setup.exe`n")
        { Publish $rel $f } | Should -Throw '*SHA256SUMS.txt*'
        { Publish-OmaDraft -Repo $repo -Tag 'v1.2.4' -Version '1.2.3' -Setup (New-Setup).Setup -Sums (New-Setup).Sums `
                -Signed $true -TemplatePath $template -Gh $rel.Block } | Should -Throw '*v1.2.4*'
        $rel.Calls.Count | Should -Be 0
    }
}

Describe 'Invoke-OmaReleasePreflight' {
    It 'preflight_maps_signpath_and_verify_policies' {
        $certs = New-Certificates -Complete
        $o = Preflight -HasToken 'true' -OrganizationId 'org-1' -Slug 'OpenMonitorAdvanced' -Certificates $certs
        $o['signing'] | Should -Be 'enabled'
        $o['signpath-policy'] | Should -Be 'release-signing'
        $o['verify-policy'] | Should -Be 'release'

        $o = Preflight -EventName 'workflow_dispatch' -Ref 'refs/heads/main' -HasToken 'true' -OrganizationId 'org-1' -Slug 'OpenMonitorAdvanced' -Certificates $certs
        $o['signpath-policy'] | Should -Be 'test-signing'
        $o['verify-policy'] | Should -Be 'test'

        $o = Preflight -EventName 'workflow_dispatch' -Ref 'refs/heads/main'
        $o['signing'] | Should -Be 'disabled'
        $o['verify-policy'] | Should -Be 'none'
    }

    It 'false_env_string_is_not_true_in_the_preflight' {
        (Preflight -HasToken 'false')['signing'] | Should -Be 'disabled'
        { Preflight -HasToken 'false' -OrganizationId 'org-1' } | Should -Throw '*SIGNPATH_API_TOKEN*'
        { Preflight -HasToken 'maybe' } | Should -Throw '*HasToken*'
        { Preflight -RequireSigning 'TRUE?' } | Should -Throw '*REQUIRE_SIGNING*'
        { Preflight -RequireSigning 'true' } | Should -Throw '*REQUIRE_SIGNING*'
    }

    It 'enabled_signing_requires_project_and_certificates' {
        $ok = New-Certificates -Complete
        { Preflight -HasToken 'true' -OrganizationId 'org-1' -Slug '' -Certificates $ok } | Should -Throw '*SignPath project slug*'
        { Preflight -HasToken 'true' -OrganizationId 'org-1' -Slug '<project-slug>' -Certificates $ok } | Should -Throw '*SignPath project slug*'
        { Preflight -HasToken 'true' -OrganizationId 'org-1' -Slug 'OpenMonitorAdvanced' -Certificates (New-Certificates) } |
            Should -Throw '*no approved certificate configured for policy release*'
        { Preflight -EventName 'workflow_dispatch' -Ref 'refs/heads/main' -HasToken 'true' -OrganizationId 'org-1' -Slug 'OpenMonitorAdvanced' -Certificates (New-Certificates) } |
            Should -Throw '*no approved certificate configured for policy test*'
        # Disabled signing needs neither.
        (Preflight -Slug '' -Certificates (New-Certificates))['signing'] | Should -Be 'disabled'
    }

    It 'stops_before_github_on_local_errors' {
        $ci = New-FakeCi
        { Preflight -EventName 'workflow_dispatch' -Ref 'refs/heads/dev' -Gh $ci.Block } | Should -Throw '*refs/heads/main*'
        { Preflight -HasToken 'true' -Gh $ci.Block } | Should -Throw '*SIGNPATH_ORGANIZATION_ID*'
        $ci.Calls.Count | Should -Be 0
    }

    It 'checks_ci_and_release_state_for_a_tag' {
        $ci = New-FakeCi -JobPages @{ '1/1' = @(, @(New-Job 'checks' 1 'failure')) }
        $rel = New-FakeRelease
        $gh = { param([string[]]$Arguments) if ($Arguments[0] -eq 'api') { & $ci.Block $Arguments } else { & $rel.Block $Arguments } }.GetNewClosure()
        { Preflight -Gh $gh } | Should -Throw '*checks*'

        $ci = New-FakeCi
        $rel = New-FakeRelease -Exists $true -IsDraft $false
        $gh = { param([string[]]$Arguments) if ($Arguments[0] -eq 'api') { & $ci.Block $Arguments } else { & $rel.Block $Arguments } }.GetNewClosure()
        { Preflight -Gh $gh } | Should -Throw 'release v1.2.3 is already published'

        # A dispatch does not look at releases.
        $ci = New-FakeCi
        $rel = New-FakeRelease -Exists $true -IsDraft $false
        $gh = { param([string[]]$Arguments) if ($Arguments[0] -eq 'api') { & $ci.Block $Arguments } else { & $rel.Block $Arguments } }.GetNewClosure()
        { Preflight -EventName 'workflow_dispatch' -Ref 'refs/heads/main' -Gh $gh } | Should -Not -Throw
        $rel.Calls.Count | Should -Be 0
    }

    It 'tag_parameter_must_match_the_ref' {
        { Preflight -Tag 'v1.2.4' } | Should -Throw '*v1.2.4*'
        (Preflight -Tag 'v1.2.3')['signing'] | Should -Be 'disabled'
        { Preflight -EventName 'workflow_dispatch' -Ref 'refs/heads/main' -Tag 'v1.2.3' } | Should -Throw '*Tag*'
    }

    It 'preflight_writes_outputs' {
        $out = Join-Path $TestDrive 'github_output.txt'
        [IO.File]::WriteAllText($out, "earlier=1`n")
        Preflight -Output $out | Out-Null
        $bytes = [IO.File]::ReadAllBytes($out)
        $bytes | Should -Not -Contain 13
        $utf8.GetString($bytes) | Should -BeExactly "earlier=1`nsigning=disabled`nsignpath-policy=release-signing`nverify-policy=none`n"
    }
}

Describe 'release-preflight.ps1' {
    BeforeAll {
        # A fake gh.cmd first on PATH: it answers the CI gate with green runs and every release
        # view with "release not found", and logs its arguments.
        $script:fakeDir = Join-Path $TestDrive 'fakegh'
        New-Item -ItemType Directory -Path $fakeDir | Out-Null
        $runs = ConvertTo-Json -InputObject @(@{ workflow_runs = @(New-Run) }) -Depth 10 -Compress
        $jobs = ConvertTo-Json -InputObject @(@{ jobs = @(All-Jobs) }) -Depth 10 -Compress
        [IO.File]::WriteAllText((Join-Path $fakeDir 'runs.json'), $runs)
        [IO.File]::WriteAllText((Join-Path $fakeDir 'jobs.json'), $jobs)
        $ps = @'
$log = Join-Path $PSScriptRoot 'calls.log'
Add-Content -LiteralPath $log -Value ($args -join ' ')
$joined = $args -join ' '
if ($joined -match 'actions/workflows/ci.yml/runs') { [Console]::Out.Write([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'runs.json'))); exit 0 }
if ($joined -match '/attempts/1/jobs') { [Console]::Out.Write([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'jobs.json'))); exit 0 }
if ($args[0] -eq 'release' -and $args[1] -eq 'view') { [Console]::Error.WriteLine('release not found'); exit 1 }
[Console]::Error.WriteLine("unexpected fake gh call: $joined"); exit 1
'@
        [IO.File]::WriteAllText((Join-Path $fakeDir 'fake-gh.ps1'), $ps)
        [IO.File]::WriteAllText((Join-Path $fakeDir 'gh.cmd'), "@pwsh -NoProfile -NonInteractive -File `"%~dp0fake-gh.ps1`" %*`r`n")
        $script:savedPath = $env:PATH
    }

    AfterEach { $env:PATH = $savedPath }

    It 'preflight_script_writes_outputs_from_string_inputs' {
        $env:PATH = "$fakeDir;$savedPath"
        $out = Join-Path $TestDrive 'entry_output.txt'
        $saved = $env:GITHUB_OUTPUT
        try {
            $env:GITHUB_OUTPUT = $out
            $log = & pwsh -NoProfile -NonInteractive -File $preflightScript -EventName push -Ref refs/tags/v1.2.3 -Sha $sha `
                -Repo $repo -HasToken false -Tag v1.2.3 2>&1
            $LASTEXITCODE | Should -Be 0 -Because ($log -join "`n")
        } finally {
            $env:GITHUB_OUTPUT = $saved
        }
        $utf8.GetString([IO.File]::ReadAllBytes($out)) | Should -BeExactly "signing=disabled`nsignpath-policy=release-signing`nverify-policy=none`n"
        (Get-Content (Join-Path $fakeDir 'calls.log')) -join "`n" | Should -Match "release view v1\.2\.3 --repo $repo"
    }

    It 'preflight_script_fails_on_a_bad_boolean' {
        $env:PATH = "$fakeDir;$savedPath"
        $log = & pwsh -NoProfile -NonInteractive -File $preflightScript -EventName push -Ref refs/tags/v1.2.3 -Sha $sha `
            -Repo $repo -HasToken yes 2>&1
        $LASTEXITCODE | Should -Be 1
        ($log -join "`n") | Should -Match 'HasToken'
    }
}

Describe 'publish-draft.ps1' {
    It 'fails_with_exit_code_before_github_when_the_setup_is_missing' {
        $f = New-Setup
        $log = & pwsh -NoProfile -NonInteractive -File $publishScript -Repo $repo -Tag v1.2.3 -Version 1.2.3 `
            -Setup (Join-Path $TestDrive 'missing.exe') -Sums $f.Sums -Signed false -TemplatePath $template 2>&1
        $LASTEXITCODE | Should -Be 1
        ($log -join "`n") | Should -Match 'missing\.exe'
    }

    It 'rejects_an_unknown_signed_value' {
        $f = New-Setup
        $log = & pwsh -NoProfile -NonInteractive -File $publishScript -Repo $repo -Tag v1.2.3 -Version 1.2.3 `
            -Setup $f.Setup -Sums $f.Sums -Signed maybe -TemplatePath $template 2>&1
        $LASTEXITCODE | Should -Be 1
        ($log -join "`n") | Should -Match 'Signed'
    }
}
