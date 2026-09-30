#Requires -Version 7
# Draft release notes (spec M6a §5.3). The template carries four HTML comment markers: the
# hand-written changes sit between oma:changes:start/end, the technical block (signing state,
# install, verification) between oma:generated:start/end. A rerun on an existing draft replaces
# only the generated block, so the hand-written changes survive byte for byte.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$script:Utf8 = [Text.UTF8Encoding]::new($false)
$script:Attribution = 'Free code signing provided by SignPath.io, certificate by SignPath Foundation'
$script:UnsignedLine = 'The installer is not code-signed yet, so Windows SmartScreen may warn you: choose *More info* → *Run anyway*.'
# Marker names in the only valid order: changes, then generated.
$script:Markers = @(
    '<!-- oma:changes:start -->'
    '<!-- oma:changes:end -->'
    '<!-- oma:generated:start -->'
    '<!-- oma:generated:end -->'
)

# Finds every marker exactly once and in order; returns their start offsets. Ordinal search on
# purpose: the text is data and must not be interpreted in any way.
function Get-OmaMarkerOffsets {
    param([string]$Text, [string]$What)
    $offsets = foreach ($m in $script:Markers) {
        $first = $Text.IndexOf($m, [StringComparison]::Ordinal)
        if ($first -lt 0) { throw "$What`: marker $m not found." }
        if ($Text.IndexOf($m, $first + 1, [StringComparison]::Ordinal) -ge 0) {
            throw "$What`: marker $m appears more than once."
        }
        $first
    }
    for ($i = 1; $i -lt $offsets.Count; $i++) {
        if ($offsets[$i] -le $offsets[$i - 1]) {
            throw "$What`: markers are out of order (expected changes start, changes end, generated start, generated end)."
        }
    }
    , @($offsets)
}

function Read-OmaTemplate {
    param([string]$Path)
    $text = $script:Utf8.GetString([IO.File]::ReadAllBytes($Path))
    $text.TrimStart([char]0xFEFF).Replace("`r`n", "`n")
}

<#
.SYNOPSIS
  Renders the template for a version and signing state; returns a string with LF endings.
#>
function New-OmaReleaseNotes {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string]$TemplatePath,
        [Parameter(Mandatory)] [string]$Version,
        [Parameter(Mandatory)] [bool]$Signed
    )
    if ($Version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
        throw "Version '$Version' is not X.Y.Z."
    }
    $text = Read-OmaTemplate -Path $TemplatePath
    $null = Get-OmaMarkerOffsets -Text $text -What 'Template'
    $signing = if ($Signed) { $script:Attribution } else { $script:UnsignedLine }
    $rendered = [regex]::Replace($text, '\{\{([A-Za-z0-9_]+)\}\}', {
            param($m)
            switch ($m.Groups[1].Value) {
                'VERSION' { $Version }
                'SIGNING' { $signing }
                default { throw "Template has an unknown placeholder $($m.Value)." }
            }
        })
    if ($rendered.Contains('{{') -or $rendered.Contains('}}')) {
        throw 'Template has a malformed placeholder.'
    }
    $rendered
}

<#
.SYNOPSIS
  Replaces the generated block of an existing draft body with a freshly rendered one. Everything
  outside the generated markers, changes block included, is kept byte for byte. Throws before
  producing anything if the existing body has missing, duplicate or misordered markers.
#>
function Update-OmaReleaseNotes {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [AllowEmptyString()] [string]$ExistingBody,
        [Parameter(Mandatory)] [string]$TemplatePath,
        [Parameter(Mandatory)] [string]$Version,
        [Parameter(Mandatory)] [bool]$Signed
    )
    $old = Get-OmaMarkerOffsets -Text $ExistingBody -What 'Existing release body'
    $fresh = New-OmaReleaseNotes -TemplatePath $TemplatePath -Version $Version -Signed $Signed
    $new = Get-OmaMarkerOffsets -Text $fresh -What 'Rendered template'

    $genStart = $script:Markers[2]; $genEnd = $script:Markers[3]
    $oldFrom = $old[2]; $oldTo = $old[3] + $genEnd.Length
    $newFrom = $new[2]; $newTo = $new[3] + $genEnd.Length
    $ExistingBody.Substring(0, $oldFrom) + $fresh.Substring($newFrom, $newTo - $newFrom) + $ExistingBody.Substring($oldTo)
}

Export-ModuleMember -Function New-OmaReleaseNotes, Update-OmaReleaseNotes
