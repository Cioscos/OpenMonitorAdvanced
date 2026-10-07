#Requires -Version 7
# Pester 5 tests for scripts/lib/OmaLicenses.psm1 (spec M6c §4): the pure parts of
# scripts/generate-licenses.ps1. Nothing here runs cargo, pnpm, dotnet or touches the network.
#   Import-Module Pester -RequiredVersion 5.7.1
#   Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI

BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaLicenses.psm1') -Force -ErrorAction Stop
    $script:fixtures = (Resolve-Path (Join-Path $PSScriptRoot 'fixtures\licenses')).Path
    $script:apache = "Apache License`r`nVersion 2.0, January 2004`r`n`r`nTERMS AND CONDITIONS  `r`n"
    $script:mit = "Permission is hereby granted, free of charge...`n"

    function script:New-Entry([string]$Eco, [string]$Name, [string]$Version, [string]$License, [object[]]$Texts) {
        [pscustomobject]@{
            Ecosystem = $Eco; Name = $Name; Version = $Version; License = $License
            Copyright = $null; Texts = $Texts
        }
    }
    function script:Text([string]$Title, [string]$Body) { [pscustomobject]@{ Title = $Title; Body = $Body } }
}

Describe 'Get-OmaNuGetRuntimePackages' {
    It 'NuGet runtime packages exclude analyzers and placeholders' {
        $r = @(Get-OmaNuGetRuntimePackages -AssetsJson (Join-Path $fixtures 'project.assets.json') -Target 'win-x64')
        ($r | ForEach-Object { "$($_.Id)/$($_.Version)" }) -join ',' |
            Should -BeExactly 'Native.Only/2.0.0,Runtime.Lib/1.2.0'
    }

    It 'accepts the full target key' {
        $r = @(Get-OmaNuGetRuntimePackages -AssetsJson (Join-Path $fixtures 'project.assets.json') -Target 'net10.0-windows/win-x64')
        $r.Count | Should -Be 2
    }

    It 'fails on a target that is not in the assets file' {
        { Get-OmaNuGetRuntimePackages -AssetsJson (Join-Path $fixtures 'project.assets.json') -Target 'linux-x64' } |
            Should -Throw -ExpectedMessage '*linux-x64*'
    }
}

