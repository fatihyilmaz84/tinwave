# Compliance and self-containment check of a deployed Tinwave bundle (ADR 0008 decision 3; ticket T-0042).
#
#   .\check-bundle.ps1 -Bundle ..\..\..\target\package\tinwave-desktop-windows-x64
#
# Exit code 0 = every check passed. Exit code 1 = at least one FAIL line was printed; package.ps1 stops on it.
# The checks (each prints PASS or FAIL):
#   shared-qt     the Qt kit is a shared build and Cargo links no Qt object files (LGPL-3.0: dynamic linking only)
#   static-qt     tinwave-desktop.exe imports Qt6Core.dll, i.e. Qt is dynamic, not linked into the exe
#   gpl-only      no shipped Qt DLL is GPL-only (per the kit's SPDX SBOM); no Qt test modules; the exe carries no trace of a
#                 GPL-only module (the test-utils are static libraries in the kit, so they would be linked INTO the exe)
#   link-libs     every Qt library that a build script asked the linker for (`rustc-link-lib` lines in target\release\build\
#                 *\output) is in the kit's SBOM, is not GPL-only and is not linked statically
#   qt-unmodified every Qt file in the bundle is byte-identical to the file in the kit (LGPL: Qt is used unmodified)
#   only-app-exe  the one executable is tinwave-desktop.exe (never image-stress.exe, Qt tools, pdb, lib files)
#   closure       every import of every PE file resolves inside the bundle or to a Windows system DLL;
#                 the MSVC runtime must be in the bundle (app-local), it is never assumed to be installed
#   legal         THIRD-PARTY-NOTICES.txt (with the FreeType and IJG acknowledgements), the LGPL/GPL texts and the Qt source
#                 offer are in the bundle
#   source-offer  the offer links the source archive of every Qt repository the shipped Qt files are built from (SBOM),
#                 at download.qt.io/archive
#   d0007         the PO decision on copyright holder / offer contact: reported as PENDING (not a failure) until decided; with
#                 -Release a pending item is a FAIL; a bundle generated before the decision is stale and fails
param(
    [Parameter(Mandatory)][string]$Bundle,
    [string]$QtDir = "",
    # Build-script outputs (cargo's target\release\build): the `rustc-link-lib` lines name every library the exe was linked with.
    [string]$BuildDir = "",
    # Release identity (D-0007), default packaging\release-identity.json.
    [string]$Identity = "",
    # Treat a pending D-0007 as a failure (the build that is meant to be published).
    [switch]$Release
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')

$failures = New-Object Collections.Generic.List[string]
function Pass([string]$check, [string]$msg) { Write-Host ("PASS {0,-13} {1}" -f $check, $msg) }
function Fail([string]$check, [string]$msg) {
    Write-Host ("FAIL {0,-13} {1}" -f $check, $msg) -ForegroundColor Red
    $failures.Add("${check}: $msg")
}

if (-not (Test-Path $Bundle)) { throw "Bundle folder not found: $Bundle" }
$root = (Resolve-Path $Bundle).Path.TrimEnd('\')
$kit = Resolve-QtKit $QtDir
$files = @(Get-BundleFiles $root)
$exe = Join-Path $root 'tinwave-desktop.exe'

# --- shared-qt -------------------------------------------------------------------------------------------------------
$enabled = @(Get-QtConfigValue $kit.Root 'QT.global.enabled_features')
$disabled = @(Get-QtConfigValue $kit.Root 'QT.global.disabled_features')
if (($enabled -contains 'shared') -and -not ($enabled -contains 'static') -and ($disabled -contains 'static')) {
    Pass 'shared-qt' "Qt kit $($kit.Version) is a shared build (mkspecs\qconfig.pri)"
} else {
    Fail 'shared-qt' "Qt kit $($kit.Root) is not a shared (dynamic) build; Tinwave uses Qt under LGPL-3.0 by dynamic linking only"
}

$meta = (& cargo metadata --format-version 1 --locked --offline --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $script:RepoRoot 'Cargo.toml') | Out-String) | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed" }
$objFeature = @($meta.resolve.nodes | Where-Object { $_.features -contains 'link_qt_object_files' } | ForEach-Object { $_.id })
if ($objFeature.Count -eq 0) {
    Pass 'shared-qt' "no crate in the build enables cxx-qt's link_qt_object_files"
} else {
    Fail 'shared-qt' "link_qt_object_files is enabled by: $($objFeature -join ', ') (links Qt statically; forbidden by ADR 0008)"
}

# --- static-qt -------------------------------------------------------------------------------------------------------
if (-not (Test-Path $exe)) {
    Fail 'static-qt' "tinwave-desktop.exe is missing from the bundle root"
} else {
    $imp = Get-PeImports $exe
    $qtImports = @($imp.Imports | Where-Object { $_ -match '^Qt6.*\.dll$' })
    if ($qtImports -contains 'Qt6Core.dll' -and ($qtImports -contains 'Qt6Gui.dll' -or $qtImports -contains 'Qt6Qml.dll')) {
        Pass 'static-qt' "tinwave-desktop.exe imports Qt dynamically: $($qtImports -join ', ')"
    } else {
        Fail 'static-qt' "tinwave-desktop.exe does not import Qt6Core.dll plus Qt6Gui/Qt6Qml: Qt appears to be linked statically (imports: $(($imp.Imports) -join ', '))"
    }
}

# --- gpl-only --------------------------------------------------------------------------------------------------------
$sbom = Read-QtSbom $kit.Root
$gplBad = New-Object Collections.Generic.List[string]
$nQt = 0
foreach ($f in $files) {
    $isDll = $f.Name -like '*.dll'
    if ($f.Name -match 'TestUtils' -or $f.Name -match '^Qt6Test' -or $f.Name -match '^Qt6QmlTest') { $gplBad.Add("$($f.Rel) (Qt test module / test-utils; GPL-only)"); continue }
    if (-not $isDll) { continue }
    if ($f.Name -like 'vc*.dll' -or $f.Name -like 'msvcp*.dll' -or $f.Name -like 'concrt*.dll') { continue }
    $pk = @(Find-QtSbomPackages $f.Path $sbom)
    if ($pk.Count -eq 0) { $gplBad.Add("$($f.Rel) (no SBOM entry in the Qt kit: license cannot be verified)"); continue }
    $nQt++
    foreach ($p in $pk) {
        if (Test-GplOnly $p.License) { $gplBad.Add("$($f.Rel) -> $($p.Name): $($p.License)") }
    }
}
# GPL-only modules of the kit (SBOM), without the GPL tools (moc, rcc, ...: "WITH Qt-GPL-exception", never linked into apps).
$gplModules = @($sbom.Values | Where-Object { $_.Name -notmatch '_Attribution_' -and $_.License -notmatch '\sWITH\s' -and (Test-GplOnly $_.License) } |
    ForEach-Object { $_.Name -replace 'Private$', '' })
$tokens = @($gplModules + 'TestUtils' | Select-Object -Unique)
if (Test-Path $exe) {
    $bytes = [IO.File]::ReadAllBytes($exe)
    $latin = [Text.Encoding]::GetEncoding(28591).GetString($bytes)
    foreach ($tk in $tokens) {
        if ($latin.Contains($tk)) { $gplBad.Add("tinwave-desktop.exe contains the string '$tk': a GPL-only Qt module was linked in") }
    }
}
if ($gplBad.Count -eq 0) { Pass 'gpl-only' "$nQt Qt binaries checked against the kit SBOMs ($($gplModules.Count) GPL-only modules known, e.g. $($gplModules -join ', ')): none shipped, none in the exe, no test modules" }
else { foreach ($b in $gplBad) { Fail 'gpl-only' $b } }

# --- link-libs -------------------------------------------------------------------------------------------------------
# The build scripts (cxx-qt-build) tell cargo which Qt libraries to link: that is the authoritative list of what went into
# the exe, whatever its strings say. Each one must be a Qt module in the SBOM that is not GPL-only, linked as a DLL.
if (-not $BuildDir) { $BuildDir = Join-Path $script:RepoRoot 'target\release\build' }
$links = @(Read-RustcLinkLibs $BuildDir)
if ($links.Count -eq 0) {
    Fail 'link-libs' "no rustc-link-lib lines found under ${BuildDir}: build tinwave-desktop in release mode first; the Qt libraries the exe was linked with cannot be verified"
} else {
    $qtLinks = @($links | Where-Object { $_.Name -match '^Qt\d' })
    $linkBad = New-Object Collections.Generic.List[string]
    foreach ($l in $qtLinks) {
        $pk = Find-QtLibSbomPackage $l.Name $sbom
        if ($null -eq $pk) { $linkBad.Add("$($l.Name) (from build script $($l.Source)) has no entry in the Qt kit SBOM: license cannot be verified") }
        elseif (Test-GplOnly $pk.License) { $linkBad.Add("$($l.Name) (from build script $($l.Source)) is GPL-only: $($pk.License)") }
        if ($l.Kind -eq 'static') { $linkBad.Add("$($l.Name) (from build script $($l.Source)) is linked statically (kind=static); Qt must be linked dynamically") }
    }
    if ($qtLinks.Count -eq 0) { $linkBad.Add("no Qt library appears in any rustc-link-lib line under $BuildDir") }
    $qtLibNames = @($qtLinks | ForEach-Object { $_.Name } | Sort-Object -Unique)
    if ($linkBad.Count -eq 0) { Pass 'link-libs' "$($qtLibNames.Count) Qt libraries requested by the build scripts ($($qtLibNames -join ', ')): all in the kit SBOM, none GPL-only, none static" }
    else { foreach ($b in ($linkBad | Select-Object -Unique)) { Fail 'link-libs' $b } }
}

# --- qt-unmodified ---------------------------------------------------------------------------------------------------
# windeployqt copies the kit's files verbatim. Everything in the bundle that is not ours (exe, MSVC runtime, legal files)
# must therefore be byte-identical to the file it came from in the kit. The LGPL notice says "Qt is used unmodified".
$crtName = '^(vcruntime|msvcp|concrt|vccorlib|vcomp)\d*(_\w+)?\.dll$'
$sha = [Security.Cryptography.SHA256]::Create()
$qtBad = New-Object Collections.Generic.List[string]
$nSame = 0
foreach ($f in $files) {
    if (($f.Rel -in 'tinwave-desktop.exe', '.tinwave-package', 'THIRD-PARTY-NOTICES.txt', 'SOURCE-OFFER-Qt.txt') -or $f.Rel -like 'licenses\*') { continue }
    if ($f.Rel -notlike '*\*' -and $f.Name.ToLowerInvariant() -match $crtName) { continue }
    $kf = Get-KitCounterpart $kit.Root $f.Rel
    if (-not $kf) { $qtBad.Add("$($f.Rel) is not a file of the Qt kit and not one of ours"); continue }
    $kl = (Get-Item -LiteralPath $kf).Length
    if ($kl -ne $f.Length) { $qtBad.Add("$($f.Rel) differs from the kit's file (size $($f.Length) vs $kl)"); continue }
    $h1 = [BitConverter]::ToString($sha.ComputeHash([IO.File]::ReadAllBytes($f.Path)))
    $h2 = [BitConverter]::ToString($sha.ComputeHash([IO.File]::ReadAllBytes($kf)))
    if ($h1 -ne $h2) { $qtBad.Add("$($f.Rel) differs from the kit's file (same size, different content)") } else { $nSame++ }
}
if ($qtBad.Count -eq 0) { Pass 'qt-unmodified' "$nSame Qt files in the bundle are byte-identical to the Qt $($kit.Version) kit; nothing else unexplained" }
else { foreach ($b in $qtBad) { Fail 'qt-unmodified' $b } }

# --- only-app-exe ----------------------------------------------------------------------------------------------------
$stray = @($files | Where-Object { $_.Name -match '\.(exe|pdb|lib|exp|ilk|obj|rlib|a)$' -and $_.Rel -ne 'tinwave-desktop.exe' })
if ($stray.Count -eq 0 -and (Test-Path $exe)) { Pass 'only-app-exe' "tinwave-desktop.exe is the only executable; no pdb/lib files" }
else { foreach ($s in $stray) { Fail 'only-app-exe' "unexpected file in the bundle: $($s.Rel)" } }

# --- closure ---------------------------------------------------------------------------------------------------------
$sys32 = Join-Path $env:SystemRoot 'System32'
$crtPattern = '^(vcruntime|msvcp|concrt|vccorlib|vcomp)\d*(_\w+)?\.dll$'
$rootNames = @{}
foreach ($f in $files) { if ($f.Rel -notlike '*\*') { $rootNames[$f.Name.ToLowerInvariant()] = $true } }
$open = New-Object Collections.Generic.List[string]
$nPe = 0
foreach ($f in $files) {
    if ($f.Name -notmatch '\.(dll|exe)$') { continue }
    $nPe++
    $imp = Get-PeImports $f.Path
    foreach ($i in ($imp.Imports | Sort-Object -Unique)) {
        $l = $i.ToLowerInvariant()
        if ($rootNames.ContainsKey($l)) { continue }
        if ($l -match $crtPattern) { $open.Add("$($f.Rel) needs $i, which must be in the bundle root (app-local MSVC runtime)"); continue }
        if ($l -match '^(api|ext)-ms-') { continue }
        if (Test-Path (Join-Path $sys32 $i)) { continue }
        $open.Add("$($f.Rel) imports $i, which is neither in the bundle root nor a Windows system DLL")
    }
}
if ($open.Count -eq 0) { Pass 'closure' "$nPe PE files: all imports resolve inside the bundle root or to Windows system DLLs; MSVC runtime is app-local" }
else { foreach ($o in ($open | Select-Object -Unique)) { Fail 'closure' $o } }

# --- legal -----------------------------------------------------------------------------------------------------------
$legal = @('THIRD-PARTY-NOTICES.txt', 'SOURCE-OFFER-Qt.txt', 'licenses\Qt-LGPL-3.0-and-GPL-3.0.txt')
$legalOk = $true
foreach ($l in $legal) {
    $p = Join-Path $root $l
    if (-not (Test-Path $p) -or (Get-Item $p).Length -lt 1000) { Fail 'legal' "$l is missing or empty"; $legalOk = $false }
}
$lgpl = Join-Path $root 'licenses\Qt-LGPL-3.0-and-GPL-3.0.txt'
if (Test-Path $lgpl) {
    $t = Read-TextFile $lgpl
    if ($t -notmatch 'GNU LESSER GENERAL PUBLIC LICENSE\s+Version 3' -or $t -notmatch 'GNU GENERAL PUBLIC LICENSE\s+Version 3') {
        Fail 'legal' 'licenses\Qt-LGPL-3.0-and-GPL-3.0.txt does not contain both the LGPL-3.0 and the GPL-3.0 text'; $legalOk = $false
    }
}
$notices = if (Test-Path (Join-Path $root 'THIRD-PARTY-NOTICES.txt')) { Read-TextFile (Join-Path $root 'THIRD-PARTY-NOTICES.txt') } else { '' }
# FreeType's FTL (clause 6) and the IJG license need an acknowledgement sentence in the documentation, not only the license text.
if ($notices -match 'FreeType' -and $notices -notmatch 'Portions of this software are copyright \(c\) \d{4} The FreeType Project \(www\.freetype\.org\)\. All rights reserved\.') {
    Fail 'legal' 'THIRD-PARTY-NOTICES.txt carries FreeType but not the FTL acknowledgement "Portions of this software are copyright (c) <year> The FreeType Project (www.freetype.org). All rights reserved."'; $legalOk = $false
}
if ($notices -match 'Independent JPEG\s+Group' -and $notices -notmatch 'This software is based in part on the work of the Independent JPEG Group\.') {
    Fail 'legal' 'THIRD-PARTY-NOTICES.txt carries the IJG license but not the sentence "This software is based in part on the work of the Independent JPEG Group."'; $legalOk = $false
}
if ($notices -and $notices -notmatch '(?m)^\d+\. Rust standard library$') {
    Fail 'legal' 'THIRD-PARTY-NOTICES.txt has no "Rust standard library" section (std is statically linked into the exe)'; $legalOk = $false
}
if ($legalOk) { Pass 'legal' 'notices (FreeType and IJG acknowledgements, Rust std), Qt source offer and LGPL-3.0 / GPL-3.0 texts are present' }

# --- source-offer ----------------------------------------------------------------------------------------------------
# One source archive per Qt repository that a shipped Qt file is built from, derived from the SBOM, not from a fixed list.
$offerPath = Join-Path $root 'SOURCE-OFFER-Qt.txt'
if (Test-Path $offerPath) {
    $offer = Read-TextFile $offerPath
    $repos = @(Get-ShippedQtRepos $files $sbom)
    $offerBad = New-Object Collections.Generic.List[string]
    foreach ($r in $repos) {
        $url = Get-QtSourceUrl $r.Repo $kit.Version
        if (-not $offer.Contains($url)) { $offerBad.Add("the source offer does not link the source of Qt repository '$($r.Repo)' ($($r.Modules.Count) shipped modules, e.g. $($r.Modules[0])): $url") }
    }
    $archive = "https://download.qt.io/archive/qt/$($kit.Version -replace '^(\d+\.\d+)\..*$', '$1')/$($kit.Version)/"
    if (-not $offer.Contains($archive)) { $offerBad.Add("the source offer does not point to the archive $archive") }
    if ($offer -match 'official_releases') { $offerBad.Add('the source offer links download.qt.io/official_releases, which Qt prunes; use download.qt.io/archive') }
    if ($offer -match '\{\{') { $offerBad.Add('the source offer still contains an unfilled {{placeholder}}') }
    if ($offerBad.Count -eq 0) { Pass 'source-offer' "the offer links the source archive of each of the $($repos.Count) Qt repositories shipped ($(($repos | ForEach-Object { $_.Repo }) -join ', ')) at download.qt.io/archive" }
    else { foreach ($b in $offerBad) { Fail 'source-offer' $b } }
} else {
    Fail 'source-offer' 'SOURCE-OFFER-Qt.txt is missing'
}

# --- d0007 -----------------------------------------------------------------------------------------------------------
# Copyright holder and offer contact are PO decisions (work/decisions/D-0007). Pending = the generated files carry the marker
# "[D-0007 PENDING: ...]" and this check says so; it does not fail the package unless -Release is given. Once decided
# (release-identity.json filled in), a bundle that still has the marker is stale and fails.
$pendingMarker = '[D-0007 PENDING'
$rid = Get-ReleaseIdentity $Identity
$offerText = if (Test-Path $offerPath) { Read-TextFile $offerPath } else { '' }
$markerInBundle = $notices.Contains($pendingMarker) -or $offerText.Contains($pendingMarker)
if ($rid.Holder -and -not $notices.Contains($rid.Holder)) { Fail 'd0007' "THIRD-PARTY-NOTICES.txt does not name the copyright holder '$($rid.Holder)' from release-identity.json: regenerate the bundle" }
if ($rid.Contact -and -not $offerText.Contains($rid.Contact)) { Fail 'd0007' "SOURCE-OFFER-Qt.txt does not contain the offer contact '$($rid.Contact)' from release-identity.json: regenerate the bundle" }
if ($rid.Holder -and $rid.Contact -and $markerInBundle) { Fail 'd0007' 'the bundle still carries a "[D-0007 PENDING" marker although release-identity.json is filled in: it is stale, regenerate it' }
if ($rid.Pending.Count -gt 0) {
    $msg = "D-0007 pending (PO decision; blocks a public release): " + ($rid.Pending -join '; ')
    if ($Release) { Fail 'd0007' "$msg. -Release was given: do not publish this bundle." }
    else { Write-Host ("PENDING {0,-13} {1}" -f 'd0007', $msg) -ForegroundColor Yellow }
} elseif (-not ($failures | Where-Object { $_ -like 'd0007:*' })) {
    Pass 'd0007' 'copyright holder, source-offer contact and root license files are set; the bundle names them'
}

if ($failures.Count -gt 0) {
    Write-Host ""
    Write-Host "check-bundle: $($failures.Count) failure(s); this bundle must not be shipped." -ForegroundColor Red
    exit 1
}
Write-Host "check-bundle: all checks passed."
exit 0
