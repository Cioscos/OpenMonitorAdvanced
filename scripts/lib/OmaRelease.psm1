#Requires -Version 7
# Release preflight and draft publishing (spec M6a §4.1-4.2, §5.2-5.3; plan L3, L4, L7).
#
# The preflight runs before any build or signing request: event and ref, signing mode and its
# local configuration, the CI gate on the same commit and, for a tag, a release that is not
# already published. Publish-OmaDraft creates or updates the draft and then checks the remote
# assets against the local files.
#
# Adapter contract for -Gh: a scriptblock `param([string[]]$Arguments)` returning
# @{ Stdout; Stderr; ExitCode } without throwing on a nonzero exit code. The default wraps
# Invoke-OmaNative -AllowFailure, so the JSON on stdout never mixes with gh's diagnostics on
# stderr. Every release command names --repo and every API call the repository in its path.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'OmaCommon.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'OmaVersion.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'OmaReleaseNotes.psm1') -ErrorAction Stop
Import-Module (Join-Path $PSScriptRoot 'OmaSigning.psm1') -ErrorAction Stop

$script:DefaultGh = { param([string[]]$Arguments) Invoke-OmaNative -FilePath 'gh' -ArgumentList $Arguments -AllowFailure }
$script:ApiHeaders = @('-H', 'Accept: application/vnd.github+json', '-H', 'X-GitHub-Api-Version: 2022-11-28')
# Plan L7: the job ids of ci.yml (they have no `name:`, so the API reports the id as the name).
$script:CiJobs = @('checks', 'service', 'installer', 'scripts', 'actionlint')
# \z, not $: in .NET $ also accepts a trailing \n.
$script:RepoPattern = '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\z'
$script:ShaPattern = '^[0-9a-f]{40}\z'
$script:TagPattern = '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\z'
$script:SumsName = 'SHA256SUMS.txt'
$script:Mismatch = 'remote assets do not match; do not publish this draft, re-run the workflow'
$script:Utf8 = [Text.UTF8Encoding]::new($false)
$script:DefaultCertificatesPath = Join-Path $PSScriptRoot '..\..\.signpath\certificates.json'

# --- helpers -------------------------------------------------------------------------------------

function Get-OmaJsonProperty($Object, [string]$Name) {
    if ($null -eq $Object) { return $null }
    $p = $Object.PSObject.Properties[$Name]
    if ($null -eq $p) { return $null }
    $p.Value
}

function Assert-OmaRepo([string]$Repo) {
    if ($Repo -cnotmatch $script:RepoPattern) { throw "invalid repository '$Repo' (expected owner/name)" }
}

function Assert-OmaTag([string]$Tag) {
    if ($Tag -cnotmatch $script:TagPattern) { throw "invalid tag '$Tag' (expected vX.Y.Z)" }
}

function Get-OmaFirstLine([string]$Text) {
    $t = "$Text".Trim()
    if (-not $t) { return '(no output)' }
    ($t -split '\r?\n')[0]
}

# Parses gh's stdout as JSON; an empty or invalid answer is an error, never "nothing found".
function ConvertFrom-OmaGhJson([string]$Text, [string]$What) {
    if ([string]::IsNullOrWhiteSpace($Text)) { throw "$What returned no JSON" }
    try {
        ConvertFrom-Json -InputObject $Text -Depth 100 -NoEnumerate
    } catch {
        throw "$What returned invalid JSON: $($_.Exception.Message)"
    }
}

# Every item of a paginated GET, read with --paginate --slurp (a JSON array of page objects).
function Get-OmaApiItems {
    param([scriptblock]$Gh, [string]$Path, [string[]]$Query, [string]$Property, [string]$What)
    $arguments = @('api', '--method', 'GET') + $script:ApiHeaders + @('--paginate', '--slurp', $Path)
    foreach ($q in $Query) { $arguments += @('-f', $q) }
    $r = & $Gh $arguments
    if ($r.ExitCode -ne 0) { throw "$What failed (gh exit code $($r.ExitCode)): $("$($r.Stderr)".Trim())" }
    $pages = ConvertFrom-OmaGhJson $r.Stdout $What
    if ($pages -isnot [array]) { throw "$What`: expected a list of pages from gh --slurp" }
    foreach ($page in $pages) {
        if ($null -eq $page -or $null -eq $page.PSObject.Properties[$Property]) { throw "$What`: a page has no '$Property'" }
        foreach ($item in @($page.$Property)) { $item }
    }
}

