#Requires -Version 7
# Pester 5 tests for the signature and payload verification (spec M6a §5.1, plan L4/L5, Task 4).
# Every provider (signatures, chain, embedded signature, version info, 7-Zip, trust store) is a
# fake: no certificate store, SDK or archive is touched. The 'Integration' block at the end runs
# only on an ephemeral runner or a VM (plan L5) and is excluded everywhere else:
#   Import-Module Pester -RequiredVersion 5.7.1
#   Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI

BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaCommon.psm1') -Force -ErrorAction Stop
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaPawnIoPins.psm1') -Force -ErrorAction Stop
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaPresentMonPins.psm1') -Force -ErrorAction Stop
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaSigning.psm1') -Force -ErrorAction Stop
    $script:verifyScript = (Resolve-Path (Join-Path $PSScriptRoot '..\verify-signatures.ps1')).Path
    $script:pwshExe = (Get-Process -Id $PID).Path

    $script:version = '0.3.0'
    $script:releaseSubject = 'CN=SignPath Foundation, O=SignPath Foundation, L=Lewes, S=Delaware, C=US'
    $script:releaseThumb = 'A1' * 20
    $script:testSubject = 'CN=SignPath Test Certificate, O=SignPath GmbH, C=AT'
    $script:testThumb = 'B2' * 20
    $script:testRoot = 'C3' * 20
    $script:pawnSubject = 'CN=PawnIO Fake Signer, O=namazso'
    $script:pawnThumb = 'D4' * 20
    $script:presentMonSubject = 'CN=PresentMon Fake Signer, O=Intel'

    function New-Certificates {
        [pscustomobject]@{
            release = [pscustomobject]@{ subject = $releaseSubject; thumbprints = @($releaseThumb) }
            test    = [pscustomobject]@{
                subject             = $testSubject
                thumbprints         = @($testThumb)
                rootThumbprints     = @($testRoot)
                rootCertificatePath = 'C:\fake\test-root.cer'
            }
        }
    }

    function New-Sig([string]$Status = 'Valid', [string]$Subject = $releaseSubject, [string]$Thumb = $releaseThumb,
        [bool]$Timestamp = $true, [string]$Type = 'Authenticode') {
        [pscustomobject]@{
            Status                 = $Status
            StatusMessage          = "fake status $Status"
            SignatureType          = $Type
            SignerCertificate      = [pscustomobject]@{ Subject = $Subject; Thumbprint = $Thumb }
            TimeStamperCertificate = if ($Timestamp) { [pscustomobject]@{ Subject = 'CN=Fake TSA'; Thumbprint = 'E5' * 20 } } else { $null }
        }
    }

    function New-Embedded([bool]$Embedded = $true, [bool]$SignatureValid = $true, [bool]$TimestampValid = $true) {
        [pscustomobject]@{ Embedded = $Embedded; SignatureValid = $SignatureValid; TimestampValid = $TimestampValid; Detail = 'fake signtool' }
    }

    # Per-test state read by the fake providers (scriptblocks defined here resolve $script: in
    # this file, whatever module calls them).
    function Reset-Fakes {
        $script:sigOverride = @{}
        $script:embeddedOverride = @{}
        $script:versionOverride = @{}
        $script:signatureCalls = [Collections.Generic.List[string]]::new()
        $script:chainRoot = $testRoot
        $script:chainPaths = [Collections.Generic.List[string]]::new()
        $script:store = [Collections.Generic.List[string]]::new()
        $script:storeLog = [Collections.Generic.List[string]]::new()
        $script:elevated = $true
        $script:rootCert = [pscustomobject]@{ Thumbprint = $testRoot; Subject = 'CN=SignPath Test Root'; HasPrivateKey = $false }
    }

    $script:sigProvider = {
        param($Path)
        $n = Split-Path -Leaf $Path
        $script:signatureCalls.Add($n)
        if ($script:sigOverride.ContainsKey($n)) { return $script:sigOverride[$n] }
        if ($n -eq 'PawnIO_setup.exe') { return New-Sig -Subject $pawnSubject -Thumb $pawnThumb }
        if ($n -eq 'PresentMon-2.6.0-x64.exe') { return New-Sig -Subject $presentMonSubject -Thumb ('F6' * 20) }
        New-Sig
    }
    $script:embeddedProvider = {
        param($Path)
        $n = Split-Path -Leaf $Path
        if ($script:embeddedOverride.ContainsKey($n)) { return $script:embeddedOverride[$n] }
        New-Embedded
    }
    $script:chainProvider = {
        param($Certificate, $Path)
        $script:chainPaths.Add($Path)
        [pscustomobject]@{ Thumbprints = @($Certificate.Thumbprint, $script:chainRoot); RootThumbprint = $script:chainRoot }
    }
    $script:versionProvider = {
        param($Path)
        $n = Split-Path -Leaf $Path
        if ($script:versionOverride.ContainsKey($n)) { return $script:versionOverride[$n] }
        $fv = if ($n -eq 'oma-service.exe') { "$version.0" } else { $version }
        [pscustomobject]@{ ProductName = 'OpenMonitor Advanced'; ProductVersion = $version; FileVersion = $fv }
    }
    $script:trustStore = @{
        IsElevated = { $script:elevated }
        LoadRoot   = { param($Path) $script:storeLog.Add("load $Path"); $script:rootCert }
        Contains   = { param($Thumbprint) $script:store.Contains($Thumbprint) }
        Add        = { param($Certificate) $script:storeLog.Add("add $($Certificate.Thumbprint)"); $script:store.Add($Certificate.Thumbprint) }
        Remove     = { param($Thumbprint) $script:storeLog.Add("remove $Thumbprint"); [void]$script:store.Remove($Thumbprint) }
    }

    function Invoke-Sig([string]$Policy = 'release', $Certificates = (New-Certificates), [string]$Name = 'oma-app.exe') {
        $file = Join-Path $TestDrive $Name
        if (-not (Test-Path -LiteralPath $file)) { [IO.File]::WriteAllText($file, "fake $Name") }
        @(Test-OmaSignature -Path $file -Policy $Policy -Certificates $Certificates -SignatureProvider $sigProvider `
                -ChainProvider $chainProvider -EmbeddedSignatureProvider $embeddedProvider -TrustStore $trustStore)
    }

    # --- fake run for Test-OmaPayload ---------------------------------------------------------------

    function Write-Fake([string]$Path, [string]$Text) {
        New-Item -ItemType Directory -Force (Split-Path $Path) | Out-Null
        [IO.File]::WriteAllText($Path, $Text)
    }

    function Get-TextSha([string]$Text) {
        [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Text))).ToLowerInvariant()
    }

    # A signing state as sign-shim.ps1 leaves it after the apply pass, a fake setup and the files
    # the fake 7-Zip "extracts" (relative path -> content; $null drops the file).
    function New-FakeRun([ValidateSet('signed', 'unsigned')] [string]$Kind = 'signed') {
        $root = Join-Path $TestDrive ([guid]::NewGuid().ToString('N').Substring(0, 8))
        $state = Join-Path $root 'state'
        $content = @{
            'app-unsigned'         = 'app patched by tauri'
            'uninstaller-unsigned' = 'uninstaller bytes'
            'service-unsigned'     = 'service unsigned'
            'app-signed'           = 'app signed by signpath'
            'uninstaller-signed'   = 'uninstaller signed by signpath'
            'service-signed'       = 'service signed by signpath'
            'overlay-unsigned'     = 'overlay unsigned'
            'overlay-signed'       = 'overlay signed by signpath'
        }
        foreach ($n in 'oma-app.exe', 'uninstall.exe', 'oma-service.exe', 'oma-overlay.exe') {
            $key = @{ 'oma-app.exe' = 'app'; 'uninstall.exe' = 'uninstaller'; 'oma-service.exe' = 'service'; 'oma-overlay.exe' = 'overlay' }[$n]
            Write-Fake (Join-Path $state "unsigned\$n") $content["$key-unsigned"]
            if ($Kind -eq 'signed') { Write-Fake (Join-Path $state "signed\$n") $content["$key-signed"] }
        }
        $sha = @{}
        foreach ($k in $content.Keys) { $sha[$k] = Get-TextSha $content[$k] }
        $setupName = "OpenMonitor Advanced_${version}_x64-setup.exe"
        $collect = @(
            [ordered]@{ role = 'app'; path = 'C:\r\target\release\oma-app.exe'; name = 'oma-app.exe'; sha256 = $sha['app-unsigned']; after = $sha['app-unsigned'] }
            [ordered]@{ role = 'uninstaller'; path = 'C:\t\nst1A2B.tmp'; name = 'uninstall.exe'; sha256 = $sha['uninstaller-unsigned']; after = $sha['uninstaller-unsigned'] }
            [ordered]@{ role = 'setup'; path = "C:\r\target\release\bundle\nsis\$setupName"; name = $setupName; sha256 = 'f' * 64; after = 'f' * 64 }
            [ordered]@{ role = 'service'; path = 'C:\r\target\installer-payload\service\oma-service.exe'; name = 'oma-service.exe'; sha256 = $sha['service-unsigned']; after = $sha['service-unsigned'] }
            [ordered]@{ role = 'overlay'; path = 'C:\r\target\installer-payload\overlay\oma-overlay.exe'; name = 'oma-overlay.exe'; sha256 = $sha['overlay-unsigned']; after = $sha['overlay-unsigned'] }
        )
        $apply = @()
        $signed = [ordered]@{}
        if ($Kind -eq 'signed') {
            $apply = @(
                [ordered]@{ role = 'app'; path = 'C:\r\target\release\oma-app.exe'; name = 'oma-app.exe'; sha256 = $sha['app-unsigned']; after = $sha['app-signed'] }
                [ordered]@{ role = 'uninstaller'; path = 'C:\t\nst3C4D.tmp'; name = 'uninstall.exe'; sha256 = $sha['uninstaller-unsigned']; after = $sha['uninstaller-signed'] }
                [ordered]@{ role = 'setup'; path = "C:\r\target\release\bundle\nsis\$setupName"; name = $setupName; sha256 = 'e' * 64; after = 'e' * 64 }
            )
            $signed = [ordered]@{ 'oma-app.exe' = $sha['app-signed']; 'uninstall.exe' = $sha['uninstaller-signed']; 'oma-service.exe' = $sha['service-signed']; 'oma-overlay.exe' = $sha['overlay-signed'] }
        }
        $manifest = [ordered]@{
            schema = 1; commit = 'a' * 40; version = $version; runId = '42'; runAttempt = '1'; repoRoot = 'C:\r'
            expected = [ordered]@{ app = 'C:\r\target\release\oma-app.exe'; setupDir = 'C:\r\target\release\bundle\nsis'; setupName = $setupName; service = 'C:\r\target\installer-payload\service\oma-service.exe'; overlay = 'C:\r\target\installer-payload\overlay\oma-overlay.exe' }
            collect = $collect; apply = $apply; signed = $signed
        }
        Write-Fake (Join-Path $state 'manifest.json') (ConvertTo-Json -InputObject $manifest -Depth 10)
        $setup = Join-Path $root $setupName
        Write-Fake $setup 'setup bytes'
        $which = if ($Kind -eq 'signed') { 'signed' } else { 'unsigned' }
        $script:extract = [ordered]@{
            'oma-app.exe'                      = $content["app-$which"]
            'oma-overlay.exe'                  = $content["overlay-$which"]
            'service\oma-service.exe'          = $content["service-$which"]
            'service\PawnIO_setup.exe'         = 'pawnio setup bytes'
            'service\presentmon\PresentMon-2.6.0-x64.exe' = 'presentmon bytes'
            'THIRD_PARTY_NOTICES.txt'          = 'notices'
            '$PLUGINSDIR\System.dll'           = 'nsis plugin'
        }
        $script:extractorExit = 0
        $script:extractorCalls = 0
        $script:extraListing = @()
        $script:listerExit = 0
        $script:lastDestination = $null
        $script:lockExtracted = $false
        $script:lockHandle = $null
        [pscustomobject]@{ Root = $root; State = $state; Manifest = Join-Path $state 'manifest.json'; Setup = $setup; Sha = $sha }
    }

    $script:extractor = {
        param($Setup, $Destination)
        $script:extractorCalls++
        $script:lastDestination = $Destination
        foreach ($rel in $script:extract.Keys) {
            if ($null -ne $script:extract[$rel]) { Write-Fake (Join-Path $Destination $rel) $script:extract[$rel] }
        }
        if ($script:lockExtracted) {
            # Held open until the test closes it, so the extraction folder cannot be removed.
            # A text entry, with retries: antivirus scanners briefly open freshly written files.
            for ($i = 1; -not $script:lockHandle; $i++) {
                try { $script:lockHandle = [IO.File]::Open((Join-Path $Destination 'THIRD_PARTY_NOTICES.txt'), 'Open', 'Read', 'None') }
                catch [IO.IOException] { if ($i -ge 20) { throw }; Start-Sleep -Milliseconds 100 }
            }
        }
        [pscustomobject]@{ ExitCode = $script:extractorExit; Output = 'fake 7z' }
    }

    # The archive listing (7z l -slt): one path per entry, so two entries at the same path show
    # up twice here even though extracting them leaves one file.
    $script:lister = {
        param($Setup)
        $paths = @($script:extract.Keys | Where-Object { $null -ne $script:extract[$_] }) + @($script:extraListing)
        [pscustomobject]@{ ExitCode = $script:listerExit; Paths = $paths; Output = 'fake 7z l' }
    }

    function Get-VerifyLeftovers { @(Get-ChildItem -LiteralPath $fakeTemp -Filter 'oma-verify-*' -Force -ErrorAction SilentlyContinue) }

    function Get-FakePins {
        [pscustomobject]@{
            Sha256           = (Get-TextSha 'pawnio setup bytes').ToUpperInvariant()
            SignerSubject    = $pawnSubject
            SignerThumbprint = $pawnThumb
        }
    }

    function Get-FakePresentMonPins {
        [pscustomobject]@{ Sha256 = (Get-TextSha 'presentmon bytes').ToUpperInvariant(); SignerSubject = $presentMonSubject }
    }

    function Invoke-Payload($Run, [string]$Policy = 'release', $Pins = (Get-FakePins), [string]$Version = $version,
        $PresentMonPins = (Get-FakePresentMonPins)) {
        @(Test-OmaPayload -Setup $Run.Setup -Policy $Policy -Manifest $Run.Manifest -Version $Version `
                -Certificates (New-Certificates) -Extractor $extractor -Lister $lister -SignatureProvider $sigProvider `
                -ChainProvider $chainProvider -VersionInfoProvider $versionProvider `
                -EmbeddedSignatureProvider $embeddedProvider -TrustStore $trustStore -PawnIoPins $Pins -PresentMonPins $PresentMonPins)
    }

    $script:savedIsolation = @{
        RUNNER_ENVIRONMENT = $env:RUNNER_ENVIRONMENT
        OMA_ISOLATED_TRUST = $env:OMA_ISOLATED_TRUST
        TMP                = $env:TMP
        TEMP               = $env:TEMP
    }
    # Test-OmaPayload extracts under GetTempPath(): keep it inside $TestDrive, where leftovers show.
    $script:fakeTemp = Join-Path $TestDrive 'temp'
    New-Item -ItemType Directory -Force $fakeTemp | Out-Null
}

