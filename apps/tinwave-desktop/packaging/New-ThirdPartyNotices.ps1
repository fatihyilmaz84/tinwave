# Generates THIRD-PARTY-NOTICES.txt for a deployed Tinwave bundle (ticket T-0042, ADR 0008 "Licenses").
#
#   .\New-ThirdPartyNotices.ps1 -Bundle <deployed folder> [-Out <file>] [-Package tinwave-desktop]
#
# Sources, all read from what is installed (nothing is downloaded, nothing is installed):
#   * Rust crates   `cargo metadata` for the normal-dependency closure of -Package on x86_64-pc-windows-msvc
#                   (proc-macro and build-only crates are not in the binary and are left out), the license files
#                   inside each crate's registry source, license-texts\ when a crate ships no file.
#   * Qt            the kit's SPDX SBOMs (which Qt module/plugin is under which license; which third-party components
#                   belong to which module), restricted to the Qt DLLs and plugins actually present in the bundle, and
#                   Qt's own attribution pages from the kit's documentation (<Qt>\Docs\Qt-<ver>\*\*-attribution-*.html:
#                   the output of qtattributionsscanner, with each component's full license text).
#   * Qt licenses   <Qt>\Licenses\LICENSE (LGPL-3.0 and GPL-3.0 texts), copied into the bundle by package.ps1.
#   * Assets        every LICENSE* file under apps\tinwave-desktop\icons\ (Lucide ISC etc.) when that folder exists.
#
# It fails (non-zero exit, nothing written) rather than skipping a component it cannot document.
param(
    [Parameter(Mandatory)][string]$Bundle,
    [string]$Out = "",
    [string[]]$Package = @('tinwave-desktop'),
    [string]$QtDir = ""
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')
if (-not $Out) { $Out = Join-Path $PSScriptRoot 'THIRD-PARTY-NOTICES.txt' }

$kit = Resolve-QtKit $QtDir
$files = @(Get-BundleFiles $Bundle)
$sb = New-Object Text.StringBuilder
function Add-Line([string]$s = '') { [void]$sb.Append($s).Append("`n") }
function Add-Rule([string]$title) {
    Add-Line; Add-Line ('=' * 100); Add-Line $title; Add-Line ('=' * 100); Add-Line
}
function Add-Block([string]$text) { foreach ($l in ($text -split "`n")) { Add-Line ('    ' + $l) } }

# --- 1. Qt shipped modules (SBOM) ------------------------------------------------------------------------------------
$sbom = Read-QtSbom $kit.Root
$shipped = @{}            # SBOM package name -> package
foreach ($f in $files) {
    if ($f.Name -notlike '*.dll' -or $f.Name -like 'vc*.dll' -or $f.Name -like 'msvcp*.dll' -or $f.Name -like 'concrt*.dll') { continue }
    $pk = @(Find-QtSbomPackages $f.Path $sbom)
    if ($pk.Count -eq 0) { throw "No SBOM entry for $($f.Rel): cannot document it. Run check-bundle.ps1." }
    foreach ($p in $pk) { $shipped[$p.Name] = $p }
}
if ($shipped.Count -eq 0) { throw "No Qt DLLs found in $Bundle." }

# --- 2. Qt third-party components (SBOM) -> attribution pages --------------------------------------------------------
# Libraries Qt builds into its own DLLs (config_qtbase.summary: "Using system X ... no"). Token = SBOM package name,
# value = the shipped Qt package(s) that contain it, and the attribution page id of the library itself.
$bundledLibs = @{
    'BundledPcre2'         = @{ Hosts = @('Core'); Page = 'pcre2' }
    'BundledZLIB'          = @{ Hosts = @('Core', 'Gui'); Page = 'zlib' }
    'ZlibPrivate'          = @{ Hosts = @('Core', 'Gui'); Page = $null }
    'BundledLibpng'        = @{ Hosts = @('Gui'); Page = 'libpng' }
    'PngPrivate'           = @{ Hosts = @('Gui'); Page = $null }
    'BundledFreetype'      = @{ Hosts = @('Gui'); Page = 'freetype' }
    'FreetypePrivate'      = @{ Hosts = @('Gui'); Page = $null }
    'BundledHarfbuzz'      = @{ Hosts = @('Gui'); Page = 'harfbuzz-ng' }
    'HarfbuzzPrivate'      = @{ Hosts = @('Gui'); Page = $null }
    'BundledLibjpeg'       = @{ Hosts = @('QJpegPlugin'); Page = 'libjpeg' }
    'BundledLibjpeg12bits' = @{ Hosts = @('QJpegPlugin'); Page = $null }
    'BundledLibjpeg16bits' = @{ Hosts = @('QJpegPlugin'); Page = $null }
    'JpegPrivate'          = @{ Hosts = @('QJpegPlugin'); Page = $null }
}
# Every library above must really be bundled in this kit, otherwise the table is wrong for it.
$summary = Read-TextFile (Join-Path $kit.Root 'config_qtbase.summary')
foreach ($lib in 'zlib', 'PCRE2', 'FreeType', 'HarfBuzz', 'libjpeg', 'libpng') {
    if ($summary -notmatch ("Using system $lib\s*\.+\s*no")) {
        throw "config_qtbase.summary does not say 'Using system $lib ... no': the bundled-library table in New-ThirdPartyNotices.ps1 does not match this Qt kit."
    }
}

$docsDir = Join-Path $kit.QtRoot "Docs\Qt-$($kit.Version)"
if (-not (Test-Path $docsDir)) { throw "Qt documentation not installed ($docsDir); it holds the attribution pages with each component's license text. Add the Qt 'Documentation' component." }
$pages = @{}
foreach ($pg in (Get-ChildItem $docsDir -Recurse -Filter '*-attribution-*.html')) {
    if ($pg.Name -match '^(qt[a-z0-9]*|qt)-attribution-(.+)\.html$') {
        $key = ($Matches[2] -replace '[^A-Za-z0-9]', '').ToLowerInvariant()
        if (-not $pages.ContainsKey($key)) { $pages[$key] = @() }
        $pages[$key] += $pg.FullName
    }
}
function Get-PageKey([string]$id) { ($id -replace '[^A-Za-z0-9]', '').ToLowerInvariant() }

$components = @{}   # page file -> @{ Sbom = list of SBOM names }
$unmapped = New-Object Collections.Generic.List[string]
$supplemented = @{}   # SBOM name -> supplement text file
function Add-Component([string]$key, [string]$label) {
    if ($pages.ContainsKey($key)) {
        foreach ($pf in $pages[$key]) {
            if (-not $components.ContainsKey($pf)) { $components[$pf] = New-Object Collections.Generic.List[string] }
            $components[$pf].Add($label)
        }
    } else {
        # Qt publishes no page for a few components. Their license text then comes from packaging\qt-supplements\<id>.txt,
        # copied verbatim from the Qt source tag of this version (README.md there says where); otherwise we cannot document it.
        $sup = Join-Path $PSScriptRoot "qt-supplements\$key.txt"
        if ((Test-Path $sup) -and $sbom.ContainsKey($label)) { $supplemented[$label] = $sup }
        else { $unmapped.Add("$label (no attribution page '$key' in $docsDir, no packaging\qt-supplements\$key.txt)") }
    }
}
foreach ($p in $sbom.Values) {
    if ($p.Name -notmatch '^(?<host>[^_]+)_Attribution_(?<id>.+)$') { continue }
    $hostName = $Matches['host']; $id = $Matches['id']
    $hostShipped = $false
    if ($bundledLibs.ContainsKey($hostName)) { $hostShipped = @($bundledLibs[$hostName].Hosts | Where-Object { $shipped.ContainsKey($_) }).Count -gt 0 }
    else { $hostShipped = $shipped.ContainsKey($hostName) }
    if ($hostShipped) { Add-Component (Get-PageKey $id) $p.Name }
}
foreach ($tok in $bundledLibs.Keys) {
    $b = $bundledLibs[$tok]
    if ($b.Page -and @($b.Hosts | Where-Object { $shipped.ContainsKey($_) }).Count -gt 0) { Add-Component (Get-PageKey $b.Page) $tok }
}
function ConvertFrom-AttributionPage([string]$path) {
    $h = Read-TextFile $path
    $i = $h.IndexOf('<h1 class="title">'); $j = $h.IndexOf('<!-- @@@')
    if ($i -lt 0 -or $j -lt $i) { throw "Unexpected layout of $path" }
    $b = $h.Substring($i, $j - $i)
    $title = [Net.WebUtility]::HtmlDecode(([regex]::Match($b, '<h1 class="title">(.*?)</h1>')).Groups[1].Value)
    $b = $b -replace '(?s)<h1 class="title">.*?</h1>', ''
    $b = $b -replace '(?s)<!--.*?-->', ''
    $b = $b -replace '(?i)<br\s*/?>', "`n"
    $b = $b -replace '(?i)</(p|div|pre|h\d|li)>', "`n"
    $b = $b -replace '(?s)<[^>]+>', ''
    $b = [Net.WebUtility]::HtmlDecode($b)
    $b = ($b -split "`n" | ForEach-Object { $_.TrimEnd() }) -join "`n"
    $b = $b -replace "`n{3,}", "`n`n"
    [pscustomobject]@{ Title = $title; Body = $b.Trim("`n") }
}

# --- 3. Rust crates --------------------------------------------------------------------------------------------------
$meta = (& cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $script:RepoRoot 'Cargo.toml') | Out-String) | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed" }
$byId = @{}; foreach ($p in $meta.packages) { $byId[$p.id] = $p }
$nodes = @{}; foreach ($n in $meta.resolve.nodes) { $nodes[$n.id] = $n }
$roots = @($meta.packages | Where-Object { $Package -contains $_.name -and $null -eq $_.source })
if ($roots.Count -ne $Package.Count) { throw "Package(s) not found in the workspace: $($Package -join ', ')" }
$seen = @{}; $queue = New-Object Collections.Generic.Queue[string]
foreach ($r in $roots) { $seen[$r.id] = $true; $queue.Enqueue($r.id) }
while ($queue.Count -gt 0) {
    $id = $queue.Dequeue()
    foreach ($d in $nodes[$id].deps) {
        if (@($d.dep_kinds | Where-Object { $null -eq $_.kind }).Count -eq 0) { continue }   # dev/build-only edge
        if ($seen.ContainsKey($d.pkg)) { continue }
        $dp = $byId[$d.pkg]
        # A proc-macro runs in the compiler, it is not part of the binary; nor is anything only it depends on.
        if (@($dp.targets | Where-Object { $_.kind -contains 'proc-macro' }).Count -gt 0) { continue }
        $seen[$d.pkg] = $true; $queue.Enqueue($d.pkg)
    }
}
$crates = @($seen.Keys | ForEach-Object { $byId[$_] } | Where-Object { $null -ne $_.source } | Sort-Object name, version)
$ownLicense = ($roots | Select-Object -First 1).license

