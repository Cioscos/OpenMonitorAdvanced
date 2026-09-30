#Requires -Version 7
# Pester 5 tests for the metadata gate of scripts/build-installer-payload.ps1 (step 4) and for the
# PawnIO pins shared with the verifier (scripts/lib/OmaPawnIoPins.psm1). The script runs against
# a fake `dotnet` under $TestDrive: nothing is published and nothing is downloaded.
#   Import-Module Pester -RequiredVersion 5.7.1
#   Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI

BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaCommon.psm1') -Force -ErrorAction Stop
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaPawnIoPins.psm1') -Force -ErrorAction Stop
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
