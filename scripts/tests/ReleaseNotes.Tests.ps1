#Requires -Version 7
# Pester 5 tests for scripts/lib/OmaReleaseNotes.psm1 and scripts/render-release-notes.ps1
# (spec M6a §5.3). Everything works on files under $TestDrive.
#   Import-Module Pester -RequiredVersion 5.7.1
#   Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI

BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\lib\OmaReleaseNotes.psm1') -Force -ErrorAction Stop
    $script:template = (Resolve-Path (Join-Path $PSScriptRoot '..\..\.github\release-notes-template.md')).Path
    $script:renderScript = (Resolve-Path (Join-Path $PSScriptRoot '..\render-release-notes.ps1')).Path
    $script:utf8 = [Text.UTF8Encoding]::new($false)
    $script:attribution = 'Free code signing provided by SignPath.io, certificate by SignPath Foundation'
    $script:unsignedLine = 'The installer is not code-signed yet, so Windows SmartScreen may warn you: choose *More info* → *Run anyway*.'
    $script:changesStart = '<!-- oma:changes:start -->'
    $script:changesEnd = '<!-- oma:changes:end -->'
    $script:genStart = '<!-- oma:generated:start -->'
    $script:genEnd = '<!-- oma:generated:end -->'

    # Extracts the text between two markers (exclusive) from a body.
    function script:Get-Between([string]$Text, [string]$From, [string]$To) {
        $a = $Text.IndexOf($From) + $From.Length
        $b = $Text.IndexOf($To)
        $Text.Substring($a, $b - $a)
    }

    # A custom template: lets the marker-validation tests control the layout exactly.
    function script:New-Template([string]$Text) {
        $p = Join-Path $TestDrive ([guid]::NewGuid().ToString('N') + '.md')
        [IO.File]::WriteAllBytes($p, $utf8.GetBytes($Text))
        $p
    }
}

Describe 'New-OmaReleaseNotes' {
    It 'renders_signed_variant' {
        $t = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $true
        $t | Should -Match ([regex]::Escape($attribution))
        $t | Should -Not -Match ([regex]::Escape($unsignedLine))
    }

    It 'renders_unsigned_variant' {
        $t = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $false
        $t | Should -Match ([regex]::Escape($unsignedLine))
        $t | Should -Not -Match ([regex]::Escape($attribution))
    }

    It 'replaces_every_placeholder' {
        foreach ($signed in $true, $false) {
            $t = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $signed
            $t | Should -Not -Match '\{\{'
            $t | Should -Not -Match '\}\}'
            $t | Should -Match ([regex]::Escape('OpenMonitor.Advanced_1.2.3_x64-setup.exe'))
        }
    }

    It 'contains_the_install_and_verify_sections' {
        $t = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $true
        $t | Should -Match '(?m)^## Install$'
        $t | Should -Match '(?m)^## Verify your download$'
        $t | Should -Match '/S\b'
        $t | Should -Match '/NOSENSORS'
        $t | Should -Match 'Get-FileHash \.\\OpenMonitor\.Advanced_1\.2\.3_x64-setup\.exe -Algorithm SHA256'
        $t | Should -Match 'gh attestation verify OpenMonitor\.Advanced_1\.2\.3_x64-setup\.exe --repo Cioscos/OpenMonitorAdvanced'
        $t | Should -Match 'CODE_SIGNING\.md'
    }

    It 'keeps_each_marker_once_in_order' {
        $t = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $true
        foreach ($m in $changesStart, $changesEnd, $genStart, $genEnd) {
            ([regex]::Matches($t, [regex]::Escape($m))).Count | Should -Be 1
        }
        $t.IndexOf($changesStart) | Should -BeLessThan $t.IndexOf($changesEnd)
        $t.IndexOf($changesEnd) | Should -BeLessThan $t.IndexOf($genStart)
        $t.IndexOf($genStart) | Should -BeLessThan $t.IndexOf($genEnd)
        $t | Should -Match '<!-- Write the changes here -->'
    }

    It 'uses_LF_only' {
        (New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $true) | Should -Not -Match "`r"
    }

    It 'rejects_a_bad_version' {
        { New-OmaReleaseNotes -TemplatePath $template -Version '1.2' -Signed $true } | Should -Throw
        { New-OmaReleaseNotes -TemplatePath $template -Version 'v1.2.3' -Signed $true } | Should -Throw
    }

    It 'rejects_a_template_with_an_unknown_placeholder' {
        $p = New-Template "$changesStart`n$changesEnd`n$genStart`n{{WHAT}}`n$genEnd`n"
        { New-OmaReleaseNotes -TemplatePath $p -Version '1.2.3' -Signed $true } | Should -Throw '*{{WHAT}}*'
    }
}