$licenseTextDir = Join-Path $PSScriptRoot 'license-texts'
$texts = @{}          # sha -> @{ Text; Names (list) }
function Add-LicenseText([string]$text, [string]$label) {
    $norm = $text.Trim([char[]]"`n`r")
    $sha = [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($norm))).Replace('-', '')
    if (-not $texts.ContainsKey($sha)) { $texts[$sha] = @{ Text = $norm; Names = (New-Object Collections.Generic.List[string]) } }
    $texts[$sha].Names.Add($label)
}
$crateRows = New-Object Collections.Generic.List[object]
$mplCrates = New-Object Collections.Generic.List[string]
foreach ($c in $crates) {
    $dir = Split-Path $c.manifest_path -Parent
    $label = "$($c.name) $($c.version)"
    $lf = @(Get-ChildItem $dir -File | Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|UNLICENSE|NOTICE)' } | Sort-Object Name)
    $expr = if ($c.license) { $c.license } else { throw "Crate $label declares no license expression." }
    $authors = ($c.authors -join ', ')
    if ($lf.Count -gt 0) {
        foreach ($f in $lf) { Add-LicenseText (Read-TextFile $f.FullName) "$label ($($f.Name))" }
    } else {
        # The published crate carries no license file: use the canonical text of every SPDX id in its expression.
        $ids = @($expr -split '[\s()]+' | Where-Object { $_ -and $_ -notin 'OR', 'AND', 'WITH' } | Select-Object -Unique)
        foreach ($sid in $ids) {
            $tf = Join-Path $licenseTextDir "$sid.txt"
            if (-not (Test-Path $tf)) { throw "Crate $label ships no license file and packaging\license-texts\$sid.txt does not exist. Add the canonical $sid text there." }
            $t = (Read-TextFile $tf).Replace('{{HOLDERS}}', $(if ($authors) { $authors } else { "the $($c.name) authors ($($c.repository))" }))
            Add-LicenseText $t "$label (no license file in the crate; canonical $sid text, copyright holders from Cargo.toml authors)"
        }
    }
    if ($expr -match 'MPL-2\.0') { $mplCrates.Add($label) }
    $crateRows.Add([pscustomobject]@{ Label = $label; License = $expr; Repo = $c.repository; Authors = $authors; Files = (@($lf | ForEach-Object { $_.Name }) -join ', ') })
}

