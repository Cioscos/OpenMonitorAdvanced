#Requires -Version 7
# Pester 5 tests for the metadata gate of scripts/build-installer-payload.ps1 (step 4), for the
# PawnIO pins shared with the verifier (scripts/lib/OmaPawnIoPins.psm1) and for the PresentMon
# staging and pins (scripts/lib/OmaPresentMonPins.psm1, step 6). The script runs against a fake
# `dotnet` under $TestDrive: nothing is published, nothing is downloaded and PresentMon never runs.
#   Import-Module Pester -RequiredVersion 5.7.1
#   Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI

BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaCommon.psm1') -Force -ErrorAction Stop
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaPawnIoPins.psm1') -Force -ErrorAction Stop
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaPresentMonPins.psm1') -Force -ErrorAction Stop
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaSigning.psm1') -Force -ErrorAction Stop
    $script:repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
    $script:payloadScript = Join-Path $repoRoot 'scripts\build-installer-payload.ps1'
    $script:pwshExe = (Get-Process -Id $PID).Path

    # Runs the payload script with a fake dotnet (a .ps1) that "publishes" $Source as
    # oma-service.exe, freshly dated, into the -o directory.
    function Invoke-PayloadWith([string]$Source) {
        $dir = Join-Path $TestDrive ([guid]::NewGuid().ToString('N').Substring(0, 8))
        $project = Join-Path $dir 'project'
        $out = Join-Path $dir 'payload'
        New-Item -ItemType Directory -Force $project, $out | Out-Null
        $fake = Join-Path $dir 'fake-dotnet.ps1'
        Set-Content -LiteralPath $fake -Value @"
`$o = `$args[[array]::IndexOf(`$args, '-o') + 1]
New-Item -ItemType Directory -Force `$o | Out-Null
`$exe = Join-Path `$o 'oma-service.exe'
Copy-Item -LiteralPath '$Source' -Destination `$exe
(Get-Item -LiteralPath `$exe).LastWriteTime = Get-Date
'Build succeeded.'
exit 0
"@
        $r = Invoke-OmaNative -FilePath $pwshExe -AllowFailure -ArgumentList @(
            '-NoProfile', '-NonInteractive', '-File', $payloadScript,
            '-DotnetExe', $fake, '-ServiceProject', $project, '-OutputRoot', $out)
        [pscustomobject]@{ ExitCode = $r.ExitCode; Output = "$($r.Stdout)`n$($r.Stderr)"; Out = $out }
    }
}

Describe 'service metadata gate (step 4)' {
    It 'accepts the service metadata of the current version' {
        $vi = [pscustomobject]@{ ProductName = 'OpenMonitor Advanced'; ProductVersion = '1.2.3'; FileVersion = '1.2.3.0' }
        Test-OmaVersionInfo -VersionInfo $vi -Version '1.2.3' -Name 'oma-service.exe' | Should -BeNullOrEmpty
    }

    It 'rejects the old product name' {
        $vi = [pscustomobject]@{ ProductName = 'oma-service'; ProductVersion = '1.2.3'; FileVersion = '1.2.3.0' }
        (Test-OmaVersionInfo -VersionInfo $vi -Version '1.2.3' -Name 'oma-service.exe') |
            Should -Be @("oma-service.exe has ProductName 'oma-service', expected 'OpenMonitor Advanced'")
    }

    It 'rejects missing metadata' {
        $vi = [pscustomobject]@{ ProductName = $null; ProductVersion = $null; FileVersion = $null }
        $p = @(Test-OmaVersionInfo -VersionInfo $vi -Version '1.2.3' -Name 'oma-service.exe')
        $p.Count | Should -Be 3
        ($p -join "`n") | Should -BeLike "*ProductName '' (missing)*"
        @(Test-OmaVersionInfo -VersionInfo $null -Version '1.2.3' -Name 'oma-service.exe') |
            Should -Be @('oma-service.exe has no version information')
    }

    It 'fails the payload build when the published exe has another product name' {
        $r = Invoke-PayloadWith $pwshExe
        $r.ExitCode | Should -Be 1
        $r.Output | Should -BeLike "*oma-service.exe has ProductName 'PowerShell', expected 'OpenMonitor Advanced'*"
        Test-Path -LiteralPath (Join-Path $r.Out 'PawnIO_setup.exe') | Should -BeFalse
    }

    It 'fails the payload build when the published exe has no version information' {
        $notPe = Join-Path $TestDrive 'not-a-pe.exe'
        Set-Content -LiteralPath $notPe -Value 'not a PE'
        $r = Invoke-PayloadWith $notPe
        $r.ExitCode | Should -Be 1
        $r.Output | Should -BeLike "*ProductName '' (missing)*"
    }

    It 'the service project sets the product name and a plain product version' {
        $csproj = Get-Content -Raw (Join-Path $repoRoot 'service\OpenMonitorAdvanced.Service\OpenMonitorAdvanced.Service.csproj')
        $csproj | Should -Match '<Product>OpenMonitor Advanced</Product>'
        $csproj | Should -Match '<IncludeSourceRevisionInInformationalVersion>false</IncludeSourceRevisionInInformationalVersion>'
    }
}

