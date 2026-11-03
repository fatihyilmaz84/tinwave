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
#   * Rust std      the standard library statically linked into the exe, and the crates it vendors (hashbrown, rustc-demangle,
#                   ...). The list is read from the link itself: the rlibs named in the exe's pdb (target\release\
#                   tinwave_desktop.pdb) that exist in the pinned toolchain's sysroot. Notices and texts come from the
#                   toolchain's own share\doc\rust\COPYRIGHT-library.html and share\doc\rust\licenses\ (no hand-typed list).
#   * Assets        every LICENSE* file under apps\tinwave-desktop\icons\ (Lucide ISC etc.) when that folder exists.
#
# It fails (non-zero exit, nothing written) rather than skipping a component it cannot document.
param(
    [Parameter(Mandatory)][string]$Bundle,
    [string]$Out = "",
    [string[]]$Package = @('tinwave-desktop'),
    [string]$QtDir = "",
    # The linker's module list of the exe: it names the Rust std rlibs that were linked (default: the desktop build's pdb).
    [string]$Pdb = "",
    [string]$IdentityFile = ""
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')
if (-not $Out) { $Out = Join-Path $PSScriptRoot 'THIRD-PARTY-NOTICES.txt' }
if (-not $Pdb) { $Pdb = Join-Path $script:RepoRoot 'target\release\tinwave_desktop.pdb' }
$identity = Get-ReleaseIdentity $IdentityFile

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
$meta = (& cargo metadata --format-version 1 --locked --offline --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $script:RepoRoot 'Cargo.toml') | Out-String) | ConvertFrom-Json
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
# The copyright holder for a crate that ships no license file: the `SPDX-FileCopyrightText: <years> <holder>` lines in its
# sources (cxx-qt: Klaralvdalens Datakonsult AB, a KDAB Group company), else the Cargo.toml authors. Never a guess: fails
# when the crate names nobody.
function Get-CrateHolders([string]$Dir, [string]$Authors, [string]$Label) {
    $by = @{}   # holder -> @{ Min; Max }
    $exts = '.rs', '.toml', '.h', '.hpp', '.cpp', '.cmake', '.md', '.txt'
    foreach ($f in (Get-ChildItem $Dir -Recurse -File | Where-Object { $exts -contains $_.Extension.ToLowerInvariant() })) {
        $txt = Read-TextFile $f.FullName
        foreach ($m in [regex]::Matches($txt, '(?m)SPDX-FileCopyrightText:[ \t]*(?:(?<y1>\d{4})(?:[ \t]*[-,][ \t]*(?<y2>\d{4}))?[ \t]+)?(?<h>[^\r\n]+?)[ \t]*$')) {
            $h = ($m.Groups['h'].Value -replace '\s*(-->|\*/)\s*$', '').Trim()
            $y1 = if ($m.Groups['y1'].Success) { [int]$m.Groups['y1'].Value } else { 0 }
            $y2 = if ($m.Groups['y2'].Success) { [int]$m.Groups['y2'].Value } else { $y1 }
            if (-not $by.ContainsKey($h)) { $by[$h] = @{ Min = 9999; Max = 0 } }
            if ($y1 -gt 0) { $by[$h].Min = [Math]::Min($by[$h].Min, $y1); $by[$h].Max = [Math]::Max($by[$h].Max, [Math]::Max($y1, $y2)) }
        }
    }
    if ($by.Count -gt 0) {
        $parts = foreach ($h in ($by.Keys | Sort-Object)) {
            $y = $by[$h]
            $years = if ($y.Max -eq 0) { '' } elseif ($y.Min -eq $y.Max) { "$($y.Min) " } else { "$($y.Min)-$($y.Max) " }
            "$years$h"
        }
        return [pscustomobject]@{ Text = ($parts -join "`nCopyright (c) "); From = 'the SPDX-FileCopyrightText lines in the crate sources' }
    }
    if ($Authors) { return [pscustomobject]@{ Text = $Authors; From = 'the Cargo.toml authors' } }
    throw "Crate $Label ships no license file and names no copyright holder (no SPDX-FileCopyrightText lines, no Cargo.toml authors): cannot write its MIT/BSD notice without inventing one."
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
        $holders = Get-CrateHolders $dir $authors $label
        $ids = @($expr -split '[\s()]+' | Where-Object { $_ -and $_ -notin 'OR', 'AND', 'WITH' } | Select-Object -Unique)
        foreach ($sid in $ids) {
            $tf = Join-Path $licenseTextDir "$sid.txt"
            if (-not (Test-Path $tf)) { throw "Crate $label ships no license file and packaging\license-texts\$sid.txt does not exist. Add the canonical $sid text there." }
            $t = (Read-TextFile $tf).Replace('{{HOLDERS}}', $holders.Text)
            Add-LicenseText $t "$label (no license file in the crate; canonical $sid text, copyright holder from $($holders.From))"
        }
    }
    if ($expr -match 'MPL-2\.0') { $mplCrates.Add($label) }
    $crateRows.Add([pscustomobject]@{ Label = $label; License = $expr; Repo = $c.repository; Authors = $authors; Files = (@($lf | ForEach-Object { $_.Name }) -join ', ') })
}