# --- 4. Assets with their own license (icons etc.) -------------------------------------------------------------------
$assetLicenses = @()
$iconsDir = Join-Path $script:RepoRoot 'apps\tinwave-desktop\icons'
if (Test-Path $iconsDir) {
    $assetLicenses = @(Get-ChildItem $iconsDir -Recurse -File | Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|NOTICE)' } | Sort-Object FullName)
}

if ($unmapped.Count -gt 0) {
    throw ("Qt components in the shipped modules without an attribution page, cannot document them:`n  " + ($unmapped -join "`n  "))
}

# --- Output ----------------------------------------------------------------------------------------------------------
Add-Line 'Tinwave - third-party notices'
Add-Line ('Qt ' + $kit.Version + ' (dynamically linked), ' + $crates.Count + ' Rust crates, ' + ($components.Count + $supplemented.Count) + ' components inside Qt.')
Add-Line
Add-Line 'Tinwave is a local-first music library manager and player. It is built on the software listed below. This file is'
Add-Line 'generated by apps/tinwave-desktop/packaging/New-ThirdPartyNotices.ps1 from the files of the build that produced this'
Add-Line 'folder; do not edit it by hand.'

Add-Rule '1. Tinwave'
Add-Line "Tinwave's own source code is licensed under: $ownLicense."
Add-Line 'The Apache-2.0 text is in licenses\Apache-2.0.txt.'

