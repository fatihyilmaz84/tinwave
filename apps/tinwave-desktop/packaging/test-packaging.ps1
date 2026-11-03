# Tests of the packaging checks themselves: every compliance rule must FAIL on a bundle that breaks it.
#
#   .\package.ps1 -NoSmoke                                  # make a good bundle first
#   .\test-packaging.ps1 [-Bundle <good bundle>]            # default: target\package\tinwave-desktop-windows-x64
#
# Each case copies the good bundle to target\package\test\<case>, breaks one thing and runs the real check-bundle.ps1 (or
# smoke-test.ps1) in a child PowerShell. The expectation is the exit code (1) AND the named FAIL line. Exit code of this
# script: 0 when every case behaved, 1 otherwise. Nothing outside target\ is written (the fixtures are removed after a clean run); every process started is killed.
param(
    [string]$Bundle = "",
    [string]$QtDir = ""
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')

$repo = $script:RepoRoot
if (-not $Bundle) { $Bundle = Join-Path $repo 'target\package\tinwave-desktop-windows-x64' }
$Bundle = (Resolve-Path $Bundle).Path
$kit = Resolve-QtKit $QtDir
$work = Join-Path $repo 'target\package\test'
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Force $work | Out-Null
$check = Join-Path $PSScriptRoot 'check-bundle.ps1'
$smoke = Join-Path $PSScriptRoot 'smoke-test.ps1'
$results = New-Object Collections.Generic.List[object]

function New-Fixture([string]$name) {
    $d = Join-Path $work $name
    Copy-Item $Bundle $d -Recurse
    $d
}
function Invoke-Script([string]$script, [string[]]$arguments) {
    $ErrorActionPreference = 'Continue'   # native stderr must not throw under -Stop
    $out = & powershell.exe -NoProfile -File $script @arguments 2>&1 | ForEach-Object { "$_" }
    [pscustomobject]@{ Code = $LASTEXITCODE; Out = ($out -join "`n") }
}
function Assert-Case([string]$name, [bool]$ok, [string]$detail) {
    $results.Add([pscustomobject]@{ Name = $name; Ok = $ok })
    if ($ok) { Write-Host "ok   $name" } else { Write-Host "FAIL $name`n$detail" -ForegroundColor Red }
}
function Test-CheckFails([string]$name, [string]$bundleDir, [string]$expectLine, [string[]]$extra = @()) {
    $r = Invoke-Script $check (@('-Bundle', $bundleDir, '-QtDir', $kit.Root) + $extra)
    $ok = ($r.Code -eq 1) -and ($r.Out -match $expectLine)
    Assert-Case $name $ok "expected exit 1 and /$expectLine/, got exit $($r.Code):`n$($r.Out)"
}

try {
    # 0. the license-expression classifier
    $cases = @(
        @('LicenseRef-Qt-Commercial OR LGPL-3.0-only OR GPL-2.0-only OR GPL-3.0-only', $false),
        @('LicenseRef-Qt-Commercial OR GPL-3.0-only', $true),
        @('GPL-3.0-only', $true),
        @('LicenseRef-Qt-Commercial OR GPL-3.0-only WITH Qt-GPL-exception-1.0', $true),
        @('LicenseRef-Qt-Commercial OR BSD-3-Clause', $false),
        @('NOASSERTION', $false)
    )
    $bad = @($cases | Where-Object { (Test-GplOnly $_[0]) -ne $_[1] } | ForEach-Object { $_[0] })
    Assert-Case 'Test-GplOnly classifies SPDX expressions' ($bad.Count -eq 0) ($bad -join "`n")

    # 0b. plugin names map to the SBOM exactly: qwindows.dll is QWindowsIntegrationPlugin only (not also the Direct2D plugin),
    #     and a stem that is merely a prefix of a package name matches nothing
    $sbom = Read-QtSbom $kit.Root
    $qw = @(Find-QtSbomPackages 'C:\x\platforms\qwindows.dll' $sbom | ForEach-Object { $_.Name })
    $qx = @(Find-QtSbomPackages 'C:\x\platforms\qwindowsx.dll' $sbom)
    $qp = @(Find-QtSbomPackages 'C:\x\platforms\qwin.dll' $sbom)
    Assert-Case 'plugin name match is exact (qwindows.dll is not also the Direct2D plugin; prefixes match nothing)' `
        ($qw.Count -eq 1 -and $qw[0] -eq 'QWindowsIntegrationPlugin' -and $qx.Count -eq 0 -and $qp.Count -eq 0) ($qw -join ',')

    # 1. the good bundle passes
    $r = Invoke-Script $check @('-Bundle', $Bundle, '-QtDir', $kit.Root)
    Assert-Case 'good bundle passes check-bundle (exit 0)' ($r.Code -eq 0) $r.Out
    Assert-Case 'good bundle: D-0007 is reported as pending, not as a failure' ($r.Code -eq 0 -and $r.Out -match 'PENDING d0007\s+D-0007 pending') $r.Out
    Assert-Case 'good bundle: Qt files byte-identical to the kit, link-libs and source-offer pass' `
        ($r.Out -match 'PASS qt-unmodified' -and $r.Out -match 'PASS link-libs' -and $r.Out -match 'PASS source-offer') $r.Out

    # 1b. D-0007: -Release turns the pending decision into a failure
    Test-CheckFails '-Release fails while D-0007 is pending' $Bundle 'FAIL d0007 .*D-0007 pending' @('-Release')

    # 1c. D-0007 decided but the bundle was generated before: stale marker
    $idFile = Join-Path $work 'identity-decided.json'
    Write-TextFile $idFile '{ "decision": "D-0007", "copyrightHolder": "Test Holder", "offerContact": "offers@example.invalid" }'
    Test-CheckFails 'a filled-in release identity with a bundle that does not name it fails (stale)' $Bundle 'FAIL d0007' @('-Identity', $idFile)

    # 2. a GPL-only Qt module DLL in the bundle (named like the Qt test-utils module)
    $d = New-Fixture 'gpl-dll'
    Copy-Item (Join-Path $d 'Qt6Svg.dll') (Join-Path $d 'Qt6QuickControlsTestUtilsPrivate.dll')
    Test-CheckFails 'GPL-only Qt module DLL fails the check' $d 'FAIL gpl-only .*TestUtils'

    # 3. the exe has a GPL-only Qt module linked in (static lib in the kit: only visible as a string in the exe)
    $d = New-Fixture 'gpl-linked'
    $marker = [Text.Encoding]::ASCII.GetBytes('Qt6QuickControlsTestUtils.lib')
    $fs = [IO.File]::Open((Join-Path $d 'tinwave-desktop.exe'), 'Append'); $fs.Write($marker, 0, $marker.Length); $fs.Close()
    Test-CheckFails 'GPL-only Qt module linked into the exe fails the check' $d 'FAIL gpl-only .*tinwave-desktop.exe contains'

    # 4. a build script asks the linker for a GPL-only Qt module: only visible in the rustc-link-lib lines of the build outputs
    #    (the exe carries no string of it, the bundle no DLL)
    $d = New-Fixture 'gpl-linklib'
    $bd = Join-Path $work 'gpl-linklib-build'
    New-Item -ItemType Directory -Force (Join-Path $bd 'fake-crate-123') | Out-Null
    Write-TextFile (Join-Path $bd 'fake-crate-123\output') "cargo::rustc-link-lib=Qt6Core`ncargo::rustc-link-lib=Qt6QuickTestUtils`n"
    Test-CheckFails 'GPL-only Qt library requested via rustc-link-lib fails the check' $d 'FAIL link-libs .*Qt6QuickTestUtils .*GPL-only' @('-BuildDir', $bd)
    $bd2 = Join-Path $work 'static-linklib-build'
    New-Item -ItemType Directory -Force (Join-Path $bd2 'fake-crate-123') | Out-Null
    Write-TextFile (Join-Path $bd2 'fake-crate-123\output') "cargo:rustc-link-lib=static=Qt6Core`n"
    Test-CheckFails 'Qt linked statically via rustc-link-lib fails the check' $d 'FAIL link-libs .*statically' @('-BuildDir', $bd2)
    $bd3 = Join-Path $work 'empty-build'
    New-Item -ItemType Directory -Force $bd3 | Out-Null
    Test-CheckFails 'no build-script outputs: the link libraries cannot be verified, check fails' $d 'FAIL link-libs .*no rustc-link-lib' @('-BuildDir', $bd3)

    # 4b. a Qt file was changed after windeployqt (one byte appended), or a foreign file sits among the Qt files
    $d = New-Fixture 'qt-modified'
    $fs = [IO.File]::Open((Join-Path $d 'Qt6Svg.dll'), 'Append'); $fs.WriteByte(0); $fs.Close()
    Test-CheckFails 'a modified Qt DLL fails the unmodified-Qt check' $d 'FAIL qt-unmodified .*Qt6Svg.dll'
    $d = New-Fixture 'qt-extra'
    Write-TextFile (Join-Path $d 'qml\QtQuick\extra.qml') "import QtQuick`nItem {}`n"
    Test-CheckFails 'a file that is neither the kits nor ours fails the unmodified-Qt check' $d 'FAIL qt-unmodified .*extra.qml'

    # 4c. the source offer misses a Qt repository that a shipped module comes from
    $d = New-Fixture 'offer-no-svg'
    $op = Join-Path $d 'SOURCE-OFFER-Qt.txt'
    $keep = @((Read-TextFile $op) -split "`n" | Where-Object { $_ -notmatch 'qtsvg-everywhere' }) -join "`n"
    Write-TextFile $op $keep
    Test-CheckFails 'a Qt repository missing from the source offer fails the check' $d 'FAIL source-offer .*qtsvg'
    $d = New-Fixture 'offer-pruned-link'
    $op = Join-Path $d 'SOURCE-OFFER-Qt.txt'
    Write-TextFile $op ((Read-TextFile $op).Replace('download.qt.io/archive/qt/', 'download.qt.io/official_releases/qt/'))
    Test-CheckFails 'a source offer pointing at the pruned official_releases fails the check' $d 'FAIL source-offer'

    # 4d. acknowledgement sentences
    $d = New-Fixture 'no-ack'
    $np = Join-Path $d 'THIRD-PARTY-NOTICES.txt'
    $nt = (Read-TextFile $np) -replace 'Portions of this software are copyright \(c\) \d{4} The FreeType Project[^\n]*', ''
    Write-TextFile $np $nt
    Test-CheckFails 'missing FreeType acknowledgement sentence fails the check' $d 'FAIL legal .*FTL acknowledgement'
    $d = New-Fixture 'no-ijg'
    $np = Join-Path $d 'THIRD-PARTY-NOTICES.txt'
    Write-TextFile $np ((Read-TextFile $np).Replace('This software is based in part on the work of the Independent JPEG Group.', ''))
    Test-CheckFails 'missing IJG acknowledgement sentence fails the check' $d 'FAIL legal .*Independent JPEG Group'

    # 5. static Qt: an exe that does not import Qt6*.dll (cmd.exe stands in for an exe with Qt linked in)
    $d = New-Fixture 'static-qt'
    Copy-Item (Join-Path $env:SystemRoot 'System32\cmd.exe') (Join-Path $d 'tinwave-desktop.exe') -Force
    Test-CheckFails 'exe without dynamic Qt imports (static Qt) fails the check' $d 'FAIL static-qt'

    # 6. a dev binary next to the app
    $d = New-Fixture 'stray-exe'
    Copy-Item (Join-Path $d 'tinwave-desktop.exe') (Join-Path $d 'image-stress.exe')
    Test-CheckFails 'image-stress.exe in the bundle fails the check' $d 'FAIL only-app-exe .*image-stress.exe'

    # 7. MSVC runtime not app-local
    $d = New-Fixture 'no-crt'
    Remove-Item (Join-Path $d 'vcruntime140.dll')
    Test-CheckFails 'missing app-local MSVC runtime fails the check' $d 'FAIL closure .*vcruntime140'

    # 8. a Qt DLL missing
    $d = New-Fixture 'no-core'
    Remove-Item (Join-Path $d 'Qt6Core.dll')
    Test-CheckFails 'missing Qt6Core.dll fails the check' $d 'FAIL closure .*Qt6Core.dll'

    # 9. legal file missing
    $d = New-Fixture 'no-offer'
    Remove-Item (Join-Path $d 'SOURCE-OFFER-Qt.txt')
    Test-CheckFails 'missing Qt source offer fails the check' $d 'FAIL legal .*SOURCE-OFFER-Qt.txt'

    # 10. the smoke test is sensitive to a missing Qt DLL even when THIS shell has Qt on PATH (the child must not see it)
    $d = New-Fixture 'smoke-no-core'
    Remove-Item (Join-Path $d 'Qt6Core.dll')
    $savedPath = $env:PATH
    try {
        $env:PATH = "$($kit.Bin);$env:PATH"
        $r = Invoke-Script $smoke @('-Bundle', $d, '-TimeoutSec', '10')
    } finally { $env:PATH = $savedPath }
    # -1073741515 = 0xC0000135 STATUS_DLL_NOT_FOUND: the failure must be for the missing DLL, not for some other reason
    Assert-Case 'smoke test fails with STATUS_DLL_NOT_FOUND (-1073741515) when Qt6Core.dll is missing, although Qt is on this shell PATH' `
        ($r.Code -eq 1 -and $r.Out -match 'FAIL smoke\s+process exited with code -1073741515 before showing a window') $r.Out

    # 11. notices: MPL-2.0 crates (Symphonia, via tinwave-playback) get the source statement and the MPL text
    $n = Join-Path $work 'notices-mpl.txt'
    $r = Invoke-Script (Join-Path $PSScriptRoot 'New-ThirdPartyNotices.ps1') @('-Bundle', $Bundle, '-Out', $n, '-Package', 'tinwave-playback', '-QtDir', $kit.Root)
    $t = if (Test-Path $n) { Read-TextFile $n } else { '' }
    Assert-Case 'notices for a package with Symphonia carry the MPL-2.0 statement and text' `
        ($r.Code -eq 0 -and $t -match 'MPL-2\.0 \(file-level copyleft\) applies to: .*symphonia' -and $t -match 'Mozilla Public License Version 2\.0') $r.Out

    # 11b. notices for the desktop app: cxx-qt's MIT holder is KDAB (SPDX-FileCopyrightText), the Rust std crates that the
    #      linker took are listed from the link, and the FreeType / IJG acknowledgements are there
    $n2 = Join-Path $work 'notices-desktop.txt'
    $r = Invoke-Script (Join-Path $PSScriptRoot 'New-ThirdPartyNotices.ps1') @('-Bundle', $Bundle, '-Out', $n2, '-QtDir', $kit.Root)
    $t = if (Test-Path $n2) { Read-TextFile $n2 } else { '' }
    Assert-Case 'desktop notices: KDAB holder for cxx-qt, std crates from the link, FreeType and IJG sentences, D-0007 marker' `
        ($r.Code -eq 0 -and $t -match 'Copyright \(c\) 20\d\d(-20\d\d)? Kl\S+lvdalens Datakonsult AB, a KDAB Group company' -and $t -notmatch 'the cxx-qt authors' -and `
         $t -match '(?m)^\d+\. Rust standard library$' -and $t -match 'hashbrown 0\.\d+\.\d+ - ' -and $t -match 'Linked crates of the standard library source tree: .*\bstd\b' -and `
         $t -match 'Portions of this software are copyright \(c\) \d{4} The FreeType Project \(www\.freetype\.org\)\. All rights reserved\.' -and `
         $t -match 'This software is based in part on the work of the Independent JPEG Group\.' -and $t -match '\[D-0007 PENDING: copyright holder not decided\]') ($r.Out + "`n" + $t.Substring(0, [Math]::Min(300, $t.Length)))

    # 11a. how the D-0007 switch flips: with the identity filled in, regenerated notices and offer carry the values, no
    #      marker is left and the check no longer says "stale"; only the root LICENSE files keep it pending (-Release fails)
    $d = New-Fixture 'decided'
    $rid = Get-ReleaseIdentity $idFile
    $r1 = Invoke-Script (Join-Path $PSScriptRoot 'New-ThirdPartyNotices.ps1') @('-Bundle', $d, '-Out', (Join-Path $d 'THIRD-PARTY-NOTICES.txt'), '-QtDir', $kit.Root, '-IdentityFile', $idFile)
    $repos = @(Get-ShippedQtRepos (Get-BundleFiles $d) (Read-QtSbom $kit.Root))
    Write-TextFile (Join-Path $d 'SOURCE-OFFER-Qt.txt') (New-SourceOffer (Join-Path $PSScriptRoot 'templates\SOURCE-OFFER-Qt.txt') $kit.Version $repos $rid.ContactText)
    $r2 = Invoke-Script $check @('-Bundle', $d, '-QtDir', $kit.Root, '-Identity', $idFile)
    $r3 = Invoke-Script $check @('-Bundle', $d, '-QtDir', $kit.Root, '-Identity', $idFile, '-Release')
    Assert-Case 'identity filled in: regenerated bundle passes (root LICENSE files still pending), -Release fails only on them' `
        ($r1.Code -eq 0 -and $r2.Code -eq 0 -and $r2.Out -notmatch 'FAIL d0007' -and $r2.Out -match 'PENDING d0007\s+D-0007 pending.*LICENSE-MIT' -and `
         $r2.Out -notmatch 'copyright holder of Tinwave' -and $r3.Code -eq 1 -and $r3.Out -match 'FAIL d0007 .*LICENSE-MIT') ($r1.Out + "`n" + $r2.Out + "`n" + $r3.Out)

    # 11c. notices: a pdb that names no std rlib of this toolchain is refused (the std list is derived from the link)
    $fakePdb = Join-Path $work 'fake.pdb'
    Write-TextFile $fakePdb 'not a pdb of this toolchain'
    $r = Invoke-Script (Join-Path $PSScriptRoot 'New-ThirdPartyNotices.ps1') @('-Bundle', $Bundle, '-Out', (Join-Path $work 'y.txt'), '-QtDir', $kit.Root, '-Pdb', $fakePdb)
    Assert-Case 'notices generator refuses a link list that names no std rlib' ($r.Code -ne 0 -and -not (Test-Path (Join-Path $work 'y.txt')) -and $r.Out -match 'names no std/core rlib') $r.Out

    # 11d. package.ps1 deletes an -OutDir only when it carries its marker file
    $foreign = Join-Path $work 'foreign-outdir'
    New-Item -ItemType Directory -Force $foreign | Out-Null
    Copy-Item (Join-Path $Bundle 'tinwave-desktop.exe') (Join-Path $foreign 'tinwave-desktop.exe')
    $r = Invoke-Script (Join-Path $PSScriptRoot 'package.ps1') @('-SkipBuild', '-NoSmoke', '-QtDir', $kit.Root, '-OutDir', $foreign)
    Assert-Case 'package.ps1 refuses to delete an -OutDir without its marker file (even with a tinwave-desktop.exe in it)' `
        ($r.Code -eq 1 -and (Test-Path (Join-Path $foreign 'tinwave-desktop.exe')) -and $r.Out -match 'does not carry \.tinwave-package') $r.Out
    Assert-Case 'the good bundle carries the marker file' (Test-Path (Join-Path $Bundle '.tinwave-package')) ''

    # 12. notices refuse a bundle without Qt
    $d = New-Fixture 'notices-no-qt'
    Get-ChildItem $d -Recurse -Filter '*.dll' | Remove-Item
    $r = Invoke-Script (Join-Path $PSScriptRoot 'New-ThirdPartyNotices.ps1') @('-Bundle', $d, '-Out', (Join-Path $work 'x.txt'), '-QtDir', $kit.Root)
    Assert-Case 'notices generator fails on a bundle without Qt DLLs' ($r.Code -ne 0 -and -not (Test-Path (Join-Path $work 'x.txt'))) $r.Out
} finally {
    Get-Process tinwave-desktop, image-stress -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$work*" } | Stop-Process -Force
}

$failed = @($results | Where-Object { -not $_.Ok })
Write-Host ""
Write-Host ("{0} cases, {1} failed" -f $results.Count, $failed.Count)
if ($failed.Count -gt 0) { Write-Host "Fixtures kept for inspection: $work"; exit 1 }
# The fixtures are full copies of the bundle (about 1 GB in all): remove them when everything behaved.
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
exit 0