# --- 3b. Rust standard library and the crates vendored by it ---------------------------------------------------------
# std is statically linked into every Rust exe. Which of its crates really are is in the link: the module list of the exe's
# pdb names every rlib the linker took, and a std rlib is recognised by existing, with the same hash in its file name, in the
# pinned toolchain's sysroot (a Cargo crate's rlib lives in target\release\deps instead). On x86_64-pc-windows-msvc this is
# alloc, compiler_builtins, core, hashbrown, panic_unwind, rustc_demangle and std: gimli, addr2line, miniz_oxide, ... are
# not linked here (backtraces use dbghelp), so they are not listed. The Rust project publishes the notices and texts of
# std and of every crate it vendors in the toolchain (share\doc\rust\COPYRIGHT-library.html, share\doc\rust\licenses).
Push-Location $script:RepoRoot
try {
    $sysroot = ((& rustc --print sysroot) | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $sysroot) { throw "rustc --print sysroot failed" }
    $rustcVersion = ((& rustc --version) | Out-String).Trim()
} finally { Pop-Location }
if (-not (Test-Path $Pdb)) { throw "$Pdb not found: it lists the Rust std crates the exe links. Build tinwave-desktop first (release, MSVC debug info is on by default) or pass -Pdb." }
$stdLibDir = Join-Path $sysroot 'lib\rustlib\x86_64-pc-windows-msvc\lib'
$pdbText = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($Pdb))
$stdLinked = New-Object Collections.Generic.SortedSet[string]
foreach ($m in [regex]::Matches($pdbText, 'lib([A-Za-z0-9_]+)-([0-9a-f]{16})\.rlib')) {
    if (Test-Path (Join-Path $stdLibDir ("lib$($m.Groups[1].Value)-$($m.Groups[2].Value).rlib"))) { [void]$stdLinked.Add($m.Groups[1].Value) }
}
if (-not ($stdLinked.Contains('std') -and $stdLinked.Contains('core'))) {
    throw "$Pdb names no std/core rlib of the sysroot ${sysroot}: it does not belong to this toolchain's build of tinwave-desktop."
}
$rustDoc = Join-Path $sysroot 'share\doc\rust'
$rustCopyright = Join-Path $rustDoc 'COPYRIGHT-library.html'
if (-not (Test-Path $rustCopyright)) { throw "$rustCopyright not found: the toolchain does not ship the Rust standard library notices." }
$cr = Read-TextFile $rustCopyright
$iTree = $cr.IndexOf('id="in-tree-files"'); $iOot = $cr.IndexOf('id="out-of-tree-dependencies"')
if ($iTree -lt 0 -or $iOot -lt $iTree) { throw "Unexpected layout of $rustCopyright" }
function Get-HtmlField([string]$Html, [string]$Label) {
    $m = [regex]::Match($Html, "<b>$Label[:]?</b>\s*(?<v>.*?)</p>", 'Singleline')
    if ($m.Success) { [Net.WebUtility]::HtmlDecode(($m.Groups['v'].Value -replace '<[^>]+>', '').Trim()) } else { '' }
}
# In-tree: library/ of the Rust repository
$stdTree = New-Object Collections.Generic.List[object]
foreach ($blk in ($cr.Substring($iTree, $iOot - $iTree) -split '<b>File/Directory:</b>' | Select-Object -Skip 1)) {
    $path = [Net.WebUtility]::HtmlDecode(([regex]::Match($blk, '<code>(.*?)</code>')).Groups[1].Value)
    $stdTree.Add([pscustomobject]@{
        Path = $path; License = (Get-HtmlField $blk 'License')
        Copyright = @([regex]::Matches($blk, '<b>Copyright:</b>\s*(?<v>[^<]+)</p>') | ForEach-Object { [Net.WebUtility]::HtmlDecode($_.Groups['v'].Value.Trim()) })
    })
}
if ($stdTree.Count -eq 0) { throw "No in-tree entries in $rustCopyright" }
# Out-of-tree: crates.io crates vendored by std
$stdVendored = New-Object Collections.Generic.List[object]
foreach ($blk in ($cr.Substring($iOot) -split '(?=<h3>)' | Where-Object { $_ -like '<h3>*' })) {
    if ($blk -notmatch '^<h3>\S+\s+(?<id>[^<]+?)</h3>') { continue }
    $id = $Matches['id']
    if ($id -notmatch '^(?<n>.+?)-(?<v>\d.*)$') { throw "Unexpected crate id '$id' in $rustCopyright" }
    $notices = @([regex]::Matches($blk, '(?s)<summary><code>(?<f>.*?)</code></summary>\s*<pre>(?<t>.*?)</pre>') | ForEach-Object {
        [pscustomobject]@{ File = [Net.WebUtility]::HtmlDecode($_.Groups['f'].Value); Text = [Net.WebUtility]::HtmlDecode($_.Groups['t'].Value) }
    })
    $stdVendored.Add([pscustomobject]@{
        Name = $Matches['n']; Version = $Matches['v']; Norm = ($Matches['n'] -replace '-', '_').ToLowerInvariant()
        Authors = (Get-HtmlField $blk 'Authors'); License = (Get-HtmlField $blk 'License'); Notices = $notices
    })
}
if ($stdVendored.Count -eq 0) { throw "No out-of-tree entries in $rustCopyright" }
$stdLinkedVendored = @($stdVendored | Where-Object { $stdLinked.Contains($_.Norm) })
$stdLinkedInTree = @($stdLinked | Where-Object { $n = $_; -not ($stdVendored | Where-Object { $_.Norm -eq $n }) })
$stdNotLinked = @($stdVendored | Where-Object { -not $stdLinked.Contains($_.Norm) } | ForEach-Object { "$($_.Name) $($_.Version)" })
foreach ($v in $stdLinkedVendored) {
    if ($v.Notices.Count -eq 0) { throw "Rust std crate $($v.Name) $($v.Version) has no license text in $rustCopyright" }
    foreach ($nt in $v.Notices) { Add-LicenseText $nt.Text "$($v.Name) $($v.Version) (Rust standard library; $($nt.File))" }
}
foreach ($t in $stdTree) {
    foreach ($sid in @($t.License -split '[\s()]+' | Where-Object { $_ -and $_ -notin 'OR', 'AND', 'WITH' } | Select-Object -Unique)) {
        $tf = Join-Path $rustDoc "licenses\$sid.txt"
        if (-not (Test-Path $tf)) { throw "Rust std in-tree license $sid (for $($t.Path)) has no text in $rustDoc\licenses" }
        $body = (Read-TextFile $tf) -replace '<year>\s*<(?:copyright holders|owner)>', ($t.Copyright -join "`nCopyright (c) ")
        Add-LicenseText $body "Rust standard library, $($t.Path) ($sid)"
    }
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
Add-Line "Copyright holder: $($identity.HolderText)"

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

# Acknowledgements that FreeType's FTL (clause 6) and the IJG license (libjpeg) require in the documentation of a binary
# distribution. They are emitted for the components that are really in this folder; the FreeType year is the newest copyright
# year of the FreeType version Qt bundles (the FTL says: the value from the version you use).
$ftPages = @($pageOrder | Where-Object { $_.Title -match '(?i)^freetype' })
$ijgPages = @($pageOrder | Where-Object { $_.Body -match 'Independent JPEG\s+Group' })
if ($ftPages.Count -gt 0 -or $ijgPages.Count -gt 0) {
    Add-Line
    Add-Line 'Acknowledgements required by the licenses of these components:'
    Add-Line
    if ($ftPages.Count -gt 0) {
        $years = @($ftPages | ForEach-Object { [regex]::Matches($_.Body, '(?im)^\s*Copyright[^\n]*') } | ForEach-Object { [regex]::Matches($_.Value, '\b(?:19|20)\d\d\b') } | ForEach-Object { [int]$_.Value })
        if ($years.Count -eq 0) { throw "FreeType is shipped but its attribution pages carry no copyright year for the FTL acknowledgement." }
        Add-Line ("  Portions of this software are copyright (c) {0} The FreeType Project (www.freetype.org). All rights reserved." -f ($years | Measure-Object -Maximum).Maximum)
    }
    if ($ijgPages.Count -gt 0) { Add-Line '  This software is based in part on the work of the Independent JPEG Group.' }
}
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
Add-Line 'Where a license offers a choice (A OR B), Tinwave uses and redistributes the crate under either. Texts follow in section 7.'
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

Add-Rule '6. Rust standard library'
Add-Line "tinwave-desktop.exe statically links the Rust standard library of the pinned toolchain ($rustcVersion)."
Add-Line 'The list below is not typed by hand: it is the set of std rlibs that the linker took for this exe (module list of'
Add-Line ('{0}), and the notices come from the toolchain itself (share\doc\rust\COPYRIGHT-library.html and' -f (Split-Path $Pdb -Leaf))
Add-Line 'share\doc\rust\licenses). Texts follow in section 7.'
Add-Line
Add-Line ('Linked crates of the standard library source tree: ' + ($stdLinkedInTree -join ', ') + '.')
Add-Line 'The Rust project documents the in-tree source files of this release as (the entries for other platforms are listed'
Add-Line 'because the Rust project lists them for the library):'
foreach ($t in $stdTree) {
    Add-Line ('  {0} - {1}' -f $t.Path, $t.License)
    foreach ($cp in $t.Copyright) { Add-Line ('      copyright: ' + $cp) }
}
Add-Line
Add-Line 'Crates vendored by the standard library and linked into this exe:'
foreach ($v in $stdLinkedVendored) {
    Add-Line ('  {0} {1} - {2}' -f $v.Name, $v.Version, $v.License)
    if ($v.Authors) { Add-Line ('      authors: ' + $v.Authors) }
    Add-Line ('      source: https://crates.io/crates/{0}/{1}' -f $v.Name, $v.Version)
}
Add-Line
Add-Line 'Crates the Rust project lists for the standard library that contribute no code to this program (other targets, the test'
Add-Line ('harness, build tools, crates without object code): ' + ($stdNotLinked -join ', ') + '.')

Add-Rule '7. License texts of the Rust crates and the Rust standard library'
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
    Add-Rule '8. Icons and other assets'
    foreach ($a in $assetLicenses) {
        $rel = $a.FullName.Substring($script:RepoRoot.Length + 1).Replace('\', '/')
        Add-Line ('-' * 100); Add-Line $rel; Add-Line ('-' * 100); Add-Line
        Add-Line ((Read-TextFile $a.FullName).Trim()); Add-Line
    }
}

Write-TextFile $Out $sb.ToString()
Write-Host ("Wrote {0} ({1:N0} bytes): Qt {2} modules/plugins {3}, Qt components {4}, crates {5}, asset licenses {6}" -f $Out, (Get-Item $Out).Length, $kit.Version, $shipped.Count, ($components.Count + $supplemented.Count), $crates.Count, $assetLicenses.Count)
