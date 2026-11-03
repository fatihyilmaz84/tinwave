# Tests of the packaging checks themselves: every compliance rule must FAIL on a bundle that breaks it.
#
#   .\package.ps1 -NoSmoke                                  # make a good bundle first
#   .\test-packaging.ps1 [-Bundle <good bundle>]            # default: target\package\tinwave-desktop-windows-x64
#
# Each case copies the good bundle to target\package\test\<case>, breaks one thing and runs the real check-bundle.ps1 (or
# smoke-test.ps1) in a child PowerShell. The expectation is the exit code (1) AND the named FAIL line. Exit code of this
# script: 0 when every case behaved, 1 otherwise. Nothing outside target\ is written; every process started is killed.
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

    # 1. the good bundle passes
    $r = Invoke-Script $check @('-Bundle', $Bundle, '-QtDir', $kit.Root)
    Assert-Case 'good bundle passes check-bundle (exit 0)' ($r.Code -eq 0) $r.Out

    # 2. a GPL-only Qt module DLL in the bundle (named like the Qt test-utils module)
    $d = New-Fixture 'gpl-dll'
    Copy-Item (Join-Path $d 'Qt6Svg.dll') (Join-Path $d 'Qt6QuickControlsTestUtilsPrivate.dll')
    Test-CheckFails 'GPL-only Qt module DLL fails the check' $d 'FAIL gpl-only .*TestUtils'

    # 3. the exe has a GPL-only Qt module linked in (static lib in the kit: only visible as a string in the exe)
    $d = New-Fixture 'gpl-linked'
    $marker = [Text.Encoding]::ASCII.GetBytes('Qt6QuickControlsTestUtils.lib')
    $fs = [IO.File]::Open((Join-Path $d 'tinwave-desktop.exe'), 'Append'); $fs.Write($marker, 0, $marker.Length); $fs.Close()
    Test-CheckFails 'GPL-only Qt module linked into the exe fails the check' $d 'FAIL gpl-only .*tinwave-desktop.exe contains'

    # 4. a build script asks for a GPL-only module
    $d = New-Fixture 'gpl-source'
    $src = Join-Path $work 'gpl-source-src'
    New-Item -ItemType Directory -Force $src | Out-Null
    Write-TextFile (Join-Path $src 'build.rs') "fn main() { cxx_qt_build::CxxQtBuilder::new().qt_module(`"QuickTestUtilsPrivate`").build(); }`n"
    Test-CheckFails 'build.rs requesting a GPL-only Qt module fails the check' $d 'FAIL gpl-only .*build.rs mentions' @('-SourceDir', $src)

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
    Assert-Case 'smoke test fails when Qt6Core.dll is missing, although Qt is on this shell PATH' ($r.Code -eq 1 -and $r.Out -match 'FAIL smoke') $r.Out

    # 11. notices: MPL-2.0 crates (Symphonia, via tinwave-playback) get the source statement and the MPL text
    $n = Join-Path $work 'notices-mpl.txt'
    $r = Invoke-Script (Join-Path $PSScriptRoot 'New-ThirdPartyNotices.ps1') @('-Bundle', $Bundle, '-Out', $n, '-Package', 'tinwave-playback', '-QtDir', $kit.Root)
    $t = if (Test-Path $n) { Read-TextFile $n } else { '' }
    Assert-Case 'notices for a package with Symphonia carry the MPL-2.0 statement and text' `
        ($r.Code -eq 0 -and $t -match 'MPL-2\.0 \(file-level copyleft\) applies to: .*symphonia' -and $t -match 'Mozilla Public License Version 2\.0') $r.Out

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
if ($failed.Count -gt 0) { exit 1 }
exit 0