Describe 'shared PawnIO pins' {
    It 'reads the hash from the single pinned file' {
        $pins = Get-OmaPawnIoPins
        $pins.Sha256 | Should -BeExactly (Get-Content -Raw (Join-Path $repoRoot 'app\src-tauri\nsis\pawnio.sha256')).Trim()
        $pins.Sha256 | Should -MatchExactly '^[0-9A-F]{64}$'
        $pins.SignerThumbprint | Should -MatchExactly '^[0-9A-F]{40}$'
        $pins.SignerSubject | Should -BeLike '*CN=namazso.eu*'
    }

    It 'keeps the signer pins in the pins module only' {
        $pins = Get-OmaPawnIoPins
        $thumb = [regex]::Escape($pins.SignerThumbprint)
        $subject = [regex]::Escape($pins.SignerSubject)
        foreach ($f in 'scripts\build-installer-payload.ps1', 'scripts\lib\OmaSigning.psm1', 'scripts\verify-signatures.ps1') {
            $text = Get-Content -Raw (Join-Path $repoRoot $f)
            $text | Should -Not -Match $thumb
            $text | Should -Not -Match $subject
        }
        (Get-Content -Raw (Join-Path $repoRoot 'scripts\build-installer-payload.ps1')) | Should -Match 'OmaPawnIoPins\.psm1'
        (Get-Content -Raw (Join-Path $repoRoot 'scripts\lib\OmaSigning.psm1')) | Should -Match 'OmaPawnIoPins\.psm1'
    }

    It 'the payload build rejects a PawnIO setup against the shared pins' {
        $out = Join-Path $TestDrive 'pawnio-only'
        New-Item -ItemType Directory -Force $out | Out-Null
        $source = Join-Path $TestDrive 'PawnIO_setup.exe'
        Set-Content -LiteralPath $source -Value 'not the pinned PawnIO setup'
        $r = Invoke-OmaNative -FilePath $pwshExe -AllowFailure -ArgumentList @(
            '-NoProfile', '-NonInteractive', '-File', $payloadScript, '-PawnIoOnly', '-PawnIoSource', $source, '-OutputRoot', $out)
        $r.ExitCode | Should -Be 1
        $want = (Get-OmaSha256 $source).ToUpperInvariant()
        $r.Stdout | Should -BeLike "*PawnIO_setup.exe rejected: SHA-256 $want, expected $((Get-OmaPawnIoPins).Sha256)*"
    }
}

Describe 'Test-OmaPawnIoSetup' {
    BeforeAll {
        $script:file = Join-Path $TestDrive 'PawnIO_setup.exe'
        Set-Content -LiteralPath $file -Value 'pawnio'
        $script:pins = [pscustomobject]@{
            Sha256           = (Get-OmaSha256 $file).ToUpperInvariant()
            SignerSubject    = 'CN=Pinned'
            SignerThumbprint = 'AA' * 20
        }
        function New-PawnSig([string]$Status = 'Valid', [string]$Subject = 'CN=Pinned', [string]$Thumb = ('AA' * 20)) {
            [pscustomobject]@{ Status = $Status; StatusMessage = "fake $Status"; SignerCertificate = [pscustomobject]@{ Subject = $Subject; Thumbprint = $Thumb } }
        }
    }

    It 'accepts the pinned hash and signer' {
        Test-OmaPawnIoSetup -Path $file -Pins $pins -SignatureProvider { param($Path) New-PawnSig } | Should -BeNullOrEmpty
    }

    It 'rejects another hash' {
        $other = [pscustomobject]@{ Sha256 = '0' * 64; SignerSubject = 'CN=Pinned'; SignerThumbprint = 'AA' * 20 }
        Test-OmaPawnIoSetup -Path $file -Pins $other -SignatureProvider { param($Path) New-PawnSig } | Should -BeLike 'SHA-256 *, expected 0000*'
    }

    It 'rejects a status other than Valid' {
        Test-OmaPawnIoSetup -Path $file -Pins $pins -SignatureProvider { param($Path) New-PawnSig -Status NotTrusted } | Should -BeLike '*NotTrusted*'
    }

    It 'rejects a valid signature by another signer' {
        Test-OmaPawnIoSetup -Path $file -Pins $pins -SignatureProvider { param($Path) New-PawnSig -Thumb ('BB' * 20) } | Should -BeLike "*$('BB' * 20)*"
        Test-OmaPawnIoSetup -Path $file -Pins $pins -SignatureProvider { param($Path) New-PawnSig -Subject 'CN=Pinned Fake' } | Should -BeLike '*CN=Pinned Fake*'
    }
}