Add-Rule '2. Qt (LGPL-3.0)'
Add-Line "This program uses the Qt $($kit.Version) toolkit (https://www.qt.io), (C) The Qt Company Ltd. and other contributors,"
Add-Line 'under the GNU Lesser General Public License version 3 (LGPL-3.0), supplemented by the GNU General Public License'
Add-Line 'version 3. Both texts are in licenses\Qt-LGPL-3.0-and-GPL-3.0.txt.'
Add-Line
Add-Line '  * Qt is used unmodified, as the binaries built and published by The Qt Company, and is linked DYNAMICALLY: the Qt'
Add-Line '    libraries are the separate Qt6*.dll files and the plugin folders next to tinwave-desktop.exe. You may replace them'
Add-Line '    with another build of the same Qt version (binary-compatible) and run Tinwave against it.'
Add-Line '  * Tinwave adds no terms that forbid modifying the program for your own use or reverse engineering to debug such'
Add-Line '    modifications.'
Add-Line '  * The corresponding Qt source code (the exact version shipped) is available, see SOURCE-OFFER-Qt.txt.'
Add-Line '  * No GPL-only Qt module (for example the test-utils modules) is part of this program.'
Add-Line
Add-Line "Qt modules and plugins in this folder (licenses per the SPDX SBOMs in the Qt $($kit.Version) kit):"
Add-Line
foreach ($grp in ($shipped.Values | Group-Object { $_.License -replace 'LicenseRef-Qt-Commercial', 'Qt-Commercial' } | Sort-Object Count -Descending)) {
    Add-Line "  License: $($grp.Name)"
    $names = @($grp.Group | ForEach-Object { $_.Name } | Sort-Object)
    $line = '   '
    foreach ($nm in $names) {
        if ($line.Length + $nm.Length + 2 -gt 110) { Add-Line $line.TrimEnd(); $line = '   ' }
        $line += ' ' + $nm + ','
    }
    Add-Line ($line.TrimEnd(',')); Add-Line
}