Describe 'Merge-OmaLicenseSections' {
    It 'Merge sorts entries and deduplicates standard texts' {
        $sections = @(
            [pscustomobject]@{ Ecosystem = '.NET'; Entries = @(
                    (New-Entry '.NET' 'Zeta.Pkg' '1.0.0' 'Apache-2.0' @((Text 'Apache-2.0' $apache))),
                    (New-Entry '.NET' 'alpha.pkg' '2.0.0' 'Apache-2.0' @((Text 'Apache-2.0' ($apache -replace "`r`n", "`n"))))
                )
            }
        )
        $m = Merge-OmaLicenseSections -Sections $sections
        @($m.Texts).Count | Should -Be 1
        $m.Texts[0].Label | Should -BeExactly 'Apache-2.0 (alpha.pkg)'
        $entries = @($m.Sections[0].Entries)
        ($entries | ForEach-Object Name) -join ',' | Should -BeExactly 'alpha.pkg,Zeta.Pkg'
        $entries | ForEach-Object { $_.Refs -join ',' | Should -BeExactly 'Apache-2.0 (alpha.pkg)' }
    }

    It 'labels each text with its title and first user, and orders sections by ecosystem' {
        $sections = @(
            [pscustomobject]@{ Ecosystem = 'JavaScript'; Entries = @((New-Entry 'JavaScript' 'b' '1.0.0' 'MIT' @((Text 'MIT' "Copyright B`n$mit")))) },
            [pscustomobject]@{ Ecosystem = 'Rust'; Entries = @((New-Entry 'Rust' 'a' '1.0.0' 'MIT' @((Text 'MIT' "Copyright A`n$mit")))) }
        )
        $m = Merge-OmaLicenseSections -Sections $sections
        ($m.Sections | ForEach-Object Ecosystem) -join ',' | Should -BeExactly 'Rust,JavaScript'
        ($m.Texts | ForEach-Object Label) -join ',' | Should -BeExactly 'MIT (a),MIT (b)'
        $m.Sections[0].Entries[0].Refs | Should -BeExactly 'MIT (a)'
        $m.Texts[0].Body | Should -BeLike 'Copyright A*'
    }

    It 'puts the programs shipped with the service last, under their own title' {
        $sections = @(
            [pscustomobject]@{ Ecosystem = 'Programs'; Entries = @((New-Entry 'Programs' 'PresentMon' '2.6.0' 'MIT' @((Text 'MIT' "Copyright Intel`n$mit")))) },
            [pscustomobject]@{ Ecosystem = '.NET'; Entries = @((New-Entry '.NET' 'm' '1.0.0' 'MIT' @((Text 'MIT' $mit)))) }
        )
        $m = Merge-OmaLicenseSections -Sections $sections
        ($m.Sections | ForEach-Object Ecosystem) -join ',' | Should -BeExactly '.NET,Programs'
        $m.Sections[1].Title | Should -BeExactly 'Programs shipped with the service'
    }

    It 'puts the adapted source code after the programs, under its own title' {
        $sections = @(
            [pscustomobject]@{ Ecosystem = 'Adapted'; Entries = @((New-Entry 'Adapted' 'OpenDCDiag' '9957c45b' 'Apache-2.0' @((Text 'Apache-2.0' 'Apache text')))) },
            [pscustomobject]@{ Ecosystem = 'Programs'; Entries = @((New-Entry 'Programs' 'PresentMon' '2.6.0' 'MIT' @((Text 'MIT' $mit)))) },
            [pscustomobject]@{ Ecosystem = 'Rust'; Entries = @((New-Entry 'Rust' 'a' '1.0.0' 'MIT' @((Text 'MIT' $mit)))) }
        )
        $m = Merge-OmaLicenseSections -Sections $sections
        ($m.Sections | ForEach-Object Ecosystem) -join ',' | Should -BeExactly 'Rust,Programs,Adapted'
        $m.Sections[2].Title | Should -BeExactly 'Source code adapted into OpenMonitor Advanced'
    }

    It 'renders memtest_vulkan next to OpenDCDiag with its zlib text and copyright' {
        $zlib = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\licenses\Zlib.txt'))
        $zlib | Should -Match 'Copyright \(c\) 2022 galkinvv by GpuZelenograd'
        $e = [pscustomobject]@{ Ecosystem = 'Adapted'; Name = 'memtest_vulkan'; Version = 'fd9ff59c'; License = 'Zlib'
            Copyright = '2022 galkinvv by GpuZelenograd'; Texts = @((Text 'Zlib' $zlib)) }
        $m = Merge-OmaLicenseSections -Sections @(
            [pscustomobject]@{ Ecosystem = 'Adapted'; Entries = @((New-Entry 'Adapted' 'OpenDCDiag' '9957c45b' 'Apache-2.0' @((Text 'Apache-2.0' $apache)))) },
            [pscustomobject]@{ Ecosystem = 'Adapted'; Entries = @($e) })
        ($m.Sections[0].Entries | ForEach-Object Name) -join ',' | Should -BeExactly 'memtest_vulkan,OpenDCDiag'
        $m.Sections[0].Entries[0].Copyright | Should -Contain '2022 galkinvv by GpuZelenograd'
    }

    It 'labels a text marked neutral by its title alone, whoever uses it first' {
        $sections = @(
            [pscustomobject]@{ Ecosystem = 'Adapted'; Entries = @((New-Entry 'Adapted' 'OpenDCDiag' '9957c45b' 'Apache-2.0' @(
                            [pscustomobject]@{ Title = 'Apache-2.0'; Body = $apache; Neutral = $true }))) },
            [pscustomobject]@{ Ecosystem = 'Rust'; Entries = @((New-Entry 'Rust' 'dunce' '1.0.5' 'Apache-2.0' @((Text 'Apache-2.0' $apache)))) }
        )
        $m = Merge-OmaLicenseSections -Sections $sections
        ($m.Texts | ForEach-Object Label) -join ',' | Should -BeExactly 'Apache-2.0'
        $m.Sections | ForEach-Object { $_.Entries[0].Refs -join ',' | Should -BeExactly 'Apache-2.0' }
    }

    It 'keeps existing labels when a dependency is added' {
        $base = @(
            (New-Entry 'Rust' 'm' '1.0.0' 'MIT' @((Text 'MIT' "Copyright M`n$mit"))),
            (New-Entry 'Rust' 'z' '1.0.0' 'MIT' @((Text 'MIT' "Copyright Z`n$mit")))
        )
        $before = Merge-OmaLicenseSections -Sections @([pscustomobject]@{ Ecosystem = 'Rust'; Entries = $base })
        $after = Merge-OmaLicenseSections -Sections @([pscustomobject]@{ Ecosystem = 'Rust'; Entries = @(
                    (New-Entry 'Rust' 'a' '1.0.0' 'MIT' @((Text 'MIT' "Copyright A`n$mit")))) + $base })
        ($before.Texts | ForEach-Object Label) -join ',' | Should -BeExactly 'MIT (m),MIT (z)'
        ($after.Texts | ForEach-Object Label) -join ',' | Should -BeExactly 'MIT (a),MIT (m),MIT (z)'
    }

    It 'disambiguates texts with the same title and first user name' {
        $m = Merge-OmaLicenseSections -Sections @([pscustomobject]@{ Ecosystem = 'Rust'; Entries = @(
                    (New-Entry 'Rust' 'w' '0.52.0' 'MIT' @((Text 'MIT' "Copyright 1`n$mit"))),
                    (New-Entry 'Rust' 'w' '0.59.0' 'MIT' @((Text 'MIT' "Copyright 2`n$mit"))),
                    (New-Entry 'Rust' 'v' '1.0.0' 'MIT' @((Text 'MIT' "Copyright 3`n$mit"), (Text 'MIT' "Copyright 4`n$mit")))
                ) })
        $labels = @($m.Texts | ForEach-Object Label)
        $labels.Count | Should -Be 4
        @($labels | Sort-Object -Unique).Count | Should -Be 4
        $labels | Should -Contain 'MIT (w 0.52.0)'
        $labels | Should -Contain 'MIT (w 0.59.0)'
        @($labels | Where-Object { $_ -like 'MIT (v 1.0.0 #*)' }).Count | Should -Be 2
    }

    It 'sorts versions numerically and merges duplicate entries' {
        $sections = @(
            [pscustomobject]@{ Ecosystem = 'Rust'; Entries = @(
                    (New-Entry 'Rust' 'x' '0.10.0' 'MIT' @((Text 'MIT' $mit))),
                    (New-Entry 'Rust' 'x' '0.9.1' 'MIT' @((Text 'MIT' $mit))),
                    (New-Entry 'Rust' 'x' '0.9.1' 'MIT' @((Text 'Apache-2.0' $apache)))
                )
            }
        )
        $m = Merge-OmaLicenseSections -Sections $sections
        ($m.Sections[0].Entries | ForEach-Object Version) -join ',' | Should -BeExactly '0.9.1,0.10.0'
        $m.Sections[0].Entries[0].Refs -join ',' | Should -BeExactly 'Apache-2.0 (x),MIT (x)'
    }
}

