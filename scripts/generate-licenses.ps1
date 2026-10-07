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
  4. Programs: Intel PresentMon 2.6.0, shipped unmodified with the service, and cereal 1.3.2,
     compiled into it; texts from pinned copies in scripts/licenses/.
  5. Adapted: source code of OpenDCDiag (Apache-2.0, M8a1) and memtest_vulkan (zlib, M8b1) adapted
     into crates/oma-load. The
     FIRESTARTER code adapted there is GPL-3.0-or-later, our own licence: it is attributed in
     THIRD_PARTY_NOTICES.md only.
  6. Fonts: Orbitron and Share Tech Mono (OFL-1.1, M8a2), bundled in app/src/assets/fonts/; texts
     (with the copyright line) from the OFL.txt of the pinned google/fonts commit, in scripts/licenses/.
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

# Intel's PresentMon console, which the installer ships unmodified next to the service (M7b), and
# the third-party code compiled into it. Entries added by hand, like the .NET runtime: the texts
# are copies (LF, no BOM) under scripts/licenses/, reviewed when the pinned PresentMon changes
# (scripts/lib/OmaPresentMonPins.psm1).
#   - PresentMon: LICENSE.txt of tag v2.6.0 of GameTechDev/PresentMon (MIT).
#   - cereal: the strings of PresentMon-2.6.0-x64.exe name cereal and none of boost, CLI11 or
#     moodycamel (concurrentqueue); vcpkg.json of v2.6.0 takes cereal from the vcpkg baseline
#     120deac3, i.e. cereal 1.3.2, whose LICENSE (tag v1.3.2 of USCiLab/cereal) is BSD-3-Clause.
function Get-ProgramEntries([string[]]$Accepted) {
    $programs = @(
        [pscustomobject]@{
            Ecosystem = 'Programs'; Name = 'PresentMon'; Version = '2.6.0'; License = 'MIT'
            Copyright = '2017-2024 Intel Corporation'; TextTitle = 'MIT'; File = 'PresentMon-2.6.0-LICENSE.txt'
        }
        [pscustomobject]@{
            Ecosystem = 'Programs'; Name = 'cereal (in PresentMon)'; Version = '1.3.2'; License = 'BSD-3-Clause'
            Copyright = '2013-2022, Randolph Voorhies, Shane Grant'; TextTitle = 'BSD-3-Clause'; File = 'cereal-1.3.2-LICENSE.txt'
        }
    )
    foreach ($p in $programs) {
        Assert-Accepted "$($p.Name) $($p.Version)" $p.License $Accepted
        [pscustomobject]@{
            Ecosystem = $p.Ecosystem; Name = $p.Name; Version = $p.Version; License = $p.License
            Copyright = $p.Copyright
            Texts = @([pscustomobject]@{ Title = $p.TextTitle; Body = [IO.File]::ReadAllText((Join-Path $licensesDir $p.File)) })
        }
    }
}

# Third-party source adapted into our own crates (M8a1), written by hand like the programs above. Only
# what is not under our own licence appears here: OpenDCDiag at commit 9957c45b (Copyright 2022 Intel
# Corporation, Apache-2.0), adapted in crates/oma-load/src/verify.rs and kernel.rs, and memtest_vulkan at
# commit fd9ff59c (Copyright (c) 2022 galkinvv by GpuZelenograd, zlib), adapted in gpu/vram.rs. FIRESTARTER
# (GPL-3.0-or-later) is covered by LICENSE and THIRD_PARTY_NOTICES.md.
function Get-AdaptedEntries([string[]]$Accepted) {
    Assert-Accepted 'OpenDCDiag 9957c45b' 'Apache-2.0' $Accepted
    Assert-Accepted 'memtest_vulkan fd9ff59c' 'Zlib' $Accepted
    [pscustomobject]@{
        Ecosystem = 'Adapted'; Name = 'OpenDCDiag'; Version = '9957c45b'; License = 'Apache-2.0'
        Copyright = '2022 Intel Corporation'
        Texts = @([pscustomobject]@{
                Title = 'Apache-2.0'; Body = [IO.File]::ReadAllText((Join-Path $licensesDir 'Apache-2.0.txt')); Neutral = $true
            })
    }
    # M8b1: memtest_vulkan, adapted in crates/oma-load (S4); the text of the pinned commit includes the copyright line.
    [pscustomobject]@{
        Ecosystem = 'Adapted'; Name = 'memtest_vulkan'; Version = 'fd9ff59c'; License = 'Zlib'
        Copyright = '2022 galkinvv by GpuZelenograd'
        Texts = @([pscustomobject]@{ Title = 'Zlib'; Body = [IO.File]::ReadAllText((Join-Path $licensesDir 'Zlib.txt')) })
    }
}

# Fonts bundled in the app (M8a2): github.com/google/fonts at commit 7085eb89a950e85db5b166b7a58d414544b4140c,
# files and SHA-256 in app/src/styles/fonts.css. The licence file of each starts with its copyright line.
function Get-FontEntries([string[]]$Accepted) {
    $fonts = @(
        @{ Name = 'Orbitron'; File = 'Orbitron-OFL.txt'; Copyright = '2018 The Orbitron Project Authors (https://github.com/theleagueof/orbitron), with Reserved Font Name "Orbitron"' }
        @{ Name = 'Share Tech Mono'; File = 'ShareTechMono-OFL.txt'; Copyright = '2012 Carrois Type Design, Ralph du Carrois (post@carrois.com www.carrois.com), with Reserved Font Name "Share"' }
    )
    foreach ($f in $fonts) {
        Assert-Accepted $f.Name 'OFL-1.1' $Accepted
        [pscustomobject]@{
            Ecosystem = 'Fonts'; Name = $f.Name; Version = '7085eb89'; License = 'OFL-1.1'
            Copyright = $f.Copyright
            Texts = @([pscustomobject]@{ Title = 'OFL-1.1'; Body = [IO.File]::ReadAllText((Join-Path $licensesDir $f.File)) })
        }
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
        Get-ProgramEntries $accepted
        Get-AdaptedEntries $accepted
        Get-FontEntries $accepted
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