# Throws unless the release is still a draft: called right before every write (spec §5.3).
function Assert-OmaStillDraft([string]$Repo, [string]$Tag, [scriptblock]$Gh) {
    if ((Get-OmaReleaseState -Repo $Repo -Tag $Tag -Gh $Gh) -ne 'draft') {
        throw "release $Tag is no longer a draft: it was published or deleted during this run, so nothing more is written"
    }
}

function Write-OmaNotesFile([string]$Text) {
    $path = Join-Path ([IO.Path]::GetTempPath()) ("oma-release-notes-" + [guid]::NewGuid().ToString('N') + '.md')
    [IO.File]::WriteAllBytes($path, $script:Utf8.GetBytes($Text.Replace("`r`n", "`n")))
    $path
}

# The two local assets and what the remote copies must match. Fails before any GitHub call.
function Get-OmaLocalAssets([string]$Tag, [string]$Version, [string]$Setup, [string]$Sums) {
    Assert-OmaTag $Tag
    if ($Tag -cne "v$Version") { throw "tag $Tag does not match version $Version" }
    $setupName = Get-OmaFinalSetupName -Version $Version
    foreach ($p in $Setup, $Sums) {
        if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { throw "file not found: $p" }
        # gh reads 'file#label' as a display label.
        if ($p.Contains('#')) { throw "asset path must not contain '#': $p" }
    }
    if ([IO.Path]::GetFileName($Setup) -cne $setupName) {
        throw "setup must be named $setupName (plan L3), got $([IO.Path]::GetFileName($Setup))"
    }
    if ([IO.Path]::GetFileName($Sums) -cne $script:SumsName) { throw "checksum file must be named $($script:SumsName): $Sums" }
    $setupHash = Get-OmaSha256 $Setup
    $expectedSums = "$setupHash  $setupName`n"
    if ($script:Utf8.GetString([IO.File]::ReadAllBytes($Sums)) -cne $expectedSums) {
        throw "$($script:SumsName) does not hold exactly '$($expectedSums.TrimEnd())' for $Setup"
    }
    [ordered]@{
        $setupName       = [pscustomobject]@{ Path = $Setup; Size = (Get-Item -LiteralPath $Setup).Length; Hash = $setupHash }
        $script:SumsName = [pscustomobject]@{ Path = $Sums; Size = (Get-Item -LiteralPath $Sums).Length; Hash = (Get-OmaSha256 $Sums) }
    }
}

# --- signing mode and run reference --------------------------------------------------------------

<#
.SYNOPSIS
  Converts 'true'/'false' (any case, nothing else) to a bool. [bool]'false' is $true in
  PowerShell, so workflow strings are never cast. With -EmptyMeans, an empty text maps to it.
#>
function ConvertFrom-OmaBoolText {
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string]$Name, [AllowNull()] [AllowEmptyString()] [string]$Text, $EmptyMeans)
    if ([string]::IsNullOrEmpty($Text) -and $null -ne $EmptyMeans) { return [bool]$EmptyMeans }
    if ($Text -ieq 'true') { return $true }
    if ($Text -ieq 'false') { return $false }
    throw "$Name must be 'true' or 'false', got '$Text'"
}

<#
.SYNOPSIS
  'enabled' when both SignPath credentials exist, 'disabled' when both are missing and signing is
  not required (spec §4.2). A single credential, or REQUIRE_SIGNING=true without credentials,
  throws. REQUIRE_SIGNING: empty means false (before activation); only true/false are accepted.
