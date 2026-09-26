# Shows what changed in Tauri's NSIS bundler files between the version our
# template derives from and a newer tauri-cli, and 3-way merges the template.
#   pwsh scripts/nsis-template-drift.ps1 -To 2.12.0            # diff + merge preview
#   pwsh scripts/nsis-template-drift.ps1 -To 2.12.0 -Apply     # write merged template + new reference copy
param(
    [Parameter(Mandatory)] [string] $To,
    [string] $Ref = "tauri-cli-v$To",   # override (e.g. 'dev') to preview unreleased changes
    [switch] $Apply
)
$ErrorActionPreference = 'Stop'
$nsisDir = Join-Path $PSScriptRoot '..\app\src-tauri\nsis'
$base = Get-ChildItem $nsisDir -Filter 'upstream-*.nsi' | Select-Object -First 1
$from = $base.BaseName -replace '^upstream-', ''
$raw = 'https://raw.githubusercontent.com/tauri-apps/tauri/{0}/crates/tauri-bundler/src/bundle/windows/nsis/{1}'
$tmp = Join-Path ([IO.Path]::GetTempPath()) "oma-nsis-drift-$From-$To"
New-Item -ItemType Directory -Force "$tmp\old", "$tmp\new" | Out-Null

# installer.nsi is ours; the others are written by the bundler at build time and
# our template calls their macros/strings, so their changes matter too.
$files = 'installer.nsi', 'utils.nsh', 'FileAssociation.nsh', 'languages/English.nsh', 'languages/Italian.nsh'
foreach ($f in $files) {
    $name = $f -replace '/', '_'
    Invoke-WebRequest ($raw -f "tauri-cli-v$from", $f) -OutFile "$tmp\old\$name"
    Invoke-WebRequest ($raw -f $Ref, $f) -OutFile "$tmp\new\$name"
    Write-Host "=== $f ($from -> $Ref)" -ForegroundColor Cyan
    git --no-pager diff --no-index --stat --patch "$tmp\old\$name" "$tmp\new\$name"
}

# 3-way merge: ours = our template, base = old upstream, theirs = new upstream.
$merged = "$tmp\installer.merged.nsi"
Copy-Item (Join-Path $nsisDir 'installer.nsi') $merged
git merge-file -L ours -L "upstream-$from" -L "upstream-$To" $merged "$tmp\old\installer.nsi" "$tmp\new\installer.nsi"
$conflicts = $LASTEXITCODE
Write-Host "=== merged template: $merged ($conflicts conflict(s))" -ForegroundColor Cyan
if ($Apply) {
    if ($conflicts -ne 0) { throw 'Resolve the conflicts in the merged file first.' }
    Copy-Item $merged (Join-Path $nsisDir 'installer.nsi')
    Copy-Item "$tmp\new\installer.nsi" (Join-Path $nsisDir "upstream-$To.nsi")
    Remove-Item $base.FullName
    Write-Host "Template rebased onto $To. Bump @tauri-apps/cli, rebuild and re-check the installer."
}
