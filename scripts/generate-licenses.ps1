<#
.SYNOPSIS
  Generates THIRD_PARTY_LICENSES.txt (spec M6c §4); with -Check, fails if the committed file
  is out of date.
.DESCRIPTION
    pwsh scripts/generate-licenses.ps1 [-Check]

  1. Rust: `cargo about generate --workspace --target x86_64-pc-windows-msvc about.hbs`
     (cargo-about $CargoAboutVersion, configured by about.toml: no dev or build dependencies).
     The workspace's own crates are left out.
  2. JavaScript: `pnpm build` in app/ with OMA_LICENSE_MANIFEST set, so the Vite plugin in
     app/vite.config.ts lists the node_modules packages that have code in the bundle; version,
     licence and LICENSE* files come from each package's directory.
  3. .NET: `dotnet restore service/OpenMonitorAdvanced.Service -r win-x64`, then the packages
     of obj/project.assets.json with a runtime or native asset for win-x64; licence file from
     the package, else the standard text of its nuspec expression (scripts/licenses/). Plus the
     .NET runtime, which the self-contained single-file service carries (MIT), with its
     third-party notices from a pinned copy (a warning, not a failure, when the restored
     runtime pack has different ones).
  Every licence expression must be satisfiable with the `accepted` list of about.toml. The
  output is UTF-8 without BOM, LF, with no date and no local path, so it is deterministic.
  -Check writes to a temporary folder and compares; exit code 1 lists the differing lines.
  Requires cargo-about $CargoAboutVersion:
    cargo install cargo-about --locked --version 0.9.2 --features cli
#>
#Requires -Version 7

