# Starts the deployed app with Qt removed from PATH and proves it reaches its window, then kills it.
#
#   .\smoke-test.ps1 -Bundle <deployed folder> [-TimeoutSec 30]
#
# A pass needs ALL of:
#   * the process stays alive and creates a visible top-level window titled "Tinwave" (Main.qml loaded, so the QML engine,
#     the FluentWinUI3 style and its Fusion dependency all resolved from the bundle)
#   * every Qt DLL the process loaded comes from the bundle folder, none from anywhere else on the machine
#   * qwindows.dll and the FluentWinUI3 style plugin were loaded
# The child gets PATH = Windows only, and none of the QT_* / QML_* variables that could point it at a Qt install; this
# script's own environment is restored afterwards. The window is briefly visible (about 2 s). The process is always killed.
# Exit code 0 = pass, 1 = fail.
param(
    [Parameter(Mandatory)][string]$Bundle,
    [int]$TimeoutSec = 30
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path $Bundle).Path.TrimEnd('\')
$exe = Join-Path $root 'tinwave-desktop.exe'
if (-not (Test-Path $exe)) { throw "No tinwave-desktop.exe in $root" }

# A missing DLL must make the child exit, not show the loader's modal "code execution cannot proceed" box (that would hang
# this script until the timeout). SetErrorMode is inherited by the child; nothing in Windows settings or the registry changes.
# The mode is process-wide, and when package.ps1 runs this script it is package.ps1's own process (an interactive shell would
# keep it): the previous mode is restored in the finally block below.
if (-not ('Tw.SmokeNative' -as [type])) {
    Add-Type -Namespace Tw -Name SmokeNative -MemberDefinition '[DllImport("kernel32.dll")] public static extern uint SetErrorMode(uint mode);'
}
$previousErrorMode = [Tw.SmokeNative]::SetErrorMode(0x0001 -bor 0x0002 -bor 0x8000)

$saved = @{}
foreach ($k in @('PATH', 'QMAKE', 'QT_PLUGIN_PATH', 'QML_IMPORT_PATH', 'QML2_IMPORT_PATH', 'QT_QPA_PLATFORM_PLUGIN_PATH', 'QT_QPA_PLATFORM', 'QT_QUICK_BACKEND', 'QT_QUICK_CONTROLS_STYLE', 'QT_DIR', 'QTDIR', 'QT_FORCE_STDERR_LOGGING')) {
    $saved[$k] = [Environment]::GetEnvironmentVariable($k, 'Process')
}
$errFile = [IO.Path]::GetTempFileName()
$outFile = [IO.Path]::GetTempFileName()
$p = $null
$failures = New-Object Collections.Generic.List[string]
try {
    foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $null, 'Process') }
    $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
    $env:QT_FORCE_STDERR_LOGGING = '1'
    $p = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru -RedirectStandardError $errFile -RedirectStandardOutput $outFile
    $null = $p.Handle   # keep the handle: ExitCode stays readable

    $sw = [Diagnostics.Stopwatch]::StartNew()
    $window = $false
    while ($sw.Elapsed.TotalSeconds -lt $TimeoutSec) {
        if ($p.HasExited) { break }
        $p.Refresh()
        if ($p.MainWindowHandle -ne [IntPtr]::Zero -and $p.MainWindowTitle -eq 'Tinwave') { $window = $true; break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $window) {
        if ($p.HasExited) { $failures.Add("process exited with code $($p.ExitCode) before showing a window") }
        else { $failures.Add("no window titled 'Tinwave' within $TimeoutSec s") }
    } else {
        $ms = [int]$sw.Elapsed.TotalMilliseconds
        Start-Sleep -Milliseconds 1500       # let late QML errors and a crash show up
        $p.Refresh()
        if ($p.HasExited) { $failures.Add("process exited with code $($p.ExitCode) after showing the window") }
        else {
            $mods = @($p.Modules | ForEach-Object { $_.FileName })
            $qt = @($mods | Where-Object { [IO.Path]::GetFileName($_) -match '^(Qt6|qwindows|qtquick|qt6)' -or $_ -match '\\Qt\\' })
            $outside = @($qt | Where-Object { -not $_.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase) })
            if ($outside.Count -gt 0) { $failures.Add("Qt files loaded from outside the bundle: $($outside -join '; ')") }
            foreach ($need in 'qwindows.dll', 'qtquickcontrols2fluentwinui3styleplugin.dll', 'Qt6Quick.dll', 'Qt6QuickControls2.dll') {
                if (-not ($mods | Where-Object { [IO.Path]::GetFileName($_) -ieq $need })) { $failures.Add("$need was not loaded") }
            }
            Write-Host ("window 'Tinwave' visible after {0} ms; {1} modules loaded, {2} of them Qt, all Qt files from the bundle: {3}" -f $ms, $mods.Count, $qt.Count, ($outside.Count -eq 0))
        }
    }
} finally {
    if ($p -ne $null -and -not $p.HasExited) { $p.Kill(); $null = $p.WaitForExit(10000) }
    $null = [Tw.SmokeNative]::SetErrorMode($previousErrorMode)
    foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k], 'Process') }
}
$stderr = Get-Content $errFile -Raw -ErrorAction SilentlyContinue
Remove-Item $errFile, $outFile -ErrorAction SilentlyContinue
if ($stderr -and $stderr.Trim()) {
    # Qt logs QML errors, missing plugins and import failures to stderr.
    if ($stderr -match '(?i)(module .* is not installed|could not load|cannot load|failed to|qml.*error|no such file|not found|Cannot find)') { $failures.Add("stderr reports a problem:`n$stderr") }
    else { Write-Host "stderr:`n$stderr" }
}
if ($failures.Count -gt 0) {
    foreach ($f in $failures) { Write-Host "FAIL smoke  $f" -ForegroundColor Red }
    exit 1
}
Write-Host "PASS smoke   tinwave-desktop.exe started with Qt off PATH, reached its window, was killed"
exit 0
