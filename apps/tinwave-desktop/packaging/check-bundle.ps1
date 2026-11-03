# Compliance and self-containment check of a deployed Tinwave bundle (ADR 0008 decision 3; ticket T-0042).
#
#   .\check-bundle.ps1 -Bundle ..\..\..\target\package\tinwave-desktop-windows-x64
#
# Exit code 0 = every check passed. Exit code 1 = at least one FAIL line was printed; package.ps1 stops on it.
# The checks (each prints PASS or FAIL):
#   shared-qt     the Qt kit is a shared build and Cargo links no Qt object files (LGPL-3.0: dynamic linking only)
#   static-qt     tinwave-desktop.exe imports Qt6Core.dll, i.e. Qt is dynamic, not linked into the exe
#   gpl-only      no shipped Qt DLL is GPL-only (per the kit's SPDX SBOM); no Qt test modules; no source asks for a GPL-only
#                 module (the test-utils are static libraries in the kit, so they would be linked INTO the exe, not shipped
#                 as a DLL); the exe carries no trace of one
#   only-app-exe  the one executable is tinwave-desktop.exe (never image-stress.exe, Qt tools, pdb, lib files)
#   closure       every import of every PE file resolves inside the bundle or to a Windows system DLL;
#                 the MSVC runtime must be in the bundle (app-local), it is never assumed to be installed
#   legal         THIRD-PARTY-NOTICES.txt, the LGPL/GPL texts and the Qt source offer are in the bundle
param(
    [Parameter(Mandatory)][string]$Bundle,
    [string]$QtDir = "",
    # Source folder scanned for a request to link a GPL-only Qt module (build.rs, Cargo.toml, Rust, C++ and QML sources).
    [string]$SourceDir = ""
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

$meta = (& cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $script:RepoRoot 'Cargo.toml') | Out-String) | ConvertFrom-Json
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
if (-not $SourceDir) { $SourceDir = Join-Path $script:RepoRoot 'apps\tinwave-desktop' }
$SourceDir = (Resolve-Path $SourceDir).Path.TrimEnd('\')
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
$srcFiles = @(Get-ChildItem $SourceDir -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Extension -in '.rs', '.toml', '.cpp', '.h', '.hpp', '.qml', '.pro', '.cmake', '.txt' -and $_.FullName.Substring($SourceDir.Length) -notmatch '^\\(target|packaging)\\' })
foreach ($sf in $srcFiles) {
    $txt = [IO.File]::ReadAllText($sf.FullName)
    foreach ($tk in $tokens) {
        if ($txt.Contains($tk)) { $gplBad.Add("$($sf.FullName.Substring($SourceDir.Length).TrimStart('\')) mentions the GPL-only Qt module '$tk': it would be linked into the exe") }
    }
}
if (Test-Path $exe) {
    $bytes = [IO.File]::ReadAllBytes($exe)
    $latin = [Text.Encoding]::GetEncoding(28591).GetString($bytes)
    foreach ($tk in $tokens) {
        if ($latin.Contains($tk)) { $gplBad.Add("tinwave-desktop.exe contains the string '$tk': a GPL-only Qt module was linked in") }
    }
}
if ($gplBad.Count -eq 0) { Pass 'gpl-only' "$nQt Qt binaries checked against the kit SBOMs ($($gplModules.Count) GPL-only modules known, e.g. $($gplModules -join ', ')): none shipped or linked, no test modules" }
else { foreach ($b in $gplBad) { Fail 'gpl-only' $b } }

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
if ($legalOk) { Pass 'legal' 'notices, Qt source offer and LGPL-3.0 / GPL-3.0 texts are present' }

if ($failures.Count -gt 0) {
    Write-Host ""
    Write-Host "check-bundle: $($failures.Count) failure(s); this bundle must not be shipped." -ForegroundColor Red
    exit 1
}
Write-Host "check-bundle: all checks passed."
exit 0