Describe 'Update-OmaReleaseNotes' {
    It 'update_switches_the_generated_block_and_keeps_changes' {
        $manual = "`n## What is new`r`n`n- caf$([char]0xE9) fix  `n`n  * trailing spaces   `n"
        $unsigned = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $false
        $existing = "intro`n$changesStart$manual$changesEnd`n" + $unsigned.Substring($unsigned.IndexOf($genStart))
        $existing | Should -Match ([regex]::Escape($unsignedLine))

        $out = Update-OmaReleaseNotes -ExistingBody $existing -TemplatePath $template -Version '1.2.3' -Signed $true
        (Get-Between $out $changesStart $changesEnd) | Should -BeExactly $manual
        $utf8.GetBytes((Get-Between $out $changesStart $changesEnd)) | Should -Be $utf8.GetBytes($manual)
        $out | Should -Match ([regex]::Escape($attribution))
        $out | Should -Not -Match ([regex]::Escape($unsignedLine))
        $out.StartsWith("intro`n$changesStart") | Should -BeTrue
        ([regex]::Matches($out, [regex]::Escape($genStart))).Count | Should -Be 1
    }

    It 'update_regenerates_the_version_in_the_generated_block' {
        $old = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $false
        $out = Update-OmaReleaseNotes -ExistingBody $old -TemplatePath $template -Version '1.2.4' -Signed $false
        $out | Should -Match '1\.2\.4_x64-setup'
        $out | Should -Not -Match '1\.2\.3_x64-setup'
    }

    It 'update_is_idempotent' {
        $body = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $true
        (Update-OmaReleaseNotes -ExistingBody $body -TemplatePath $template -Version '1.2.3' -Signed $true) | Should -BeExactly $body
    }

    It 'update_rejects_missing_markers' {
        $good = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $true
        foreach ($m in $changesStart, $changesEnd, $genStart, $genEnd) {
            $bad = $good.Replace($m, '')
            { Update-OmaReleaseNotes -ExistingBody $bad -TemplatePath $template -Version '1.2.3' -Signed $true } |
                Should -Throw "*$m*"
        }
    }

    It 'update_rejects_duplicate_markers' {
        $good = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $true
        foreach ($m in $changesStart, $changesEnd, $genStart, $genEnd) {
            $bad = $good + "`n$m`n"
            { Update-OmaReleaseNotes -ExistingBody $bad -TemplatePath $template -Version '1.2.3' -Signed $true } |
                Should -Throw "*$m*"
        }
    }

    It 'update_rejects_wrong_order' {
        $good = New-OmaReleaseNotes -TemplatePath $template -Version '1.2.3' -Signed $true
        $swapped = "$genStart`ng`n$genEnd`n$changesStart`nx`n$changesEnd`n"
        { Update-OmaReleaseNotes -ExistingBody $swapped -TemplatePath $template -Version '1.2.3' -Signed $true } | Should -Throw '*order*'
        # An end before its own start is also out of order.
        $inverted = $good.Replace($genStart, '@@').Replace($genEnd, $genStart).Replace('@@', $genEnd)
        { Update-OmaReleaseNotes -ExistingBody $inverted -TemplatePath $template -Version '1.2.3' -Signed $true } | Should -Throw '*order*'
        # A template that is itself out of order is rejected when rendering.
        $p = New-Template "$genStart`ngen`n$genEnd`n$changesStart`nx`n$changesEnd`n"
        { New-OmaReleaseNotes -TemplatePath $p -Version '1.2.3' -Signed $true } | Should -Throw '*order*'
    }

    It 'update_throws_on_a_body_without_markers' {
        { Update-OmaReleaseNotes -ExistingBody 'no markers' -TemplatePath $template -Version '1.2.3' -Signed $true } | Should -Throw
    }
}

Describe 'render-release-notes.ps1' {
    It 'writes_utf8_without_bom_and_lf' {
        $out = Join-Path $TestDrive 'notes.md'
        & $renderScript -Version '1.2.3' -Signed:$true -Out $out
        $bytes = [IO.File]::ReadAllBytes($out)
        ($bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) | Should -BeFalse
        $bytes | Should -Not -Contain 13
        $utf8.GetString($bytes) | Should -Match ([regex]::Escape($attribution))
    }

    It 'honours_Signed_false' {
        $out = Join-Path $TestDrive 'unsigned.md'
        & $renderScript -Version '1.2.3' -Signed:$false -Out $out
        $utf8.GetString([IO.File]::ReadAllBytes($out)) | Should -Match ([regex]::Escape($unsignedLine))
    }

    It 'updates_an_existing_body_keeping_the_changes' {
        $first = Join-Path $TestDrive 'first.md'
        $second = Join-Path $TestDrive 'second.md'
        & $renderScript -Version '1.2.3' -Signed:$false -Out $first
        $text = $utf8.GetString([IO.File]::ReadAllBytes($first)).Replace('<!-- Write the changes here -->', 'Hand-written news')
        [IO.File]::WriteAllBytes($first, $utf8.GetBytes($text))
        & $renderScript -Version '1.2.3' -Signed:$true -Out $second -Existing $first
        $t = $utf8.GetString([IO.File]::ReadAllBytes($second))
        $t | Should -Match 'Hand-written news'
        $t | Should -Match ([regex]::Escape($attribution))
    }

    It 'fails_without_writing_when_markers_are_bad' {
        $existing = Join-Path $TestDrive 'bad.md'
        [IO.File]::WriteAllBytes($existing, $utf8.GetBytes('nothing'))
        $out = Join-Path $TestDrive 'never.md'
        { & $renderScript -Version '1.2.3' -Signed:$true -Out $out -Existing $existing } | Should -Throw
        Test-Path $out | Should -BeFalse
    }
}