Describe 'shared PresentMon pins' {
    It 'reads the hash from the single pinned file and pins the Intel signer' {
        $pins = Get-OmaPresentMonPins
        $pins.Sha256 | Should -BeExactly (Get-Content -Raw (Join-Path $repoRoot 'app\src-tauri\nsis\presentmon.sha256')).Trim()
        $pins.Sha256 | Should -MatchExactly '^[0-9A-F]{64}$'
        $pins.SignerSubject | Should -BeExactly 'CN=Intel Corporation, O=Intel Corporation, S=California, C=US'
    }

    It 'keeps the signer pin in the pins module only' {
        $subject = [regex]::Escape((Get-OmaPresentMonPins).SignerSubject)
        foreach ($f in 'scripts\build-installer-payload.ps1', 'scripts\lib\OmaSigning.psm1', 'scripts\verify-signatures.ps1') {
            Get-Content -Raw (Join-Path $repoRoot $f) | Should -Not -Match $subject
        }
        (Get-Content -Raw (Join-Path $repoRoot 'scripts\build-installer-payload.ps1')) | Should -Match 'OmaPresentMonPins\.psm1'
        (Get-Content -Raw (Join-Path $repoRoot 'scripts\lib\OmaSigning.psm1')) | Should -Match 'OmaPresentMonPins\.psm1'
    }

    It 'the payload build rejects a PresentMon against the shared pins and stages nothing' {
        $out = Join-Path $TestDrive 'presentmon-only'
        New-Item -ItemType Directory -Force $out | Out-Null
        $source = Join-Path $TestDrive 'PresentMon-2.6.0-x64.exe'
        Set-Content -LiteralPath $source -Value 'not the pinned PresentMon'
        $r = Invoke-OmaNative -FilePath $pwshExe -AllowFailure -ArgumentList @(
            '-NoProfile', '-NonInteractive', '-File', $payloadScript, '-PresentMonOnly', '-PresentMonSource', $source, '-OutputRoot', $out)
        $r.ExitCode | Should -Be 1
        $want = (Get-OmaSha256 $source).ToUpperInvariant()
        $r.Stdout | Should -BeLike "*PresentMon-2.6.0-x64.exe rejected: SHA-256 $want, expected $((Get-OmaPresentMonPins).Sha256)*"
        Test-Path -LiteralPath (Join-Path $out 'presentmon\PresentMon-2.6.0-x64.exe') | Should -BeFalse
        Test-Path -LiteralPath (Join-Path $out 'presentmon\PresentMon-2.6.0-x64.exe.partial') | Should -BeFalse
        # PresentMon only: neither the service nor PawnIO is touched.
        Test-Path -LiteralPath (Join-Path $out 'PawnIO_setup.exe') | Should -BeFalse
        Test-Path -LiteralPath (Join-Path $out 'service') | Should -BeFalse
    }

    It 'the payload script defaults to the official v2.6.0 release' {
        $text = Get-Content -Raw $payloadScript
        $text | Should -Match ([regex]::Escape('https://github.com/GameTechDev/PresentMon/releases/download/v2.6.0/PresentMon-2.6.0-x64.exe'))
        # No second copy of a hash in the script.
        $text | Should -Not -Match '[0-9A-Fa-f]{64}'
    }
}

