# Builds and packages tinwave-desktop for Windows x64 into a self-contained folder (ticket T-0042, ADR 0008 section 3.5).
#
#   $env:QMAKE = 'C:\Qt\6.12.0\msvc2022_64\bin\qmake.exe'
#   .\package.ps1                       # -> target\package\tinwave-desktop-windows-x64\
#   .\package.ps1 -SkipBuild -NoSmoke   # reuse target\release\tinwave-desktop.exe, no window
#
# Steps, in order; any failure stops the script with a non-zero exit code and no bundle should be shipped:
#   1  cargo build --release --frozen -p tinwave-desktop --bin tinwave-desktop   (only that binary; image-stress is dev-only;
#      --frozen: no network at package time)
#   2  copy tinwave-desktop.exe (and nothing else from target\release), windeployqt with the ADR 0008 flags, drop the
#      styles the app does not use
#   3  copy the MSVC runtime DLLs the bundle imports (app-local, from the Visual Studio redistributable folder)
#   4  legal files: THIRD-PARTY-NOTICES.txt (New-ThirdPartyNotices.ps1), Qt LGPL-3.0/GPL-3.0 texts, Qt source offer (one
#      source archive per Qt repository that the shipped Qt files come from)
#   5  check-bundle.ps1: no GPL-only Qt (SBOM + build-script link libs), no static Qt, every Qt file identical to the kit,
#      only the app exe, imports resolve, legal files and source offer complete
#   6  smoke-test.ps1: run the bundle with Qt removed from PATH, reach the window, kill it
# The environment is changed for this process only (QMAKE, PATH) and restored on exit.
# -OutDir is wiped and rebuilt only when it carries this script's marker file (.tinwave-package) or is empty.
# -Release: D-0007 (copyright holder, source-offer contact) still pending = the check fails, see README.
param(
    [string]$OutDir = "",
    [string]$QtDir = "",
    [string]$CrtDir = "",
    [switch]$SkipBuild,
    [switch]$NoSmoke,
    [switch]$Release
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')

$repo = $script:RepoRoot
if (-not $OutDir) { $OutDir = Join-Path $repo 'target\package\tinwave-desktop-windows-x64' }
$OutDir = [IO.Path]::GetFullPath($OutDir)
$kit = Resolve-QtKit $QtDir
$savedQmake = $env:QMAKE; $savedPath = $env:PATH
$exitCode = 0
$markerName = '.tinwave-package'

function Invoke-Native([string]$What, [scriptblock]$Cmd) {
    # cargo and windeployqt write progress to stderr: that is not an error, the exit code is.
    $ErrorActionPreference = 'Continue'
    & $Cmd
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit code $LASTEXITCODE)" }
}