#>
function Get-OmaSigningMode {
    [CmdletBinding()]
    param([bool]$HasToken, [string]$OrganizationId, [string]$RequireSigning)
    $require = ConvertFrom-OmaBoolText -Name 'REQUIRE_SIGNING' -Text $RequireSigning -EmptyMeans $false
    $hasOrganization = -not [string]::IsNullOrWhiteSpace($OrganizationId)
    if ($HasToken -and $hasOrganization) { return 'enabled' }
    if ($HasToken) {
        throw 'partial signing configuration: the SIGNPATH_API_TOKEN secret is set but the SIGNPATH_ORGANIZATION_ID variable is missing'
    }
    if ($hasOrganization) {
        throw 'partial signing configuration: the SIGNPATH_ORGANIZATION_ID variable is set but the SIGNPATH_API_TOKEN secret is missing'
    }
    if ($require) {
        throw 'REQUIRE_SIGNING is true but SIGNPATH_API_TOKEN and SIGNPATH_ORGANIZATION_ID are missing: refusing an unsigned build'
    }
    'disabled'
}

<#
.SYNOPSIS
  push: only refs/tags/vX.Y.Z (canonical, components <= 65535); workflow_dispatch: only
  refs/heads/main. Returns @{ Kind = 'tag'|'dispatch'; Tag; Version }.
#>
function Assert-OmaRunRef {
    [CmdletBinding()]
    param([string]$EventName, [string]$Ref)
    switch -CaseSensitive ($EventName) {
        'push' {
            if (-not $Ref.StartsWith('refs/tags/', [StringComparison]::Ordinal)) {
                throw "a push run must be for a tag refs/tags/vX.Y.Z, got '$Ref'"
            }
            $tag = $Ref.Substring('refs/tags/'.Length)
            if ($tag -cnotmatch $script:TagPattern) { throw "tag '$tag' is not vX.Y.Z" }
            $version = $tag.Substring(1)
            try { $null = ConvertTo-OmaVersion $version } catch { throw "tag '$tag': $($_.Exception.Message)" }
            return [pscustomobject]@{ Kind = 'tag'; Tag = $tag; Version = $version }
        }
        'workflow_dispatch' {
            if ($Ref -cne 'refs/heads/main') { throw "a workflow_dispatch run must start from refs/heads/main, got '$Ref'" }
            return [pscustomobject]@{ Kind = 'dispatch'; Tag = $null; Version = $null }
        }
    }
    throw "unsupported event '$EventName': releases run on a tag push or on workflow_dispatch from main"
}

<#
.SYNOPSIS
  With signing enabled: a real SignPath project slug and complete L4 certificates for the verify
  policy, checked with the validator of verify-signatures. No SignPath call, no token.
#>
function Assert-OmaSigningConfiguration {
    [CmdletBinding()]
    param([string]$ProjectSlug, [string]$CertificatesPath, [ValidateSet('release', 'test')] [string]$Policy)
    if ([string]::IsNullOrWhiteSpace($ProjectSlug) -or $ProjectSlug -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._-]*\z' -or
        $ProjectSlug -imatch '^(todo|tbd|changeme|placeholder|x+)\z') {
        throw "signing is enabled but the SignPath project slug is missing or a placeholder: '$ProjectSlug'"
    }
    if (-not (Test-Path -LiteralPath $CertificatesPath -PathType Leaf)) { throw "certificate configuration not found: $CertificatesPath" }
    $certificates = Get-Content -Raw -LiteralPath $CertificatesPath | ConvertFrom-Json
    $config = Get-OmaPolicyCertificates $certificates $Policy
    if ($config.Problem) { throw $config.Problem }
    if ($Policy -eq 'test' -and -not (Test-Path -LiteralPath $config.rootCertificatePath -PathType Leaf)) {
        throw "test root certificate not found: $($config.rootCertificatePath)"
    }
}

# --- GitHub state --------------------------------------------------------------------------------

<#
.SYNOPSIS
  Plan L7: the most recent ci.yml run of a push to main of this repository on this commit, its
  latest attempt completed with success, and every expected job of that attempt successful.
  API or authentication errors are errors, never "no run".