Describe 'signature and payload verification with fake providers' {
    AfterAll {
        foreach ($k in $savedIsolation.Keys) { [Environment]::SetEnvironmentVariable($k, $savedIsolation[$k]) }
    }

    BeforeEach {
        Reset-Fakes
        # Declared isolated by default; the guard tests clear it.
        $env:RUNNER_ENVIRONMENT = $null
        $env:OMA_ISOLATED_TRUST = '1'
        $env:TMP = $fakeTemp
        $env:TEMP = $fakeTemp
    }

    Describe 'Test-OmaSignature, release policy' {
        It 'release_accepts_valid_expected_signer' {
            Invoke-Sig | Should -BeNullOrEmpty
        }

        It 'release_rejects_substring_subject' {
            $sigOverride['oma-app.exe'] = New-Sig -Subject 'CN=SignPath Foundation Fake'
            $p = Invoke-Sig
            $p | Should -Not -BeNullOrEmpty
            ($p -join "`n") | Should -BeLike '*CN=SignPath Foundation Fake*'
        }

        It 'release_rejects_unknown_thumbprint' {
            $sigOverride['oma-app.exe'] = New-Sig -Thumb ('FF' * 20)
            ($p = Invoke-Sig) | Should -Not -BeNullOrEmpty
            ($p -join "`n") | Should -BeLike "*$('FF' * 20)*"
        }

        It 'release_fails_without_configured_certificate' {
            $c = New-Certificates
            $c.release.subject = ''
            (Invoke-Sig -Certificates $c) | Should -Contain 'no approved certificate configured for policy release'
            $c = New-Certificates
            $c.release.thumbprints = @()
            (Invoke-Sig -Certificates $c) | Should -Contain 'no approved certificate configured for policy release'
            $signatureCalls.Count | Should -Be 0
        }

        It 'release requires the configured subject to carry CN=SignPath Foundation' {
            $c = New-Certificates
            $c.release.subject = 'CN=Someone Else, O=SignPath Foundation'
            $sigOverride['oma-app.exe'] = New-Sig -Subject $c.release.subject
            ((Invoke-Sig -Certificates $c) -join "`n") | Should -BeLike '*CN=SignPath Foundation*'
        }

        It 'rejects_hash_mismatch_status' {
            $sigOverride['oma-app.exe'] = New-Sig -Status 'HashMismatch'
            ((Invoke-Sig) -join "`n") | Should -BeLike '*HashMismatch*'
        }

        It 'rejects_missing_timestamp' {
            $sigOverride['oma-app.exe'] = New-Sig -Timestamp $false
            ((Invoke-Sig) -join "`n") | Should -BeLike '*timestamp*'
        }

        It 'rejects_invalid_timestamp' {
            $embeddedOverride['oma-app.exe'] = New-Embedded -TimestampValid $false
            ((Invoke-Sig) -join "`n") | Should -BeLike '*timestamp*'
        }

        It 'rejects_catalog_signature' {
            $sigOverride['oma-app.exe'] = New-Sig -Type 'Catalog'
            $embeddedOverride['oma-app.exe'] = New-Embedded -Embedded $false -SignatureValid $false -TimestampValid $false
            ((Invoke-Sig) -join "`n") | Should -BeLike '*embedded*'
        }

        It 'rejects a signature without a signature type' {
            $sig = New-Sig
            $sig.PSObject.Properties.Remove('SignatureType')
            $sigOverride['oma-app.exe'] = $sig
            ((Invoke-Sig) -join "`n") | Should -BeLike '*signature type*Authenticode*'
        }

        It 'rejects a valid status when the embedded signature does not verify' {
            $embeddedOverride['oma-app.exe'] = New-Embedded -SignatureValid $false
            ((Invoke-Sig) -join "`n") | Should -BeLike '*embedded signature*'
        }

        It 'turns a provider exception into a problem' {
            $throwing = { param($Path) throw 'boom from provider' }
            $file = Join-Path $TestDrive 'oma-app.exe'
            [IO.File]::WriteAllText($file, 'x')
            $p = @(Test-OmaSignature -Path $file -Policy release -Certificates (New-Certificates) -SignatureProvider $throwing `
                    -ChainProvider $chainProvider -EmbeddedSignatureProvider $embeddedProvider -TrustStore $trustStore)
            ($p -join "`n") | Should -BeLike '*boom from provider*'
        }

        It 'release never imports a root' {
            Invoke-Sig | Should -BeNullOrEmpty
            $storeLog.Count | Should -Be 0
        }
    }

    Describe 'Test-OmaSignature, test policy' {
        BeforeEach {
            $sigOverride['oma-app.exe'] = New-Sig -Subject $testSubject -Thumb $testThumb
        }

        It 'test_accepts_only_valid_signature_of_configured_leaf_and_root' {
            Invoke-Sig -Policy test | Should -BeNullOrEmpty
            $storeLog | Should -Be @("load C:\fake\test-root.cer", "add $testRoot", "remove $testRoot")
        }

        It 'test_rejects_unknown_error_even_with_valid_certificate_chain' {
            $sigOverride['oma-app.exe'] = New-Sig -Status 'UnknownError' -Subject $testSubject -Thumb $testThumb
            ((Invoke-Sig -Policy test) -join "`n") | Should -BeLike '*UnknownError*'
        }

        It 'test rejects <_> after the import' -ForEach @('HashMismatch', 'NotSigned', 'NotTrusted', 'Incompatible') {
            $sigOverride['oma-app.exe'] = New-Sig -Status $_ -Subject $testSubject -Thumb $testThumb
            ((Invoke-Sig -Policy test) -join "`n") | Should -BeLike "*$_*"
        }

        It 'test passes the signed file to the chain provider, for the certificates embedded in it' {
            Invoke-Sig -Policy test | Should -BeNullOrEmpty
            $chainPaths | Should -Be @((Join-Path $TestDrive 'oma-app.exe'))
        }

        It 'test_rejects_untrusted_root_of_other_root' {
            $script:chainRoot = 'AB' * 20
            ((Invoke-Sig -Policy test) -join "`n") | Should -BeLike "*$('AB' * 20)*"
        }

        It 'test_rejects_valid_but_wrong_leaf' {
            $sigOverride['oma-app.exe'] = New-Sig -Subject $testSubject -Thumb ('CD' * 20)
            ((Invoke-Sig -Policy test) -join "`n") | Should -BeLike "*$('CD' * 20)*"
            $sigOverride['oma-app.exe'] = New-Sig -Subject 'CN=Other Test Leaf' -Thumb $testThumb
            ((Invoke-Sig -Policy test) -join "`n") | Should -BeLike '*CN=Other Test Leaf*'
        }

        It 'test rejects the release certificate' {
            $sigOverride['oma-app.exe'] = New-Sig
            Invoke-Sig -Policy test | Should -Not -BeNullOrEmpty
        }

        It 'test_trust_is_cleaned_up_on_failure' {
            $sigOverride['oma-app.exe'] = New-Sig -Status 'HashMismatch' -Subject $testSubject -Thumb $testThumb
            Invoke-Sig -Policy test | Should -Not -BeNullOrEmpty
            $store.Count | Should -Be 0
            $storeLog | Should -Contain "remove $testRoot"

            Reset-Fakes
            $throwing = { param($Path) throw 'provider died' }
            $file = Join-Path $TestDrive 'oma-app.exe'
            [IO.File]::WriteAllText($file, 'x')
            $p = @(Test-OmaSignature -Path $file -Policy test -Certificates (New-Certificates) -SignatureProvider $throwing `
                    -ChainProvider $chainProvider -EmbeddedSignatureProvider $embeddedProvider -TrustStore $trustStore)
            ($p -join "`n") | Should -BeLike '*provider died*'
            $store.Count | Should -Be 0
            $storeLog | Should -Contain "remove $testRoot"
        }

        It 'test_keeps_preexisting_root' {
            $store.Add($testRoot)
            Invoke-Sig -Policy test | Should -BeNullOrEmpty
            $store | Should -Contain $testRoot
            $storeLog | Should -Not -Contain "add $testRoot"
            $storeLog | Should -Not -Contain "remove $testRoot"
        }

        It 'test_refuses_nonisolated_environment' {
            $env:OMA_ISOLATED_TRUST = $null
            $env:RUNNER_ENVIRONMENT = 'self-hosted'
            ((Invoke-Sig -Policy test) -join "`n") | Should -BeLike '*isolated*'
            $storeLog.Count | Should -Be 0
            $signatureCalls.Count | Should -Be 0
        }

        It 'test refuses a non-elevated process even when declared isolated' {
            $env:RUNNER_ENVIRONMENT = 'github-hosted'
            $script:elevated = $false
            ((Invoke-Sig -Policy test) -join "`n") | Should -BeLike '*administrator*'
            $storeLog.Count | Should -Be 0
        }

        It 'test accepts a GitHub-hosted runner as isolated' {
            $env:OMA_ISOLATED_TRUST = $null
            $env:RUNNER_ENVIRONMENT = 'github-hosted'
            Invoke-Sig -Policy test | Should -BeNullOrEmpty
        }

        It 'test refuses a root file whose thumbprint is not pinned, before the import' {
            $script:rootCert = [pscustomobject]@{ Thumbprint = 'EE' * 20; Subject = 'CN=Other Root'; HasPrivateKey = $false }
            ((Invoke-Sig -Policy test) -join "`n") | Should -BeLike "*$('EE' * 20)*"
            $storeLog | Should -Not -Contain "add $('EE' * 20)"
        }

        It 'test fails when a required test field is empty' {
            foreach ($field in 'subject', 'thumbprints', 'rootThumbprints', 'rootCertificatePath') {
                $c = New-Certificates
                $c.test.$field = if ($field -like '*s') { @() } else { '' }
                (Invoke-Sig -Policy test -Certificates $c) | Should -Contain 'no approved certificate configured for policy test'
            }
            $storeLog.Count | Should -Be 0
        }
    }

    Describe 'Test-OmaVersionInfo' {
        It 'metadata_product_name_checked' {
            $vi = [pscustomobject]@{ ProductName = 'oma-service'; ProductVersion = '0.3.0'; FileVersion = '0.3.0.0' }
            ((Test-OmaVersionInfo -VersionInfo $vi -Version '0.3.0' -Name 'oma-service.exe') -join "`n") | Should -BeLike "*ProductName 'oma-service'*"
        }

        It 'accepts the formats recorded by the spike' {
            $pe = [pscustomobject]@{ ProductName = 'OpenMonitor Advanced'; ProductVersion = '0.3.0'; FileVersion = '0.3.0' }
            $svc = [pscustomobject]@{ ProductName = 'OpenMonitor Advanced'; ProductVersion = '0.3.0'; FileVersion = '0.3.0.0' }
            Test-OmaVersionInfo -VersionInfo $pe -Version '0.3.0' -Name 'oma-app.exe' | Should -BeNullOrEmpty
            Test-OmaVersionInfo -VersionInfo $svc -Version '0.3.0' -Name 'oma-service.exe' | Should -BeNullOrEmpty
        }

        It 'rejects a ProductVersion with a source revision suffix' {
            $svc = [pscustomobject]@{ ProductName = 'OpenMonitor Advanced'; ProductVersion = '0.3.0+abc'; FileVersion = '0.3.0.0' }
            Test-OmaVersionInfo -VersionInfo $svc -Version '0.3.0' -Name 'oma-service.exe' | Should -Not -BeNullOrEmpty
        }
    }

    Describe 'Test-OmaPayload' {
        It 'accepts a consistent signed setup and notes the installed uninstaller' {
            $run = New-FakeRun
            $p = Test-OmaPayload -Setup $run.Setup -Policy release -Manifest $run.Manifest -Version $version `
                -Certificates (New-Certificates) -Extractor $extractor -Lister $lister -SignatureProvider $sigProvider `
                -ChainProvider $chainProvider -VersionInfoProvider $versionProvider `
                -EmbeddedSignatureProvider $embeddedProvider -TrustStore $trustStore -PawnIoPins (Get-FakePins) `
                -PresentMonPins (Get-FakePresentMonPins) -InformationVariable notes
            @($p) | Should -BeNullOrEmpty
            ($notes -join "`n") | Should -BeLike '*installed uninstaller signature not verified*'
            # Setup, both extracted binaries, the signed uninstaller copy, PawnIO and PresentMon.
            $signatureCalls | Should -Contain 'uninstall.exe'
            $signatureCalls | Should -Contain 'PawnIO_setup.exe'
            $signatureCalls | Should -Contain 'PresentMon-2.6.0-x64.exe'
            $signatureCalls | Should -Contain (Split-Path -Leaf $run.Setup)
        }

        It 'verify lists exactly one oma-overlay.exe with the version metadata' {
            # Installed next to the app (plan DP7) and checked like the other product files.
            $run = New-FakeRun
            @(Invoke-Payload $run) | Should -BeNullOrEmpty
            $signatureCalls | Should -Contain 'oma-overlay.exe'

            $run = New-FakeRun
            $extract['oma-overlay.exe'] = $null
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*exactly one oma-overlay.exe in the archive listing, found 0*'

            $run = New-FakeRun
            $script:extraListing = @('oma-overlay.exe')
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*exactly one oma-overlay.exe in the archive listing, found 2*'

            $run = New-FakeRun
            $extract['oma-overlay.exe'] = 'overlay unsigned'
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*oma-overlay.exe in the setup has SHA-256 *from the signed copy*'

            $run = New-FakeRun
            $versionOverride['oma-overlay.exe'] = [pscustomobject]@{ ProductName = 'oma-overlay'; ProductVersion = '0.3.0'; FileVersion = '0.3.0.0' }
            $text = (Invoke-Payload $run) -join "`n"
            $text | Should -BeLike "*oma-overlay.exe has ProductName 'oma-overlay', expected 'OpenMonitor Advanced'*"
            $text | Should -BeLike "*oma-overlay.exe has FileVersion '0.3.0.0', expected '0.3.0'*"

            # Unsigned builds compare it with the collect pass.
            $versionOverride.Clear()
            $run = New-FakeRun -Kind unsigned
            @(Invoke-Payload $run -Policy none) | Should -BeNullOrEmpty
            $m = Get-Content -Raw $run.Manifest | ConvertFrom-Json -AsHashtable
            $m['collect'] = @($m['collect'] | Where-Object { $_['role'] -ne 'overlay' })
            Write-Fake $run.Manifest (ConvertTo-Json -InputObject $m -Depth 10)
            ((Invoke-Payload $run -Policy none) -join "`n") | Should -BeLike '*exactly one overlay, found 0*'
        }

        It 'payload_hash_must_match_manifest' {
            $run = New-FakeRun
            $extract['oma-app.exe'] = 'app patched by tauri'   # the unsigned copy inside a "signed" setup
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*oma-app.exe*SHA-256*'
        }

        It 'payload_missing_or_duplicate_fails' {
            $run = New-FakeRun
            $extract['service\oma-service.exe'] = $null
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*oma-service.exe*found 0*'

            $run = New-FakeRun
            $extract['other\oma-app.exe'] = 'app signed by signpath'
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*oma-app.exe*found 2*'

            $run = New-FakeRun
            $extract['service\PawnIO_setup.exe'] = $null
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*PawnIO_setup.exe*found 0*'

            $run = New-FakeRun
            $extract['service\presentmon\PresentMon-2.6.0-x64.exe'] = $null
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*exactly one PresentMon-2.6.0-x64.exe in the archive listing, found 0*'

            $run = New-FakeRun
            $extract['service\PresentMon-2.6.0-x64.exe'] = 'presentmon bytes'
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*PresentMon-2.6.0-x64.exe*found 2*'
        }

        It 'extractor_failure_fails' {
            $run = New-FakeRun
            $script:extractorExit = 2
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*extract*exit code 2*'
            $run = New-FakeRun
            $throwing = { param($Setup, $Destination) throw '7z not found' }
            $p = @(Test-OmaPayload -Setup $run.Setup -Policy none -Manifest $run.Manifest -Version $version -Extractor $throwing -Lister $lister `
                    -SignatureProvider $sigProvider -VersionInfoProvider $versionProvider -PawnIoPins (Get-FakePins))
            ($p -join "`n") | Should -BeLike '*7z not found*'
        }

        It 'same-path duplicates in the archive listing fail' {
            # 7z x -y would overwrite the first entry with the second: only the listing sees both.
            $run = New-FakeRun
            $script:extraListing = @('oma-app.exe')
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*exactly one oma-app.exe in the archive listing, found 2*'
            $run = New-FakeRun
            $script:extraListing = @('service\PawnIO_setup.exe')
            ((Invoke-Payload $run -Policy none) -join "`n") | Should -BeLike '*exactly one PawnIO_setup.exe in the archive listing, found 2*'
        }

        It 'a failed archive listing fails before extraction' {
            $run = New-FakeRun
            $script:listerExit = 2
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*listing the setup with 7-Zip failed: exit code 2*'
            $extractorCalls | Should -Be 0
        }

        It 'removes the extraction folder after success, failure and exception' {
            $run = New-FakeRun
            Invoke-Payload $run | Should -BeNullOrEmpty
            $lastDestination | Should -Not -BeNullOrEmpty
            Test-Path -LiteralPath $lastDestination | Should -BeFalse

            $run = New-FakeRun
            $script:extractorExit = 2
            Invoke-Payload $run | Should -Not -BeNullOrEmpty
            Test-Path -LiteralPath $lastDestination | Should -BeFalse

            $run = New-FakeRun
            $throwing = {
                param($Setup, $Destination)
                $script:lastDestination = $Destination
                Write-Fake (Join-Path $Destination 'partial.bin') 'half extracted'
                throw '7z crashed'
            }
            $p = @(Test-OmaPayload -Setup $run.Setup -Policy none -Manifest $run.Manifest -Version $version -Extractor $throwing -Lister $lister `
                    -SignatureProvider $sigProvider -VersionInfoProvider $versionProvider -PawnIoPins (Get-FakePins))
            ($p -join "`n") | Should -BeLike '*7z crashed*'
            Test-Path -LiteralPath $lastDestination | Should -BeFalse
            Get-VerifyLeftovers | Should -BeNullOrEmpty
        }

        It 'reports an extraction folder it cannot remove' {
            $run = New-FakeRun
            $script:lockExtracted = $true
            try {
                ((Invoke-Payload $run) -join "`n") | Should -BeLike "*cannot remove the temporary extraction folder $lastDestination*"
            } finally {
                if ($script:lockHandle) { $script:lockHandle.Dispose(); $script:lockHandle = $null }
                if ($lastDestination -and (Test-Path -LiteralPath $lastDestination)) { Remove-Item -LiteralPath $lastDestination -Recurse -Force }
            }
        }

        It 'pawnio_hash_and_signature_checked' {
            $run = New-FakeRun
            $extract['service\PawnIO_setup.exe'] = 'tampered pawnio'
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*PawnIO_setup.exe*SHA-256*'

            $run = New-FakeRun
            $sigOverride['PawnIO_setup.exe'] = New-Sig -Subject $pawnSubject -Thumb ('99' * 20)
            ((Invoke-Payload $run) -join "`n") | Should -BeLike "*PawnIO_setup.exe*$('99' * 20)*"

            # Valid alone is not enough: the pinned author is required.
            $run = New-FakeRun
            $sigOverride['PawnIO_setup.exe'] = New-Sig
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*PawnIO_setup.exe*signer*'

            $run = New-FakeRun
            $sigOverride['PawnIO_setup.exe'] = New-Sig -Status 'HashMismatch' -Subject $pawnSubject -Thumb $pawnThumb
            ((Invoke-Payload $run -Policy none) -join "`n") | Should -BeLike '*PawnIO_setup.exe*HashMismatch*'
        }

        It 'presentmon_hash_and_intel_signature_checked' {
            $run = New-FakeRun
            Invoke-Payload $run | Should -BeNullOrEmpty

            $run = New-FakeRun
            $extract['service\presentmon\PresentMon-2.6.0-x64.exe'] = 'tampered presentmon'
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*PresentMon-2.6.0-x64.exe: SHA-256*'

            # Valid alone is not enough: the pinned signer is required.
            $run = New-FakeRun
            $sigOverride['PresentMon-2.6.0-x64.exe'] = New-Sig
            ((Invoke-Payload $run) -join "`n") | Should -BeLike "*PresentMon-2.6.0-x64.exe: signer '$releaseSubject'*"

            # Checked under every policy, the unsigned build included.
            $run = New-FakeRun -Kind unsigned
            $sigOverride['PresentMon-2.6.0-x64.exe'] = New-Sig -Status 'NotSigned' -Subject $presentMonSubject
            ((Invoke-Payload $run -Policy none) -join "`n") | Should -BeLike '*PresentMon-2.6.0-x64.exe: Authenticode status NotSigned*'
        }

        It 'uses the shared PresentMon pins by default' {
            $run = New-FakeRun
            $pins = Get-OmaPresentMonPins
            $p = @(Test-OmaPayload -Setup $run.Setup -Policy none -Manifest $run.Manifest -Version $version -Extractor $extractor -Lister $lister `
                    -SignatureProvider $sigProvider -VersionInfoProvider $versionProvider -PawnIoPins (Get-FakePins))
            ($p -join "`n") | Should -BeLike "*PresentMon-2.6.0-x64.exe: SHA-256 *, expected $($pins.Sha256)*"
        }

        It 'uses the shared PawnIO pins by default' {
            $run = New-FakeRun
            $pins = Get-OmaPawnIoPins
            $p = @(Test-OmaPayload -Setup $run.Setup -Policy none -Manifest $run.Manifest -Version $version -Extractor $extractor -Lister $lister `
                    -SignatureProvider $sigProvider -VersionInfoProvider $versionProvider)
            ($p -join "`n") | Should -BeLike "*PawnIO_setup.exe*expected $($pins.Sha256)*"
        }

        It 'none_policy_checks_content_and_warns' {
            $run = New-FakeRun -Kind unsigned
            Invoke-Payload $run -Policy none | Should -BeNullOrEmpty
            # Only the signatures of PawnIO and PresentMon are read; the product files are unsigned by design.
            $signatureCalls | Should -Be @('PawnIO_setup.exe', 'PresentMon-2.6.0-x64.exe')

            # Hashes are compared with the collect pass.
            $run = New-FakeRun -Kind unsigned
            $extract['oma-app.exe'] = 'something else'
            ((Invoke-Payload $run -Policy none) -join "`n") | Should -BeLike '*oma-app.exe*SHA-256*'

            # Empty setup.
            $run = New-FakeRun -Kind unsigned
            [IO.File]::WriteAllBytes($run.Setup, [byte[]]@())
            ((Invoke-Payload $run -Policy none) -join "`n") | Should -BeLike '*empty*'
            $extractorCalls | Should -Be 0
        }

        It 'none checks the collect pass of the uninstaller without looking for signed copies' {
            $run = New-FakeRun -Kind unsigned
            $m = Get-Content -Raw $run.Manifest | ConvertFrom-Json -AsHashtable
            $m['collect'] = @($m['collect'] | Where-Object { $_['role'] -ne 'uninstaller' })
            Write-Fake $run.Manifest (ConvertTo-Json -InputObject $m -Depth 10)
            ((Invoke-Payload $run -Policy none) -join "`n") | Should -BeLike '*uninstaller*collect*'
        }

        It 'release checks the imported uninstaller and its replacement' {
            $run = New-FakeRun
            Write-Fake (Join-Path $run.State 'signed\uninstall.exe') 'uninstaller changed after import'
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*uninstall.exe*imported*'

            $run = New-FakeRun
            $m = Get-Content -Raw $run.Manifest | ConvertFrom-Json -AsHashtable
            ($m['apply'] | Where-Object { $_['role'] -eq 'uninstaller' })['after'] = '0' * 64
            Write-Fake $run.Manifest (ConvertTo-Json -InputObject $m -Depth 10)
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*uninstall*replaced*'

            $run = New-FakeRun
            $sigOverride['uninstall.exe'] = New-Sig -Status 'NotSigned'
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*uninstall.exe*NotSigned*'
        }

        It 'metadata_setup_and_product_file_versions_checked' {
            $run = New-FakeRun
            $setupName = Split-Path -Leaf $run.Setup
            $versionOverride[$setupName] = [pscustomobject]@{ ProductName = 'OpenMonitor Advanced'; ProductVersion = '0.2.0'; FileVersion = '0.3.0' }
            $versionOverride['oma-service.exe'] = [pscustomobject]@{ ProductName = 'OpenMonitor Advanced'; ProductVersion = '0.3.0'; FileVersion = '0.3.0' }
            $versionOverride['oma-app.exe'] = [pscustomobject]@{ ProductName = 'OpenMonitor Advanced'; ProductVersion = '0.3.0'; FileVersion = '0.3.0.0' }
            $versionOverride['uninstall.exe'] = [pscustomobject]@{ ProductName = 'OpenMonitor Advanced'; ProductVersion = $null; FileVersion = '0.3.0' }
            $text = (Invoke-Payload $run) -join "`n"
            $text | Should -BeLike "*$setupName has ProductVersion '0.2.0', expected '0.3.0'*"
            $text | Should -BeLike "*oma-service.exe has FileVersion '0.3.0', expected '0.3.0.0'*"
            $text | Should -BeLike "*oma-app.exe has FileVersion '0.3.0.0', expected '0.3.0'*"
            $text | Should -BeLike "*uninstall.exe has ProductVersion '' (missing), expected '0.3.0'*"
        }

        It 'metadata product name of the payload is checked' {
            $run = New-FakeRun
            $versionOverride['oma-service.exe'] = [pscustomobject]@{ ProductName = 'oma-service'; ProductVersion = '0.3.0'; FileVersion = '0.3.0.0' }
            ((Invoke-Payload $run) -join "`n") | Should -BeLike "*oma-service.exe has ProductName 'oma-service'*"
        }

        It 'rejects a manifest of another version or a missing manifest' {
            $run = New-FakeRun
            ((Invoke-Payload $run -Version '0.4.0') -join "`n") | Should -BeLike '*manifest*0.3.0*'
            $run = New-FakeRun
            Remove-Item -LiteralPath $run.Manifest
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*manifest*'
        }

        It 'the signature policy of the setup applies to the product files' {
            $run = New-FakeRun
            $sigOverride['oma-service.exe'] = New-Sig -Status 'NotSigned'
            ((Invoke-Payload $run) -join "`n") | Should -BeLike '*oma-service.exe*NotSigned*'
        }
    }

    Describe 'Test-OmaSignedFiles' {
        BeforeEach {
            $script:dir = Join-Path $TestDrive ([guid]::NewGuid().ToString('N').Substring(0, 8))
            foreach ($n in 'oma-app.exe', 'uninstall.exe', 'oma-service.exe', 'oma-overlay.exe') { Write-Fake (Join-Path $dir $n) "signed $n" }
        }

        It 'accepts the four signed files' {
            Test-OmaSignedFiles -Directory $dir -Policy release -Version $version -Certificates (New-Certificates) `
                -SignatureProvider $sigProvider -ChainProvider $chainProvider -VersionInfoProvider $versionProvider `
                -EmbeddedSignatureProvider $embeddedProvider -TrustStore $trustStore | Should -BeNullOrEmpty
        }

        It 'requires exactly the four names' {
            Write-Fake (Join-Path $dir 'extra.dll') 'x'
            Remove-Item -LiteralPath (Join-Path $dir 'uninstall.exe')
            Remove-Item -LiteralPath (Join-Path $dir 'oma-overlay.exe')
            $p = @(Test-OmaSignedFiles -Directory $dir -Policy release -Version $version -Certificates (New-Certificates) `
                    -SignatureProvider $sigProvider -ChainProvider $chainProvider -VersionInfoProvider $versionProvider `
                    -EmbeddedSignatureProvider $embeddedProvider -TrustStore $trustStore)
            ($p -join "`n") | Should -BeLike '*missing: uninstall.exe, oma-overlay.exe*unexpected: extra.dll*'
        }

        It 'checks signatures and metadata of each file' {
            $sigOverride['uninstall.exe'] = New-Sig -Status 'HashMismatch'
            $versionOverride['oma-service.exe'] = [pscustomobject]@{ ProductName = 'oma-service'; ProductVersion = '0.3.0'; FileVersion = '0.3.0.0' }
            $sigOverride['oma-overlay.exe'] = New-Sig -Status 'NotSigned'
            $text = @(Test-OmaSignedFiles -Directory $dir -Policy release -Version $version -Certificates (New-Certificates) `
                    -SignatureProvider $sigProvider -ChainProvider $chainProvider -VersionInfoProvider $versionProvider `
                    -EmbeddedSignatureProvider $embeddedProvider -TrustStore $trustStore) -join "`n"
            $text | Should -BeLike '*uninstall.exe*HashMismatch*'
            $text | Should -BeLike "*oma-service.exe has ProductName 'oma-service'*"
            $text | Should -BeLike '*oma-overlay.exe*NotSigned*'
        }
    }

    Describe 'verify-signatures.ps1' {
        It 'exposes no provider or certificate override parameters' {
            $params = (Get-Command $verifyScript).Parameters.Keys
            $params | Should -Contain 'Policy'
            $params | Should -Contain 'Version'
            $params | Should -Contain 'Setup'
            $params | Should -Contain 'Manifest'
            $params | Should -Contain 'Files'
            @($params | Where-Object { $_ -match 'Provider|Extractor|Certificate|Trust|Pins' }) | Should -BeNullOrEmpty
        }

        It 'none writes the unsigned build warning and fails on bad content' {
            $setup = Join-Path $TestDrive 'empty-setup.exe'
            [IO.File]::WriteAllBytes($setup, [byte[]]@())
            $r = Invoke-OmaNative -FilePath $pwshExe -AllowFailure -ArgumentList @(
                '-NoProfile', '-NonInteractive', '-File', $verifyScript, '-Policy', 'none', '-Version', '0.3.0',
                '-Setup', $setup, '-Manifest', (Join-Path $TestDrive 'no-manifest.json'))
            $r.ExitCode | Should -Be 1
            $r.Stdout | Should -BeLike '*::warning::unsigned build*'
            $r.Stdout | Should -BeLike '*empty*'
        }

        It 'release refuses to run with the initial empty certificate configuration' {
            $cfg = Get-Content -Raw (Join-Path $PSScriptRoot '..\..\.signpath\certificates.json') | ConvertFrom-Json
            if ($cfg.release.subject) { Set-ItResult -Skipped -Because 'the release certificate is configured' }
            $dir = Join-Path $TestDrive 'files'
            foreach ($n in 'oma-app.exe', 'uninstall.exe', 'oma-service.exe', 'oma-overlay.exe') { Write-Fake (Join-Path $dir $n) $n }
            $r = Invoke-OmaNative -FilePath $pwshExe -AllowFailure -ArgumentList @(
                '-NoProfile', '-NonInteractive', '-File', $verifyScript, '-Policy', 'release', '-Version', '0.3.0', '-Files', $dir)
            $r.ExitCode | Should -Be 1
            $r.Stdout | Should -BeLike '*no approved certificate configured for policy release*'
        }

        It 'rejects an invalid version' {
            $r = Invoke-OmaNative -FilePath $pwshExe -AllowFailure -ArgumentList @(
                '-NoProfile', '-NonInteractive', '-File', $verifyScript, '-Policy', 'none', '-Version', '0.3',
                '-Setup', (Join-Path $TestDrive 'x.exe'), '-Manifest', (Join-Path $TestDrive 'm.json'))
            $r.ExitCode | Should -Be 1
            ($r.Stdout + $r.Stderr) | Should -BeLike '*invalid version*'
        }

        It 'the initial certificates file has the documented shape' {
            $cfg = Get-Content -Raw (Join-Path $PSScriptRoot '..\..\.signpath\certificates.json') | ConvertFrom-Json
            @($cfg.release.PSObject.Properties.Name) | Should -Be @('subject', 'thumbprints')
            @($cfg.test.PSObject.Properties.Name) | Should -Be @('subject', 'thumbprints', 'rootThumbprints', 'rootCertificatePath')
        }
    }

    Describe 'real providers' {
        It 'finds signtool in the Windows SDK or reports it missing' {
            $st = Get-OmaSignToolPath
            if ($st) { $st | Should -BeLike '*signtool.exe' ; Test-Path -LiteralPath $st | Should -BeTrue }
        }

        It 'the default chain provider uses the certificates embedded in the signature' {
            $sig = Get-AuthenticodeSignature -LiteralPath $pwshExe
            if ($sig.SignatureType -ne 'Authenticode') { Set-ItResult -Skipped -Because 'this pwsh.exe carries no embedded signature'; return }
            $embedded = [Security.Cryptography.X509Certificates.X509Certificate2Collection]::new()
            $embedded.Import($pwshExe)
            $embedded.Count | Should -BeGreaterThan 1
            $chain = Get-OmaCertificateChain -Certificate $sig.SignerCertificate -Path $pwshExe
            foreach ($c in $embedded) { $chain.Thumbprints | Should -Contain $c.Thumbprint }
            $chain.RootThumbprint | Should -Be $chain.Thumbprints[-1]
        }

        It 'parses the 7-Zip listing, keeping same-path duplicates and dropping folders' {
            $text = "Path = oma-app.exe`nSize = 1`nAttributes = `n`nPath = service`nFolder = +`n`n" +
                "Path = oma-app.exe`nAttributes = A`n`nPath = `$PLUGINSDIR`nAttributes = D`n`nPath = service\oma-service.exe`nAttributes = `n"
            ConvertFrom-Oma7ZipListing $text | Should -Be @('oma-app.exe', 'oma-app.exe', 'service\oma-service.exe')
        }

        It 'the default lister reads a real setup (when one was built here)' {
            $setup = @(Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot '..\..\target\release\bundle\nsis') -Filter '*-setup.exe' -ErrorAction SilentlyContinue)
            if ($setup.Count -eq 0) { Set-ItResult -Skipped -Because 'no setup in target/release/bundle/nsis'; return }
            $lister = InModuleScope OmaSigning { $script:DefaultLister }
            $r = & $lister $setup[0].FullName
            $r.ExitCode | Should -Be 0
            $r.Paths | Should -Contain 'oma-app.exe'
            $r.Paths | Should -Contain 'service\oma-service.exe'
            $r.Paths | Should -Contain 'service\PawnIO_setup.exe'
            @($r.Paths | Where-Object { $_ -like '*uninstall.exe' }) | Should -BeNullOrEmpty
        }

        It 'the default embedded provider rejects a catalog-only Windows file' {
            if (-not (Get-OmaSignToolPath)) { Set-ItResult -Skipped -Because 'no Windows SDK signtool' }
            $where = Join-Path $env:SystemRoot 'System32\where.exe'
            $r = Get-OmaEmbeddedSignature -Path $where
            $r.Embedded | Should -BeFalse
            $r.SignatureValid | Should -BeFalse
        }

        It 'the default embedded provider verifies an embedded, timestamped signature' {
            if (-not (Get-OmaSignToolPath)) { Set-ItResult -Skipped -Because 'no Windows SDK signtool' }
            $r = Get-OmaEmbeddedSignature -Path $pwshExe
            if ((Get-AuthenticodeSignature -LiteralPath $pwshExe).SignatureType -ne 'Authenticode') {
                Set-ItResult -Skipped -Because 'this pwsh.exe carries no embedded signature'
            }
            $r.Embedded | Should -BeTrue
            $r.SignatureValid | Should -BeTrue
            $r.TimestampValid | Should -BeTrue
        }
    }

}

# Ephemeral runner or VM only (plan L5): creates a throwaway CA and signing key, trusts the CA in
# Cert:\LocalMachine\Root through the same helper as the test policy, and checks the real
# Authenticode helpers on a PE built here. No external TSA: the full policy is expected to fail on
# the missing timestamp only; the timestamp gate is covered by the fakes and by the SignPath run.
Describe 'real Authenticode verification' -Tag Integration {
    BeforeAll {
        $script:skipReason = $null
        $isolated = $env:RUNNER_ENVIRONMENT -eq 'github-hosted' -or $env:OMA_ISOLATED_TRUST -eq '1'
        $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
            [Security.Principal.WindowsBuiltInRole]::Administrator)
        $script:signtool = Get-OmaSignToolPath
        if (-not ($isolated -and $admin)) { $script:skipReason = 'needs an isolated, elevated environment (plan L5)' }
        elseif (-not $signtool) { $script:skipReason = 'needs signtool from the Windows SDK' }
        if ($skipReason) { return }

        $script:work = Join-Path $TestDrive 'authenticode'
        New-Item -ItemType Directory -Force $work | Out-Null
        # Our own minimal PE (never Windows code, which may be catalog-signed).
        $script:fixture = Join-Path $work 'oma-fixture.dll'
        $source = 'public static class OmaFixture { public static int Answer() { int s = 0; for (int i = 0; i < 1000; i++) { s += i * 7; } return s; } }'
        Invoke-OmaNative -FilePath $pwshExe -ArgumentList @('-NoProfile', '-NonInteractive', '-Command',
            "Add-Type -TypeDefinition '$source' -OutputAssembly '$fixture' -OutputType Library") | Out-Null

        $X509 = 'System.Security.Cryptography.X509Certificates'
        $script:rootKey = [Security.Cryptography.RSA]::Create(3072)
        $script:leafKey = [Security.Cryptography.RSA]::Create(3072)
        $notBefore = [DateTimeOffset]::UtcNow.AddHours(-1)
        $notAfter = [DateTimeOffset]::UtcNow.AddDays(1)
        $sha256 = [Security.Cryptography.HashAlgorithmName]::SHA256
        $pkcs1 = [Security.Cryptography.RSASignaturePadding]::Pkcs1
        $rootReq = [Security.Cryptography.X509Certificates.CertificateRequest]::new('CN=OMA Integration Test Root', $rootKey, $sha256, $pkcs1)
        $rootReq.CertificateExtensions.Add((New-Object "$X509.X509BasicConstraintsExtension" $true, $false, 0, $true))
        $rootReq.CertificateExtensions.Add((New-Object "$X509.X509KeyUsageExtension" ([Security.Cryptography.X509Certificates.X509KeyUsageFlags]'KeyCertSign, CrlSign'), $true))
        $script:root = $rootReq.CreateSelfSigned($notBefore, $notAfter)
        $leafReq = [Security.Cryptography.X509Certificates.CertificateRequest]::new('CN=OMA Integration Test Signer', $leafKey, $sha256, $pkcs1)
        $leafReq.CertificateExtensions.Add((New-Object "$X509.X509BasicConstraintsExtension" $false, $false, 0, $true))
        $leafReq.CertificateExtensions.Add((New-Object "$X509.X509KeyUsageExtension" ([Security.Cryptography.X509Certificates.X509KeyUsageFlags]'DigitalSignature'), $true))
        $eku = [Security.Cryptography.OidCollection]::new()
        [void]$eku.Add([Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.3'))
        $leafReq.CertificateExtensions.Add((New-Object "$X509.X509EnhancedKeyUsageExtension" $eku, $true))
        $serial = [byte[]](1..16 | ForEach-Object { Get-Random -Maximum 256 })
        $leafPublic = $leafReq.Create($root, $notBefore.AddMinutes(1), $notAfter.AddMinutes(-1), $serial)
        $script:leaf = [Security.Cryptography.X509Certificates.RSACertificateExtensions]::CopyWithPrivateKey($leafPublic, $leafKey)

        $script:rootCer = Join-Path $work 'root.cer'
        [IO.File]::WriteAllBytes($rootCer, $root.Export([Security.Cryptography.X509Certificates.X509ContentType]::Cert))
        $script:pfx = Join-Path $work 'leaf.pfx'
        $script:pfxPassword = [guid]::NewGuid().ToString('N')
        $chain = [Security.Cryptography.X509Certificates.X509Certificate2Collection]::new()
        [void]$chain.Add($leaf)
        # Root without its private key: with two keyed certificates in the PFX signtool refuses to
        # choose ("Multiple certificates were found that meet all the given criteria").
        [void]$chain.Add([Security.Cryptography.X509Certificates.X509Certificate2]::new($root.RawData))
        [IO.File]::WriteAllBytes($pfx, $chain.Export([Security.Cryptography.X509Certificates.X509ContentType]::Pfx, $pfxPassword))
        Invoke-OmaNative -FilePath $signtool -ArgumentList @('sign', '/fd', 'SHA256', '/f', $pfx, '/p', $pfxPassword, $fixture) | Out-Null
        Remove-Item -LiteralPath $pfx -Force

        $script:certs = [pscustomobject]@{
            release = [pscustomobject]@{ subject = ''; thumbprints = @() }
            test    = [pscustomobject]@{ subject = $leaf.Subject; thumbprints = @($leaf.Thumbprint); rootThumbprints = @($root.Thumbprint); rootCertificatePath = $rootCer }
        }
        # The root must not be trusted before the test (otherwise nothing proves the cleanup).
        $script:rootWasTrusted = @(Get-ChildItem Cert:\LocalMachine\Root | Where-Object Thumbprint -EQ $root.Thumbprint).Count -gt 0
    }

    AfterAll {
        if ($skipReason) { return }
        foreach ($k in 'rootKey', 'leafKey') { $v = Get-Variable -Scope Script -Name $k -ValueOnly -ErrorAction SilentlyContinue; if ($v) { $v.Dispose() } }
        if ($work -and (Test-Path -LiteralPath $work)) { Remove-Item -LiteralPath $work -Recurse -Force }
        if (-not $rootWasTrusted) {
            # Belt and braces: the helper removes what it added; a leftover here is a test failure
            # reported below, but it must still not survive the run.
            Get-ChildItem Cert:\LocalMachine\Root | Where-Object Thumbprint -EQ $root.Thumbprint | Remove-Item
        }
    }

    It 'verifies our own signed PE and fails only on the missing timestamp, then cleans up' {
        if ($skipReason) { Set-ItResult -Skipped -Because $skipReason; return }
        $p = @(Test-OmaSignature -Path $fixture -Policy test -Certificates $certs)
        $p | Should -Not -BeNullOrEmpty
        # Only the two timestamp messages may remain (no TSA here); anything else, including a
        # signtool failure whose detail mentions the /tw warning, is a real failure.
        $name = Split-Path -Leaf $fixture
        $timestampOnly = @("${name}: no timestamp (no timestamping certificate)", "${name}: the timestamp is missing or does not verify:*")
        @($p | Where-Object { $_ -ne $timestampOnly[0] -and $_ -notlike $timestampOnly[1] }) | Should -BeNullOrEmpty
        @($p | Where-Object { $_ -like '*embedded signature does not verify*' }) | Should -BeNullOrEmpty
        if (-not $rootWasTrusted) {
            @(Get-ChildItem Cert:\LocalMachine\Root | Where-Object Thumbprint -EQ $root.Thumbprint) | Should -BeNullOrEmpty
        }
    }

    It 'rejects the PE after one byte of its code section changes' {
        if ($skipReason) { Set-ItResult -Skipped -Because $skipReason; return }
        $bytes = [IO.File]::ReadAllBytes($fixture)
        $pe = [BitConverter]::ToInt32($bytes, 0x3C)
        $sections = [BitConverter]::ToUInt16($bytes, $pe + 6)
        $optional = [BitConverter]::ToUInt16($bytes, $pe + 20)
        $table = $pe + 24 + $optional
        $offset = -1
        for ($i = 0; $i -lt $sections; $i++) {
            $s = $table + 40 * $i
            if ([Text.Encoding]::ASCII.GetString($bytes, $s, 8).TrimEnd([char]0) -eq '.text') {
                $raw = [BitConverter]::ToUInt32($bytes, $s + 20)
                $size = [BitConverter]::ToUInt32($bytes, $s + 16)
                $offset = [int]($raw + [math]::Floor($size / 2))
            }
        }
        $offset | Should -BeGreaterThan 0
        $bytes[$offset] = $bytes[$offset] -bxor 0xFF
        $tampered = Join-Path $work 'oma-fixture-tampered.dll'
        [IO.File]::WriteAllBytes($tampered, $bytes)
        # Inside the trust window both the PowerShell status and the Windows verifier must fail.
        $p = @(Test-OmaSignature -Path $tampered -Policy test -Certificates $certs)
        ($p -join "`n") | Should -BeLike '*HashMismatch*'
        ($p -join "`n") | Should -BeLike '*embedded signature does not verify*'
        if (-not $rootWasTrusted) {
            @(Get-ChildItem Cert:\LocalMachine\Root | Where-Object Thumbprint -EQ $root.Thumbprint) | Should -BeNullOrEmpty
        }
    }
}