Add-Rule '3. Third-party software inside Qt'
Add-Line 'Qt contains the following components, which belong to the Qt modules above. The text of each entry is the attribution'
Add-Line "page that Qt publishes for it (Qt $($kit.Version) documentation, generated from the qt_attribution.json files with"
Add-Line 'qtattributionsscanner), including the full license text. A few entries apply to other platforms or optional'
Add-Line 'features of the same module and are listed because Qt lists them for the module.'
$pageOrder = @($components.Keys | ForEach-Object { ConvertFrom-AttributionPage $_ })
foreach ($name in $supplemented.Keys) {
    $sp = $sbom[$name]
    $cn = if ($sp.Comment -match '(?m)^\s*Name:\s*(.+)$') { $Matches[1].Trim() } else { $name }
    $desc = if ($sp.Comment -match '(?m)^\s*Description:\s*(.+)$') { $Matches[1].Trim() } else { '' }
    $body = "$desc`n`nLicense: $($sp.License)`n`n $($sp.Copyright)`n`n" + (Read-TextFile $supplemented[$name]).Trim("`n")
    $pageOrder += [pscustomobject]@{ Title = $cn; Body = $body.Trim("`n") }
}
$pageOrder = @($pageOrder | Sort-Object Title -Unique)
foreach ($pg in $pageOrder) {
    Add-Line; Add-Line ('-' * 100); Add-Line $pg.Title; Add-Line ('-' * 100); Add-Line
    Add-Line $pg.Body
}

Add-Rule '4. Microsoft Visual C++ runtime'
$crt = @($files | Where-Object { $_.Name -match '^(vcruntime|msvcp|concrt|vccorlib|vcomp)' -and $_.Rel -notlike '*\*' } | ForEach-Object { $_.Name })
if ($crt.Count -gt 0) {
    Add-Line "The Microsoft Visual C++ runtime DLLs in this folder ($($crt -join ', '))"
    Add-Line 'are redistributable files of Microsoft Visual Studio, distributed under the Visual Studio license terms'
    Add-Line '(redistributable code, "redist.txt").'
} else {
    Add-Line 'This folder contains no Microsoft Visual C++ runtime DLLs.'
}

Add-Rule "5. Rust crates ($($crates.Count))"
Add-Line "Crates compiled into tinwave-desktop.exe (normal dependencies of: $($Package -join ', ')), with the license each declares."
Add-Line 'Where a license offers a choice (A OR B), Tinwave uses and redistributes the crate under either. Texts follow in section 6.'
if ($mplCrates.Count -gt 0) {
    Add-Line
    Add-Line ('MPL-2.0 (file-level copyleft) applies to: ' + ($mplCrates -join ', ') + '.')
    Add-Line 'These crates are used UNMODIFIED. Their source is available from the "source" URL of each entry and from crates.io'
    Add-Line '(https://crates.io/crates/<name>/<version>); any change made to them would be published under MPL-2.0.'
}
Add-Line
foreach ($r in $crateRows) {
    Add-Line ('  {0} - {1}' -f $r.Label, $r.License)
    if ($r.Authors) { Add-Line ('      authors: ' + $r.Authors) }
    if ($r.Repo) { Add-Line ('      source: ' + $r.Repo) }
}

Add-Rule '6. License texts of the Rust crates'
$n = 0
foreach ($t in ($texts.Values | Sort-Object { $_.Names[0] })) {
    $n++
    Add-Line ('-' * 100)
    Add-Line "Text $n, used by:"
    foreach ($nm in ($t.Names | Sort-Object -Unique)) { Add-Line "  $nm" }
    Add-Line ('-' * 100); Add-Line
    Add-Line $t.Text; Add-Line
}

if ($assetLicenses.Count -gt 0) {
    Add-Rule '7. Icons and other assets'
    foreach ($a in $assetLicenses) {
        $rel = $a.FullName.Substring($script:RepoRoot.Length + 1).Replace('\', '/')
        Add-Line ('-' * 100); Add-Line $rel; Add-Line ('-' * 100); Add-Line
        Add-Line ((Read-TextFile $a.FullName).Trim()); Add-Line
    }
}

Write-TextFile $Out $sb.ToString()
Write-Host ("Wrote {0} ({1:N0} bytes): Qt {2} modules/plugins {3}, Qt components {4}, crates {5}, asset licenses {6}" -f $Out, (Get-Item $Out).Length, $kit.Version, $shipped.Count, ($components.Count + $supplemented.Count), $crates.Count, $assetLicenses.Count)