#>
function Assert-OmaCiGreen {
    [CmdletBinding()]
    param([string]$Repo, [string]$Sha, [scriptblock]$Gh = $script:DefaultGh)
    Assert-OmaRepo $Repo
    if ($Sha -cnotmatch $script:ShaPattern) { throw "invalid commit SHA '$Sha' (expected 40 lowercase hex digits)" }

    $runs = @(Get-OmaApiItems -Gh $Gh -Path "repos/$Repo/actions/workflows/ci.yml/runs" -Property 'workflow_runs' `
            -Query @('event=push', 'branch=main', "head_sha=$Sha", 'per_page=100') -What "listing the CI runs of $Repo")
    $relevant = @($runs | Where-Object {
            (Get-OmaJsonProperty $_ 'event') -ceq 'push' -and
            (Get-OmaJsonProperty $_ 'head_branch') -ceq 'main' -and
            (Get-OmaJsonProperty $_ 'head_sha') -ceq $Sha -and
            (Get-OmaJsonProperty (Get-OmaJsonProperty $_ 'repository') 'full_name') -ieq $Repo -and
            (Get-OmaJsonProperty (Get-OmaJsonProperty $_ 'head_repository') 'full_name') -ieq $Repo
        })
    if ($relevant.Count -eq 0) {
        throw "no CI run of ci.yml for a push to main of $Repo on $($Sha): wait for CI on main, then re-run"
    }
    $run = $relevant | Sort-Object { [long](Get-OmaJsonProperty $_ 'run_number') }, { [long](Get-OmaJsonProperty $_ 'id') } -Descending |
        Select-Object -First 1
    $id = [long](Get-OmaJsonProperty $run 'id')
    $attempt = [int](Get-OmaJsonProperty $run 'run_attempt')
    $status = Get-OmaJsonProperty $run 'status'
    $conclusion = Get-OmaJsonProperty $run 'conclusion'
    $label = "CI run $id (attempt $attempt) on $Sha"
    if ($status -cne 'completed') { throw "$label is $status, not completed: wait for it, then re-run" }
    if ($conclusion -cne 'success') { throw "$label concluded $conclusion" }
    if ($attempt -lt 1) { throw "$label has no valid run_attempt" }

    $jobs = @(Get-OmaApiItems -Gh $Gh -Path "repos/$Repo/actions/runs/$id/attempts/$attempt/jobs" -Property 'jobs' `
            -Query @('per_page=100') -What "listing the jobs of $label")
    # Only the jobs of the latest attempt count, never those of an earlier one.
    $jobs = @($jobs | Where-Object { [int](Get-OmaJsonProperty $_ 'run_attempt') -eq $attempt })
    $problems = @(foreach ($name in $script:CiJobs) {
            $matching = @($jobs | Where-Object { (Get-OmaJsonProperty $_ 'name') -ceq $name })
            if ($matching.Count -eq 0) { "job $name missing"; continue }
            foreach ($j in $matching) {
                $s = Get-OmaJsonProperty $j 'status'
                $c = Get-OmaJsonProperty $j 'conclusion'
                if ($s -cne 'completed') { "job $name is $s" }
                elseif ($c -cne 'success') { "job $name concluded $c" }
            }
        })
    if ($problems.Count -gt 0) { throw "$label is not green: $($problems -join '; ')" }
}

<#
.SYNOPSIS
  'absent' | 'draft' | 'published'. gh release view also finds drafts. Only gh's own
  "release not found" means absent; any other failure or an unreadable answer throws.
#>
function Get-OmaReleaseState {
    [CmdletBinding()]
    param([string]$Repo, [string]$Tag, [scriptblock]$Gh = $script:DefaultGh)
    Assert-OmaRepo $Repo
    Assert-OmaTag $Tag
    $r = & $Gh @('release', 'view', $Tag, '--repo', $Repo, '--json', 'isDraft,tagName')
    if ($r.ExitCode -ne 0) {
        if ("$($r.Stderr)".Trim() -ceq 'release not found') { return 'absent' }
        throw "cannot read release $Tag of $Repo (gh exit code $($r.ExitCode)): $("$($r.Stderr)".Trim())"
    }
    $o = ConvertFrom-OmaGhJson $r.Stdout "gh release view $Tag"
    $tagName = Get-OmaJsonProperty $o 'tagName'
    if ($tagName -cne $Tag) { throw "gh release view $Tag returned release '$tagName'" }
    $isDraft = Get-OmaJsonProperty $o 'isDraft'
    if ($isDraft -isnot [bool]) { throw "gh release view $Tag returned no boolean isDraft" }
    if ($isDraft) { 'draft' } else { 'published' }
}

