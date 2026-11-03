# Dev-only (not packaged): vendors Lucide icons into apps/tinwave-desktop/icons/ (design-system.md section 3.4).
#
#   powershell -File tools/vendor-icons.ps1                      re-fetch exactly the vendored set at the pinned version
#   powershell -File tools/vendor-icons.ps1 -Add a-icon,b-icon   add icons to the set
#   powershell -File tools/vendor-icons.ps1 -Version 1.70.0      bump the pinned version (re-fetches the whole set)
#
# Only icons that QML references are vendored (tests/icons.rs enforces both directions). The pinned version lives in
# icons/VERSION. Each SVG is fetched from the tagged Lucide source and gets one change: stroke-width 2 -> 1.75, the
# design-system stroke for 20 px icons (Lucide's own default is 2; the value is baked in because an SVG in the
# Qt resource file cannot take it from a token at run time). The licence text is refreshed from the same tag.
param(
    [string[]]$Add = @(),
    [string]$Version = ''
)
$ErrorActionPreference = 'Stop'
$iconDir = Join-Path (Split-Path -Parent $PSScriptRoot) 'icons'
$versionFile = Join-Path $iconDir 'VERSION'
if (-not $Version) { $Version = (Get-Content $versionFile -Raw).Trim() }
$stroke = '1.75'

# `powershell -File` passes "a,b" as one string; accept both spellings.
$Add = @($Add | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$names = @(@(Get-ChildItem $iconDir -Filter *.svg | ForEach-Object { $_.BaseName }) + $Add | Sort-Object -Unique)
$utf8 = New-Object System.Text.UTF8Encoding($false)
$base = "https://raw.githubusercontent.com/lucide-icons/lucide/$Version"

foreach ($name in $names) {
    if ($name -notmatch '^[a-z0-9]+(-[a-z0-9]+)*$') { throw "bad icon name '$name'" }
    $svg = (Invoke-WebRequest -UseBasicParsing "$base/icons/$name.svg").Content
    if ($svg -is [byte[]]) { $svg = $utf8.GetString($svg) }
    $svg = $svg -replace "`r`n", "`n"
    if ($svg -notmatch 'stroke-width="2"') { throw "$name.svg has no stroke-width=""2"" to replace" }
    $svg = $svg -replace 'stroke-width="2"', "stroke-width=`"$stroke`""
    [IO.File]::WriteAllText((Join-Path $iconDir "$name.svg"), $svg, $utf8)
    "vendored $name"
}

$licence = (Invoke-WebRequest -UseBasicParsing "$base/LICENSE").Content
if ($licence -is [byte[]]) { $licence = $utf8.GetString($licence) }
[IO.File]::WriteAllText((Join-Path $iconDir 'LICENSE-lucide.txt'), ($licence -replace "`r`n", "`n"), $utf8)
[IO.File]::WriteAllText($versionFile, "$Version`n", $utf8)
"Lucide $Version, $($names.Count) icons"