Describe 'PresentMon staging' {
    BeforeAll {
        $script:intel = 'CN=Intel Corporation, O=Intel Corporation, S=California, C=US'
        function New-PmSig([string]$Status = 'Valid', [string]$Subject = $intel) {
            $cert = if ($Status -eq 'NotSigned') { $null } else { [pscustomobject]@{ Subject = $Subject; Thumbprint = 'CC' * 20 } }
            [pscustomobject]@{ Status = $Status; StatusMessage = "fake $Status"; SignerCertificate = $cert }
        }
        # A fake PresentMon (a text file, never executed) and the pins that match it.
        function New-PmCase {
            $dir = Join-Path $TestDrive ([guid]::NewGuid().ToString('N').Substring(0, 8))
            New-Item -ItemType Directory -Force $dir | Out-Null
            $source = Join-Path $dir 'PresentMon-2.6.0-x64.exe'
            Set-Content -LiteralPath $source -Value 'fake presentmon bytes' -NoNewline
            [pscustomobject]@{
                Source = $source
                Out    = Join-Path $dir 'payload\presentmon\PresentMon-2.6.0-x64.exe'
                Pins   = [pscustomobject]@{ Sha256 = (Get-OmaSha256 $source).ToUpperInvariant(); SignerSubject = $intel }
            }
        }
    }

    It 'PresentMon is staged when the hash and the Intel signature match' {
        $c = New-PmCase
        $reused = Save-OmaPresentMon -Source $c.Source -Destination $c.Out -Pins $c.Pins -SignatureProvider { param($Path) New-PmSig }
        $reused | Should -BeFalse
        Get-Content -Raw -LiteralPath $c.Out | Should -BeExactly 'fake presentmon bytes'
        Test-Path -LiteralPath "$($c.Out).partial" | Should -BeFalse
    }

    It 'a PresentMon with a different hash is rejected' {
        $c = New-PmCase
        $other = [pscustomobject]@{ Sha256 = '0' * 64; SignerSubject = $intel }
        { Save-OmaPresentMon -Source $c.Source -Destination $c.Out -Pins $other -SignatureProvider { param($Path) New-PmSig } } |
            Should -Throw '*PresentMon-2.6.0-x64.exe rejected: SHA-256 *, expected 0000*'
        Test-Path -LiteralPath $c.Out | Should -BeFalse
        Test-Path -LiteralPath "$($c.Out).partial" | Should -BeFalse
    }

    It 'an unsigned PresentMon is rejected' {
        $c = New-PmCase
        { Save-OmaPresentMon -Source $c.Source -Destination $c.Out -Pins $c.Pins -SignatureProvider { param($Path) New-PmSig -Status NotSigned } } |
            Should -Throw '*PresentMon-2.6.0-x64.exe rejected: Authenticode status NotSigned*'
        Test-Path -LiteralPath $c.Out | Should -BeFalse
        # Valid, but not signed by Intel.
        { Save-OmaPresentMon -Source $c.Source -Destination $c.Out -Pins $c.Pins -SignatureProvider { param($Path) New-PmSig -Subject 'CN=Intel Corporation Fake' } } |
            Should -Throw "*signer 'CN=Intel Corporation Fake'*"
        Test-Path -LiteralPath $c.Out | Should -BeFalse
        Test-Path -LiteralPath "$($c.Out).partial" | Should -BeFalse
    }

    It 'a cached PresentMon is reused' {
        $c = New-PmCase
        New-Item -ItemType Directory -Force (Split-Path $c.Out) | Out-Null
        Copy-Item -LiteralPath $c.Source -Destination $c.Out
        # The source is never read when the cached copy passes.
        $missing = Join-Path $TestDrive 'no-such-presentmon.exe'
        $reused = Save-OmaPresentMon -Source $missing -Destination $c.Out -Pins $c.Pins -SignatureProvider { param($Path) New-PmSig }
        $reused | Should -BeTrue
        Get-Content -Raw -LiteralPath $c.Out | Should -BeExactly 'fake presentmon bytes'

        # A cached copy that fails the pins is removed and fetched again.
        Set-Content -LiteralPath $c.Out -Value 'stale presentmon'
        $reused = Save-OmaPresentMon -Source $c.Source -Destination $c.Out -Pins $c.Pins -SignatureProvider { param($Path) New-PmSig }
        $reused | Should -BeFalse
        Get-Content -Raw -LiteralPath $c.Out | Should -BeExactly 'fake presentmon bytes'
    }

    It 'Test-OmaPresentMonExe returns the first problem or null' {
        $c = New-PmCase
        Test-OmaPresentMonExe -Path $c.Source -Pins $c.Pins -SignatureProvider { param($Path) New-PmSig } | Should -BeNullOrEmpty
        Test-OmaPresentMonExe -Path $c.Source -Pins $c.Pins -SignatureProvider { param($Path) New-PmSig -Status HashMismatch } |
            Should -BeLike 'Authenticode status HashMismatch*'
    }
}