# --- assets --------------------------------------------------------------------------------------

<#
.SYNOPSIS
  Plan L3: OpenMonitor.Advanced_X.Y.Z_x64-setup.exe, dots instead of spaces like GitHub's assets.
#>
function Get-OmaFinalSetupName {
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string]$Version)
    $null = ConvertTo-OmaVersion $Version
    "OpenMonitor.Advanced_$($Version)_x64-setup.exe"
}

<#
.SYNOPSIS
  Writes "<sha256 lowercase>  <file name>\n" (sha256sum format), UTF-8 without BOM.
#>
function Write-OmaSha256Sums {
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string]$Setup, [Parameter(Mandatory)] [string]$Out)
    $line = "$(Get-OmaSha256 $Setup)  $([IO.Path]::GetFileName($Setup))`n"
    [IO.File]::WriteAllBytes($Out, $script:Utf8.GetBytes($line))
}

<#
.SYNOPSIS
  The draft must list exactly the setup and SHA256SUMS.txt, uploaded, with the local sizes and
  sha256 digests. A null digest (older gh or API) is neither success nor a dead end: the asset
  is downloaded from the draft with authenticated gh and hashed. Any mismatch throws.
#>
function Assert-OmaRemoteAssets {
    [CmdletBinding()]
    param([string]$Repo, [string]$Tag, [string]$Setup, [string]$Sums, [scriptblock]$Gh = $script:DefaultGh)
    Assert-OmaRepo $Repo
    Assert-OmaTag $Tag
    $expected = Get-OmaLocalAssets -Tag $Tag -Version $Tag.Substring(1) -Setup $Setup -Sums $Sums

    $r = & $Gh @('release', 'view', $Tag, '--repo', $Repo, '--json', 'assets,isDraft')
    if ($r.ExitCode -ne 0) { throw "$($script:Mismatch): cannot read the assets of $Tag ($("$($r.Stderr)".Trim()))" }
    $o = ConvertFrom-OmaGhJson $r.Stdout "gh release view $Tag"
    if ((Get-OmaJsonProperty $o 'isDraft') -ne $true) { throw "$($script:Mismatch): release $Tag is not a draft any more" }
    $assets = @(Get-OmaJsonProperty $o 'assets')

    $problems = [Collections.Generic.List[string]]::new()
    $remoteNames = @($assets | ForEach-Object { [string](Get-OmaJsonProperty $_ 'name') } | Sort-Object)
    $expectedNames = @($expected.Keys | Sort-Object)
    if (($remoteNames -join '|') -cne ($expectedNames -join '|')) {
        $problems.Add("remote assets are [$($remoteNames -join ', ')], expected exactly [$($expectedNames -join ', ')]")
    }
    $toDownload = [Collections.Generic.List[string]]::new()
    foreach ($name in $expected.Keys) {
        $local = $expected[$name]
        $a = @($assets | Where-Object { (Get-OmaJsonProperty $_ 'name') -ceq $name })
        if ($a.Count -ne 1) { continue }
        $a = $a[0]
        $state = Get-OmaJsonProperty $a 'state'
        if ($state -cne 'uploaded') { $problems.Add("$name is in state '$state', not uploaded") }
        $size = Get-OmaJsonProperty $a 'size'
        if ($null -eq $size -or [long]$size -ne $local.Size) { $problems.Add("$name has size $size, expected $($local.Size)") }
        $digest = Get-OmaJsonProperty $a 'digest'
        if ([string]::IsNullOrEmpty($digest)) { $toDownload.Add($name) }
        elseif ($digest -cne "sha256:$($local.Hash)") { $problems.Add("$name has digest $digest, expected sha256:$($local.Hash)") }
    }

    if ($toDownload.Count -gt 0) {
        $version = & $Gh @('--version')
        Write-Information ("gh returned no digest for $($toDownload -join ', ') ($(Get-OmaFirstLine $version.Stdout)): " +
            'downloading them from the draft to compare the SHA-256')
        $dir = Join-Path ([IO.Path]::GetTempPath()) ('oma-release-assets-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            foreach ($name in $toDownload) {
                $d = & $Gh @('release', 'download', $Tag, '--repo', $Repo, '--pattern', $name, '--dir', $dir)
                $file = Join-Path $dir $name
                if ($d.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $file -PathType Leaf)) {
                    $problems.Add("cannot download $name to verify it: $("$($d.Stderr)".Trim())")
                    continue
                }
                $hash = Get-OmaSha256 $file
                if ($hash -cne $expected[$name].Hash) { $problems.Add("downloaded $name has SHA-256 $hash, expected $($expected[$name].Hash)") }
            }
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    if ($problems.Count -gt 0) { throw "$($script:Mismatch): $($problems -join '; ')" }
}