Describe 'ConvertTo-OmaLicenseText' {
    It 'Output is deterministic' {
        $entries = @(
            (New-Entry 'Rust' 'serde' '1.0.0' 'MIT OR Apache-2.0' @((Text 'Apache-2.0' $apache))),
            (New-Entry '.NET' 'MessagePack' '3.1.10' 'MIT' @((Text 'MIT' $mit))),
            (New-Entry 'JavaScript' 'uplot' '1.6.32' 'MIT' @((Text 'MIT' "Copyright Leon`r`n$mit")))
        )
        $a = ConvertTo-OmaLicenseText -Entries $entries
        $b = ConvertTo-OmaLicenseText -Entries @($entries[2], $entries[0], $entries[1])
        $b | Should -BeExactly $a
        $a | Should -Not -Match "`r"
        $a | Should -Not -Match ([char]0xFEFF)
        $a | Should -Match "(?m)^serde 1\.0\.0$"
        $a.EndsWith("`n") | Should -BeTrue
        $a.EndsWith("`n`n") | Should -BeFalse
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($a)
        $bytes[0] | Should -Not -Be 0xEF
    }
}

Describe 'Test-OmaLicenseAccepted' {
    It 'evaluates OR, AND, parentheses and WITH' {
        $ok = 'MIT', 'Apache-2.0', 'Apache-2.0 WITH LLVM-exception'
        Test-OmaLicenseAccepted -Expression 'MIT OR GPL-2.0-only' -Accepted $ok | Should -BeTrue
        Test-OmaLicenseAccepted -Expression 'MIT AND GPL-2.0-only' -Accepted $ok | Should -BeFalse
        Test-OmaLicenseAccepted -Expression '(MIT OR Zlib) AND Apache-2.0' -Accepted $ok | Should -BeTrue
        Test-OmaLicenseAccepted -Expression 'Apache-2.0 WITH LLVM-exception' -Accepted $ok | Should -BeTrue
        Test-OmaLicenseAccepted -Expression 'GPL-2.0-only WITH Classpath-exception-2.0' -Accepted $ok | Should -BeFalse
        Test-OmaLicenseAccepted -Expression 'MIT/Apache-2.0' -Accepted $ok | Should -BeTrue
    }
}

