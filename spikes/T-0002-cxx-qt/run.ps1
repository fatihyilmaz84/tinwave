# Usage: .\run.ps1 [-Exe path] [-Mode empty|list] [-Style X] [-Forward 0|1] [-Out file]
param(
    [string]$Exe = "$PSScriptRoot\target\release\t0002-cxx-qt.exe",
    [string]$Mode = "list",
    [string]$Style = "FluentWinUI3",
    [string]$Forward = "1",
    [int]$TimeoutSec = 120,
    [string]$QtBin = "C:\Qt\6.12.0\msvc2022_64\bin"
)
$env:PATH = "$QtBin;" + $env:PATH
$env:T2_MODE = if ($Mode -eq "empty") { "empty" } else { "list" }
$env:T2_BENCH = "1"
$env:T2_STYLE = $Style
$env:T2_FORWARD = $Forward
$env:QT_FORCE_STDERR_LOGGING = "1"
$out = [IO.Path]::GetTempFileName(); $err = [IO.Path]::GetTempFileName()
$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath $Exe -PassThru -NoNewWindow -RedirectStandardOutput $out -RedirectStandardError $err
$null = $p.Handle
if (-not $p.WaitForExit($TimeoutSec * 1000)) { $p.Kill(); "TIMEOUT-KILLED" }
$sw.Stop()
"wall_ms_start_to_exit=$($sw.ElapsedMilliseconds) exit=$($p.ExitCode)"
Get-Content $out
Get-Content $err
Remove-Item $out, $err