<#
.SYNOPSIS
  Creates the draft for a tag, or updates an existing draft (notes: generated block only; assets
  with --clobber), then verifies the remote assets. A published release is never touched.
.DESCRIPTION
  The writes are not atomic and isDraft is re-read right before each of them; publishing by hand
  during a run stays forbidden (spec §5.3). After a partial failure nothing is deleted and the
  error says not to publish the draft. Notes go through a temporary UTF-8/LF file.
#>
function Publish-OmaDraft {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$Repo,
        [Parameter(Mandatory)] [string]$Tag,
        [Parameter(Mandatory)] [string]$Version,
        [Parameter(Mandatory)] [string]$Setup,
        [Parameter(Mandatory)] [string]$Sums,
        [Parameter(Mandatory)] [bool]$Signed,
        [Parameter(Mandatory)] [string]$TemplatePath,
        [scriptblock]$Gh = $script:DefaultGh
    )
    Assert-OmaRepo $Repo
    $null = Get-OmaLocalAssets -Tag $Tag -Version $Version -Setup $Setup -Sums $Sums
    $retry = 'do not publish this draft, re-run the workflow'

    $state = Get-OmaReleaseState -Repo $Repo -Tag $Tag -Gh $Gh
    if ($state -eq 'published') { throw "release $Tag is already published" }
    $notesFile = $null
    try {
        if ($state -eq 'absent') {
            $notesFile = Write-OmaNotesFile (New-OmaReleaseNotes -TemplatePath $TemplatePath -Version $Version -Signed $Signed)
            $r = & $Gh @('release', 'create', $Tag, '--repo', $Repo, '--draft', '--verify-tag',
                '--title', "OpenMonitor Advanced $Version", '--notes-file', $notesFile, $Setup, $Sums)
            if ($r.ExitCode -ne 0) {
                throw "creating the draft $Tag failed (gh exit code $($r.ExitCode)): $("$($r.Stderr)".Trim()); a draft with part of the assets may exist: $retry"
            }
            $action = 'created'
        } else {
            $r = & $Gh @('release', 'view', $Tag, '--repo', $Repo, '--json', 'body,isDraft')
            if ($r.ExitCode -ne 0) { throw "cannot read the draft $Tag (gh exit code $($r.ExitCode)): $("$($r.Stderr)".Trim())" }
            $o = ConvertFrom-OmaGhJson $r.Stdout "gh release view $Tag"
            if ((Get-OmaJsonProperty $o 'isDraft') -ne $true) { throw "release $Tag is already published" }
            # Invalid markers throw here, before any write.
            $body = [string](Get-OmaJsonProperty $o 'body')
            $notes = Update-OmaReleaseNotes -ExistingBody $body.Replace("`r`n", "`n") -TemplatePath $TemplatePath -Version $Version -Signed $Signed
            $notesFile = Write-OmaNotesFile $notes

            Assert-OmaStillDraft -Repo $Repo -Tag $Tag -Gh $Gh
            $r = & $Gh @('release', 'edit', $Tag, '--repo', $Repo, '--notes-file', $notesFile)
            if ($r.ExitCode -ne 0) {
                throw "updating the notes of the draft $Tag failed (gh exit code $($r.ExitCode)): $("$($r.Stderr)".Trim()); $retry"
            }
            Assert-OmaStillDraft -Repo $Repo -Tag $Tag -Gh $Gh
            $r = & $Gh @('release', 'upload', $Tag, '--repo', $Repo, '--clobber', $Setup, $Sums)
            if ($r.ExitCode -ne 0) {
                throw "uploading the assets to the draft $Tag failed (gh exit code $($r.ExitCode)): $("$($r.Stderr)".Trim()); the draft may hold a partial upload: $retry"
            }
            $action = 'updated'
        }
    } finally {
        if ($notesFile) { Remove-Item -LiteralPath $notesFile -Force -ErrorAction SilentlyContinue }
    }

    Assert-OmaRemoteAssets -Repo $Repo -Tag $Tag -Setup $Setup -Sums $Sums -Gh $Gh
    [pscustomobject]@{ Tag = $Tag; Action = $action; Setup = [IO.Path]::GetFileName($Setup) }
}