Describe 'Test-OmaPinnedText' {
    It 'compares texts ignoring line ends, BOM and trailing whitespace' {
        $pinned = Join-Path $TestDrive 'pinned.txt'
        $same = Join-Path $TestDrive 'same.txt'
        $other = Join-Path $TestDrive 'other.txt'
        [IO.File]::WriteAllText($pinned, "Notices`nxxHash`n", [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($same, "Notices  `r`nxxHash`r`n", [Text.UTF8Encoding]::new($true))
        [IO.File]::WriteAllText($other, "Notices`nxxHash`nmimalloc`n", [Text.UTF8Encoding]::new($false))
        Test-OmaPinnedText -Pinned $pinned -Current $same | Should -BeTrue
        Test-OmaPinnedText -Pinned $pinned -Current $other | Should -BeFalse
    }
}

Describe 'Get-OmaNuGetPackageLicense' {
    BeforeAll {
        $script:pkgRoot = Join-Path $TestDrive 'packages'
        function script:New-Package([string]$Id, [string]$Version, [string]$LicenseXml, [hashtable]$Files = @{}) {
            $dir = Join-Path $pkgRoot "$($Id.ToLowerInvariant())\$($Version.ToLowerInvariant())"
            New-Item -ItemType Directory -Force $dir | Out-Null
            $nuspec = "<?xml version=`"1.0`"?><package xmlns=`"http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd`"><metadata><id>$Id</id><version>$Version</version>$LicenseXml<copyright>(c) $Id authors</copyright></metadata></package>"
            Set-Content -LiteralPath (Join-Path $dir "$($Id.ToLowerInvariant()).nuspec") -Value $nuspec
            foreach ($f in $Files.Keys) { Set-Content -LiteralPath (Join-Path $dir $f) -Value $Files[$f] -NoNewline }
        }
        New-Package 'Expr.Pkg' '1.0.0' '<license type="expression">MIT</license>'
        New-Package 'File.Pkg' '2.0.0' '<license type="file">LICENSE.txt</license>' @{ 'LICENSE.txt' = 'own terms' }
        New-Package 'Loose.Pkg' '3.0.0' '<license type="expression">MIT</license>' @{ 'LICENSE.TXT' = 'loose terms' }
        New-Package 'Url.Pkg' '4.0.0' '<licenseUrl>https://example.invalid/terms</licenseUrl>'
        New-Package 'Notices.Pkg' '5.0.0' '<license type="expression">MIT</license>' @{ 'Third-Party-Notices.TXT' = 'component notices' }
    }

    It 'reads the third-party notices file of the package, or null' {
        (Get-OmaNuGetPackageLicense -PackagesRoot $pkgRoot -Id 'Notices.Pkg' -Version '5.0.0').Notices |
            Should -BeExactly 'component notices'
        (Get-OmaNuGetPackageLicense -PackagesRoot $pkgRoot -Id 'Expr.Pkg' -Version '1.0.0').Notices |
            Should -BeNullOrEmpty
    }

    It 'reads the expression and the copyright from the nuspec' {
        $l = Get-OmaNuGetPackageLicense -PackagesRoot $pkgRoot -Id 'Expr.Pkg' -Version '1.0.0'
        $l.License | Should -BeExactly 'MIT'
        $l.Copyright | Should -BeExactly '(c) Expr.Pkg authors'
        $l.Text | Should -BeNullOrEmpty
    }

    It 'prefers the licence file of the package' {
        (Get-OmaNuGetPackageLicense -PackagesRoot $pkgRoot -Id 'File.Pkg' -Version '2.0.0' -Overrides @{ 'File.Pkg' = 'Apache-2.0' }).Text |
            Should -BeExactly 'own terms'
        $l = Get-OmaNuGetPackageLicense -PackagesRoot $pkgRoot -Id 'Loose.Pkg' -Version '3.0.0'
        $l.Text | Should -BeExactly 'loose terms'
        $l.License | Should -BeExactly 'MIT'
    }

    It 'needs an override when the nuspec has no expression' {
        { Get-OmaNuGetPackageLicense -PackagesRoot $pkgRoot -Id 'Url.Pkg' -Version '4.0.0' } |
            Should -Throw -ExpectedMessage '*Url.Pkg*'
        (Get-OmaNuGetPackageLicense -PackagesRoot $pkgRoot -Id 'Url.Pkg' -Version '4.0.0' -Overrides @{ 'Url.Pkg' = 'MIT' }).License |
            Should -BeExactly 'MIT'
    }
}