try {
    $env:QMAKE = Join-Path $kit.Bin 'qmake.exe'
    $env:PATH = "$($kit.Bin);$env:PATH"      # Qt DLLs on PATH for the build and for windeployqt only; the smoke test removes them

    # --- 1 build ---------------------------------------------------------------------------------------------------
    $exeSrc = Join-Path $repo 'target\release\tinwave-desktop.exe'
    if (-not $SkipBuild) {
        Write-Host "== cargo build --release --frozen -p tinwave-desktop --bin tinwave-desktop"
        Push-Location $repo
        try { Invoke-Native 'cargo build' { cargo build --release --frozen -p tinwave-desktop --bin tinwave-desktop } } finally { Pop-Location }
    }
    if (-not (Test-Path $exeSrc)) { throw "Missing $exeSrc (run without -SkipBuild)." }

    # --- 2 deploy --------------------------------------------------------------------------------------------------
    if (Test-Path $OutDir) {
        $existing = @(Get-ChildItem $OutDir -Force)
        if ($existing.Count -gt 0 -and -not (Test-Path (Join-Path $OutDir $markerName))) {
            throw "$OutDir exists, is not empty and does not carry $markerName (the marker this script writes into every folder it creates); refusing to delete it. Choose another -OutDir or remove the folder yourself."
        }
        Remove-Item $OutDir -Recurse -Force
    }
    New-Item -ItemType Directory -Force $OutDir | Out-Null
    Write-TextFile (Join-Path $OutDir $markerName) "Created by apps/tinwave-desktop/packaging/package.ps1. The script deletes and rebuilds this folder only while this file is here.`n"
    Copy-Item $exeSrc (Join-Path $OutDir 'tinwave-desktop.exe')

    Write-Host "== windeployqt ($($kit.Version))"
    # Flags from ADR 0008 section 3.5, plus --no-compiler-runtime (step 3 copies the DLLs; windeployqt would copy the
    # vc_redist installer instead). Build outputs of QML go through --qmldir so only imported modules are deployed.
    Invoke-Native 'windeployqt' {
        & (Join-Path $kit.Bin 'windeployqt.exe') --release --qmldir (Join-Path $repo 'apps\tinwave-desktop\qml') `
            --no-translations --no-opengl-sw --no-system-d3d-compiler --no-system-dxc-compiler --no-compiler-runtime `
            --skip-plugin-types qmltooling,generic `
            --no-quickcontrols2material --no-quickcontrols2universal --no-quickcontrols2imagine `
            --no-quickcontrols2windows --no-quickcontrols2native `
            (Join-Path $OutDir 'tinwave-desktop.exe') | Out-Null
    }

    # FluentWinUI3 imports QtQuick.Controls.Fusion (its qmldir), so Fusion stays. The other styles' QML folders and style
    # implementation DLLs were still copied by the QML scan (Main.qml imports QtQuick.Controls, which lists them all).
    $keep = @('Basic', 'FluentWinUI3', 'Fusion', 'impl')
    $ctl = Join-Path $OutDir 'qml\QtQuick\Controls'
    Get-ChildItem $ctl -Directory | Where-Object { $keep -notcontains $_.Name } | Remove-Item -Recurse -Force
    foreach ($style in 'Material', 'Universal', 'Imagine', 'Windows', 'Native') {
        Get-ChildItem $OutDir -File -Filter "Qt6QuickControls2${style}*.dll" | Remove-Item -Force
    }
    Get-ChildItem $OutDir -File -Filter 'Qt6QuickControls2*.dll' | ForEach-Object { Write-Host "   kept $($_.Name)" }

    # --- 3 MSVC runtime, app-local ---------------------------------------------------------------------------------
    $crtPattern = '^(vcruntime|msvcp|concrt|vccorlib|vcomp)\d*(_\w+)?\.dll$'
    function Get-NeededCrt {
        $need = @{}
        foreach ($f in (Get-ChildItem $OutDir -Recurse -File | Where-Object { $_.Name -match '\.(dll|exe)$' })) {
            foreach ($i in (Get-PeImports $f.FullName).Imports) { if ($i.ToLowerInvariant() -match $crtPattern) { $need[$i.ToLowerInvariant()] = $true } }
        }
        @($need.Keys)
    }
    $crtNeeded = @(Get-NeededCrt)
    if ($crtNeeded.Count -gt 0) {
        if (-not $CrtDir) {
            if ($env:VCToolsRedistDir) { $CrtDir = Get-ChildItem (Join-Path $env:VCToolsRedistDir 'x64') -Directory -Filter 'Microsoft.VC*.CRT' | Select-Object -First 1 -ExpandProperty FullName }
            if (-not $CrtDir) {
                $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
                if (Test-Path $vswhere) {
                    $inst = (& $vswhere -latest -products * -property installationPath | Select-Object -First 1)
                    if ($inst) {
                        $ver = Get-ChildItem (Join-Path $inst 'VC\Redist\MSVC') -Directory -ErrorAction SilentlyContinue |
                            Where-Object { Test-Path (Join-Path $_.FullName 'x64') } | Sort-Object Name -Descending | Select-Object -First 1
                        if ($ver) { $CrtDir = Get-ChildItem (Join-Path $ver.FullName 'x64') -Directory -Filter 'Microsoft.VC*.CRT' | Select-Object -First 1 -ExpandProperty FullName }
                    }
                }
            }
        }
        if (-not $CrtDir -or -not (Test-Path $CrtDir)) {
            throw "The bundle imports the MSVC runtime ($($crtNeeded -join ', ')) but no Visual Studio redistributable folder was found. Install the MSVC build tools or pass -CrtDir '<VS>\VC\Redist\MSVC\<ver>\x64\Microsoft.VC*.CRT'."
        }
        Write-Host "== MSVC runtime from $CrtDir"
        $done = @{}
        while ($true) {
            $todo = @(Get-NeededCrt | Where-Object { -not $done.ContainsKey($_) })
            if ($todo.Count -eq 0) { break }
            foreach ($n in $todo) {
                $src = Get-ChildItem $CrtDir -File | Where-Object { $_.Name -ieq $n } | Select-Object -First 1
                if (-not $src) { throw "$n is imported by the bundle but is not in $CrtDir" }
                Copy-Item $src.FullName (Join-Path $OutDir $src.Name)
                Write-Host "   copied $($src.Name)"
                $done[$n] = $true
            }
        }
    }

    # --- 4 legal files ---------------------------------------------------------------------------------------------
    Write-Host "== legal files"
    $lic = Join-Path $OutDir 'licenses'
    New-Item -ItemType Directory -Force $lic | Out-Null
    $qtLicense = Join-Path $kit.QtRoot 'Licenses\LICENSE'
    if (-not (Test-Path $qtLicense)) { throw "Qt license file not found: $qtLicense" }
    Copy-Item $qtLicense (Join-Path $lic 'Qt-LGPL-3.0-and-GPL-3.0.txt')
    # Tinwave's own license: the GPL-3.0 text in the repository root (ADR 0009). Its absence is an error, never skipped.
    $rootLicense = Join-Path $repo 'LICENSE'
    if (-not (Test-Path $rootLicense)) { throw "Tinwave's license file not found: $rootLicense (ADR 0009: GPL-3.0-only)" }
    Copy-Item $rootLicense (Join-Path $lic 'Tinwave-LICENSE-GPL-3.0.txt')
    $identity = Get-ReleaseIdentity
    $repos = @(Get-ShippedQtRepos (Get-BundleFiles $OutDir) (Read-QtSbom $kit.Root))
    if ($repos.Count -eq 0) { throw "No Qt repository found for the Qt files in $OutDir (SBOM lookup)." }
    Write-Host ("   Qt source archives in the offer: " + (($repos | ForEach-Object { $_.Repo }) -join ', '))
    Write-TextFile (Join-Path $OutDir 'SOURCE-OFFER-Qt.txt') (New-SourceOffer (Join-Path $PSScriptRoot 'templates\SOURCE-OFFER-Qt.txt') $kit.Version $repos $identity.ContactText)
    & (Join-Path $PSScriptRoot 'New-ThirdPartyNotices.ps1') -Bundle $OutDir -Out (Join-Path $OutDir 'THIRD-PARTY-NOTICES.txt') -QtDir $kit.Root
    if (-not $?) { throw "New-ThirdPartyNotices.ps1 failed" }
    $committed = Join-Path $PSScriptRoot 'THIRD-PARTY-NOTICES.txt'
    if (-not (Test-Path $committed) -or (Get-FileHash $committed).Hash -ne (Get-FileHash (Join-Path $OutDir 'THIRD-PARTY-NOTICES.txt')).Hash) {
        Write-Warning "packaging\THIRD-PARTY-NOTICES.txt differs from what this build generated (dependencies or Qt changed). Refresh it: copy $OutDir\THIRD-PARTY-NOTICES.txt over it and commit."
    }

    # --- 5 check ---------------------------------------------------------------------------------------------------
    Write-Host "== check-bundle"
    & (Join-Path $PSScriptRoot 'check-bundle.ps1') -Bundle $OutDir -QtDir $kit.Root -Release:$Release
    if ($LASTEXITCODE -ne 0) { throw "check-bundle failed: do not ship $OutDir" }

    # --- 6 smoke ---------------------------------------------------------------------------------------------------
    if (-not $NoSmoke) {
        Write-Host "== smoke test (Qt removed from PATH)"
        & (Join-Path $PSScriptRoot 'smoke-test.ps1') -Bundle $OutDir
        if ($LASTEXITCODE -ne 0) { throw "smoke test failed: do not ship $OutDir" }
    } else {
        Write-Host "== smoke test skipped (-NoSmoke)"
    }

    $s = Get-BundleSummary $OutDir
    Write-Host ""
    Write-Host ("Bundle: {0}" -f $OutDir)
    Write-Host ("        {0} files, {1:N0} bytes ({2} MiB)" -f $s.Files, $s.Bytes, $s.MiB)
} catch {
    Write-Host "package.ps1 FAILED: $($_.Exception.Message)" -ForegroundColor Red
    $exitCode = 1
} finally {
    $env:QMAKE = $savedQmake; $env:PATH = $savedPath
}
exit $exitCode