# --- preflight -----------------------------------------------------------------------------------

<#
.SYNOPSIS
  The release preflight (spec §4.1-4.2): run ref, signing mode and configuration, CI gate and,
  for a tag, a release not yet published. Returns and, with -OutputPath, appends the outputs
  signing, signpath-policy and verify-policy. HasToken and RequireSigning are workflow strings.
.DESCRIPTION
  Signing policy: a tag push is a real release (release-signing, verified with policy release);
  a workflow_dispatch is the rehearsal (test-signing, policy test), independently of
  REQUIRE_SIGNING. With signing disabled the verify policy is none.
#>
function Invoke-OmaReleasePreflight {
    [CmdletBinding()]
    param(
        [string]$EventName,
        [string]$Ref,
        [string]$Sha,
        [string]$Repo,
        [string]$HasToken,
        [string]$OrganizationId,
        [string]$RequireSigning,
        [string]$Tag,
        [string]$SignPathProjectSlug,
        [string]$CertificatesPath = $script:DefaultCertificatesPath,
        [string]$OutputPath,
        [scriptblock]$Gh = $script:DefaultGh
    )
    $hasTokenValue = ConvertFrom-OmaBoolText -Name 'HasToken' -Text $HasToken
    $run = Assert-OmaRunRef -EventName $EventName -Ref $Ref
    if ($run.Kind -eq 'tag') {
        if ($Tag -and $Tag -cne $run.Tag) { throw "-Tag $Tag does not match the ref $Ref" }
    } elseif ($Tag) {
        throw "-Tag is only valid for a tag push, got '$Tag' with $EventName"
    }

    $mode = Get-OmaSigningMode -HasToken $hasTokenValue -OrganizationId $OrganizationId -RequireSigning $RequireSigning
    $signpathPolicy = if ($run.Kind -eq 'tag') { 'release-signing' } else { 'test-signing' }
    $verifyPolicy = if ($run.Kind -eq 'tag') { 'release' } else { 'test' }
    if ($mode -eq 'enabled') {
        Assert-OmaSigningConfiguration -ProjectSlug $SignPathProjectSlug -CertificatesPath $CertificatesPath -Policy $verifyPolicy
    } else {
        $verifyPolicy = 'none'
        Write-Information '::notice::code signing is disabled (no SignPath credentials): this build is not signed'
    }

    Assert-OmaCiGreen -Repo $Repo -Sha $Sha -Gh $Gh
    if ($run.Kind -eq 'tag' -and (Get-OmaReleaseState -Repo $Repo -Tag $run.Tag -Gh $Gh) -eq 'published') {
        throw "release $($run.Tag) is already published"
    }

    $outputs = [ordered]@{ 'signing' = $mode; 'signpath-policy' = $signpathPolicy; 'verify-policy' = $verifyPolicy }
    if ($OutputPath) {
        $text = -join @($outputs.Keys | ForEach-Object { "$_=$($outputs[$_])`n" })
        [IO.File]::AppendAllText($OutputPath, $text, $script:Utf8)
    }
    $outputs
}

Export-ModuleMember -Function ConvertFrom-OmaBoolText, Get-OmaSigningMode, Assert-OmaRunRef, Assert-OmaSigningConfiguration,
    Assert-OmaCiGreen, Get-OmaReleaseState, Get-OmaFinalSetupName, Write-OmaSha256Sums, Assert-OmaRemoteAssets,
    Publish-OmaDraft, Invoke-OmaReleasePreflight