[CmdletBinding()]
param(
    [switch]$Check,
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'lib\OmaCommon.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'lib\OmaLicenses.psm1') -Force

# Pinned here and in .github/workflows/ci.yml (job installer).
$CargoAboutVersion = '0.9.2'

# NuGet packages whose nuspec has no SPDX expression; each one reviewed by hand.
$NuGetOverrides = @{
    # nuspec: <license type="file">LICENSE.txt</license>, the Apache License 2.0 with the
    # HidSharp copyright notice; the file itself is copied.
    'HidSharp'               = 'Apache-2.0'
    # nuspec: licenseUrl https://go.microsoft.com/fwlink/?linkid=869050, which redirects to
    # https://github.com/mono/mono/blob/main/LICENSE: the Mono class libraries are MIT.
    'Mono.Posix.NETStandard' = 'MIT'
}

$utf8 = [Text.UTF8Encoding]::new($false)
$licensesDir = Join-Path $PSScriptRoot 'licenses'

# THIRD-PARTY-NOTICES.TXT of the .NET runtime, copied (LF, no BOM) from the runtime pack
# microsoft.netcore.app.runtime.win-x64 10.0.11. Pinned so the output does not depend on the
# SDK patch level; the script warns when the restored pack's file differs.
$PinnedRuntimeNotices = Join-Path $licensesDir 'dotnet-runtime-THIRD-PARTY-NOTICES.txt'
$PinnedRuntimeNoticesVersion = '10.0.11'

function Get-AcceptedLicenses {
    $toml = [IO.File]::ReadAllText((Join-Path $RepoRoot 'about.toml'))
    $noComments = ($toml -split "\r?\n" | ForEach-Object { ($_ -replace '#.*$', '') }) -join "`n"
    $m = [regex]::Match($noComments, '(?ms)^accepted\s*=\s*\[(.*?)\]')
    if (-not $m.Success) { throw 'about.toml: no top-level accepted list' }
    @([regex]::Matches($m.Groups[1].Value, '"([^"]+)"') | ForEach-Object { $_.Groups[1].Value })
}

function Get-StandardText([string]$Id) {
    $path = Join-Path $licensesDir "$Id.txt"
    if (-not (Test-Path -LiteralPath $path)) { throw "no standard text for $Id in scripts/licenses/" }
    [pscustomobject]@{ Title = $Id; Body = [IO.File]::ReadAllText($path) }
}

# Standard texts for a simple expression (one id, or ids joined by OR/AND without WITH).
function Get-StandardTexts([string]$Expression, [string[]]$Accepted) {
    $ids = @($Expression -replace '[()]', ' ' -split '\s+(?:OR|AND)\s+|\s+' | Where-Object { $_ -and $_ -notin 'OR', 'AND' })
    if ($Expression -match '\bWITH\b') { throw "no standard text for '$Expression'" }
    # For an OR, one accepted licence is enough: the first accepted one, in about.toml order.
    if ($Expression -match '\bOR\b' -and $Expression -notmatch '\bAND\b') {
        $first = $Accepted | Where-Object { $_ -in $ids } | Select-Object -First 1
        return @(Get-StandardText $first)
    }
    @($ids | Sort-Object -Unique | ForEach-Object { Get-StandardText $_ })
}

function Assert-Accepted([string]$What, [string]$Expression, [string[]]$Accepted) {
    if (-not (Test-OmaLicenseAccepted -Expression $Expression -Accepted $Accepted)) {
        throw "$What is licensed under '$Expression', which is not in the accepted list of about.toml"
    }
}

function Get-RustEntries([string]$Temp) {
    $version = (Invoke-OmaNative -FilePath 'cargo' -ArgumentList @('about', '--version') -WorkingDirectory $RepoRoot -AllowFailure)
    if ($version.ExitCode -ne 0 -or $version.Stdout.Trim() -ne "cargo-about $CargoAboutVersion") {
        throw "cargo-about $CargoAboutVersion is required (found: '$($version.Stdout.Trim())'): cargo install cargo-about --locked --version $CargoAboutVersion --features cli"
    }
    $json = Join-Path $Temp 'rust.json'
    Invoke-OmaNative -FilePath 'cargo' -WorkingDirectory $RepoRoot -ArgumentList @(
        'about', 'generate', '--workspace', '--target', 'x86_64-pc-windows-msvc', 'about.hbs', '-o', $json) | Out-Null
    $metadata = (Invoke-OmaNative -FilePath 'cargo' -WorkingDirectory $RepoRoot -ArgumentList @(
            'metadata', '--no-deps', '--format-version', '1')).Stdout | ConvertFrom-Json
    $own = @($metadata.packages | ForEach-Object name)
    foreach ($license in ([IO.File]::ReadAllText($json, $utf8) | ConvertFrom-Json)) {
        foreach ($crate in $license.used_by) {
            if ($crate.name -in $own) { continue }
            [pscustomobject]@{
                Ecosystem = 'Rust'; Name = $crate.name; Version = [string]$crate.version
                License = ($crate.license -replace '\s+', ' ').Trim(); Copyright = $null
                Texts = @([pscustomobject]@{ Title = $license.id; Body = $license.text })
            }
        }
    }
}

function Get-JsEntries([string]$Temp, [string[]]$Accepted) {
    $manifest = Join-Path $Temp 'js-manifest.json'
    $previous = $env:OMA_LICENSE_MANIFEST
    $env:OMA_LICENSE_MANIFEST = $manifest
    try {
        Invoke-OmaNative -FilePath 'pnpm' -ArgumentList @('build') -WorkingDirectory (Join-Path $RepoRoot 'app') | Out-Null
    } finally {
        $env:OMA_LICENSE_MANIFEST = $previous
    }
    foreach ($pkg in ([IO.File]::ReadAllText($manifest) | ConvertFrom-Json)) {
        $meta = [IO.File]::ReadAllText((Join-Path $pkg.dir 'package.json')) | ConvertFrom-Json
        $expr = if ($meta.license -is [string]) { $meta.license } else { $meta.license.type }
        Assert-Accepted "$($pkg.name) $($meta.version)" $expr $Accepted
        $files = @(Get-ChildItem -LiteralPath $pkg.dir -File | Where-Object { $_.Name -match '^licen[cs]e' } |
                Sort-Object -Property { $_.Name.ToUpperInvariant() })
        $texts = if ($files.Count -gt 0) {
            foreach ($f in $files) {
                $title = if ($f.Name -match 'apache') { 'Apache-2.0' }
                elseif ($f.Name -match 'mit') { 'MIT' }
                elseif ($expr -match '^[\w.+-]+$') { $expr }
                else { "LicenseRef-$($pkg.name)" }
                [pscustomobject]@{ Title = $title; Body = [IO.File]::ReadAllText($f.FullName) }
            }
        } else {
            Get-StandardTexts $expr $Accepted
        }
        [pscustomobject]@{
            Ecosystem = 'JavaScript'; Name = $pkg.name; Version = $meta.version; License = $expr
            Copyright = $null; Texts = @($texts)
        }
    }
}

function Get-NuGetEntries([string[]]$Accepted) {
    $project = Join-Path $RepoRoot 'service\OpenMonitorAdvanced.Service'
    Invoke-OmaNative -FilePath 'dotnet' -ArgumentList @('restore', $project, '-r', 'win-x64') -WorkingDirectory $RepoRoot | Out-Null
    $assetsPath = Join-Path $project 'obj\project.assets.json'
    $assets = [IO.File]::ReadAllText($assetsPath) | ConvertFrom-Json -AsHashtable
    $packagesRoot = @($assets['packageFolders'].Keys)[0]
    foreach ($p in Get-OmaNuGetRuntimePackages -AssetsJson $assetsPath -Target 'win-x64') {
        $lic = Get-OmaNuGetPackageLicense -PackagesRoot $packagesRoot -Id $p.Id -Version $p.Version -Overrides $NuGetOverrides
        Assert-Accepted "$($p.Id) $($p.Version)" $lic.License $Accepted
        $texts = if ($lic.Text) {
            $title = if ($lic.License -match '^[\w.+-]+$') { $lic.License } else { "LicenseRef-$($p.Id)" }
            @([pscustomobject]@{ Title = $title; Body = $lic.Text })
        } else {
            Get-StandardTexts $lic.License $Accepted
        }
        if ($lic.Notices) {
            $texts = @($texts) + [pscustomobject]@{ Title = 'Third-party notices'; Body = $lic.Notices }
        }
        [pscustomobject]@{
            Ecosystem = '.NET'; Name = $p.Id; Version = $p.Version; License = $lic.License
            Copyright = $lic.Copyright; Texts = @($texts)
        }
    }
    # The self-contained publish carries the .NET runtime of the target framework, with its
    # third-party notices (BSD-style components such as xxHash, Brotli, mimalloc, fmtlib).
    $framework = @($assets['targets'].Keys | Where-Object { $_ -notmatch '/' })[0]
    if ($framework -notmatch '^net(\d+\.\d+)') { throw "unexpected target framework '$framework' in project.assets.json" }
    $runtimeVersion = $Matches[1]
    Test-RuntimeNotices $assets $packagesRoot
    [pscustomobject]@{
        Ecosystem = '.NET'; Name = '.NET runtime'; Version = $runtimeVersion; License = 'MIT'
        Copyright = '.NET Foundation and Contributors'
        Texts = @(
            (Get-StandardText 'MIT'),
            [pscustomobject]@{ Title = 'Third-party notices'; Body = [IO.File]::ReadAllText($PinnedRuntimeNotices) }
        )
    }
}

# Warns (never fails, so the output stays the same on every machine) when the THIRD-PARTY-NOTICES
# of the restored runtime pack differ from the pinned copy, which then needs a review and update.
function Test-RuntimeNotices($Assets, [string]$PackagesRoot) {
    $pack = $null
    foreach ($fw in $Assets['project']['frameworks'].Values) {
        foreach ($d in @($fw['downloadDependencies'])) {
            if ($d -and $d['name'] -eq 'Microsoft.NETCore.App.Runtime.win-x64') { $pack = $d['version'] -replace '[\[\]\s]', '' -replace ',.*$', '' }
        }
    }
    $message = $null
    if (-not $pack) {
        $message = 'the .NET runtime pack is not in project.assets.json: cannot compare its THIRD-PARTY-NOTICES with the pinned copy'
    } else {
        $file = Join-Path $PackagesRoot "microsoft.netcore.app.runtime.win-x64\$pack\THIRD-PARTY-NOTICES.TXT"
        if (-not (Test-Path -LiteralPath $file)) {
            $message = "THIRD-PARTY-NOTICES.TXT not found in the .NET runtime pack $pack"
        } elseif (-not (Test-OmaPinnedText -Pinned $PinnedRuntimeNotices -Current $file)) {
            $message = "the THIRD-PARTY-NOTICES.TXT of the .NET runtime pack $pack differs from scripts/licenses/dotnet-runtime-THIRD-PARTY-NOTICES.txt (pinned from $PinnedRuntimeNoticesVersion): review it and update the pinned copy"
        }
    }
    if ($message) {
        if ($env:GITHUB_ACTIONS -eq 'true') { [Console]::Out.WriteLine("::warning title=Third-party licences::$message") }
        else { Write-Warning $message }
    }
}

$temp = Join-Path ([IO.Path]::GetTempPath()) "oma-licenses-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    $accepted = Get-AcceptedLicenses
    $entries = @(
        Get-RustEntries $temp
        Get-JsEntries $temp $accepted
        Get-NuGetEntries $accepted
    )
    $text = ConvertTo-OmaLicenseText -Entries $entries
    $target = Join-Path $RepoRoot 'THIRD_PARTY_LICENSES.txt'

    if (-not $Check) {
        [IO.File]::WriteAllBytes($target, $utf8.GetBytes($text))
        Write-Output "wrote THIRD_PARTY_LICENSES.txt ($(@($entries).Count) entries)"
        exit 0
    }

    $fresh = Join-Path $temp 'THIRD_PARTY_LICENSES.txt'
    [IO.File]::WriteAllBytes($fresh, $utf8.GetBytes($text))
    $current = if (Test-Path -LiteralPath $target) { [IO.File]::ReadAllBytes($target) } else { [byte[]]@() }
    if ([Convert]::ToHexString($current) -eq [Convert]::ToHexString([IO.File]::ReadAllBytes($fresh))) {
        Write-Output 'licence check: ok'
        exit 0
    }
    [Console]::Error.WriteLine('THIRD_PARTY_LICENSES.txt is out of date; run pwsh scripts/generate-licenses.ps1 and commit it.')
    $old = $utf8.GetString($current) -split "`n"
    $new = $text -split "`n"
    $diff = @(Compare-Object -ReferenceObject $old -DifferenceObject $new -CaseSensitive)
    foreach ($d in ($diff | Select-Object -First 50)) {
        $sign = if ($d.SideIndicator -eq '=>') { '+' } else { '-' }
        [Console]::Error.WriteLine("$sign $($d.InputObject)")
    }
    if ($diff.Count -gt 50) { [Console]::Error.WriteLine("... and $($diff.Count - 50) more lines") }
    if ($diff.Count -eq 0) { [Console]::Error.WriteLine('(the files differ in line ends or encoding)') }
    exit 1
} catch {
    [Console]::Error.WriteLine("generate-licenses: $($_.Exception.Message)")
    exit 1
} finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
