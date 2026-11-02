# Startup-to-first-frame runs. -Fresh copies the deployed folder to a brand-new directory per run (first exec of new files).
param([int]$N = 10, [switch]$Fresh, [string]$Mode = "list", [string]$Style = "FluentWinUI3", [string]$Src = "$PSScriptRoot\deploy-min", [string]$Backend = "")
$env:PATH = "C:\Windows\System32;C:\Windows"
$env:T2_MODE = $Mode; $env:T2_BENCH = "1"; $env:T2_STYLE = $Style; $env:T2_EXIT_AT_FIRST_FRAME = "1"
if ($Backend -eq "software") { $env:QT_QUICK_BACKEND = "software" }
for ($i = 0; $i -lt $N; $i++) {
    $dir = $Src
    if ($Fresh) { $dir = Join-Path $env:TEMP ("t2fresh_" + [guid]::NewGuid().ToString("N")); Copy-Item $Src $dir -Recurse }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $o = [IO.Path]::GetTempFileName()
    $p = Start-Process -FilePath "$dir\t0002-cxx-qt.exe" -WorkingDirectory $dir -PassThru -NoNewWindow -RedirectStandardOutput $o
    $null = $p.Handle
    if (-not $p.WaitForExit(60000)) { $p.Kill() }
    $ms = $sw.ElapsedMilliseconds
    $ff = (Get-Content $o | Select-String "first_frame") -replace "T2 first_frame ", ""
    "wall_ms=$ms $ff"
    Remove-Item $o
    if ($Fresh) { Remove-Item $dir -Recurse -Force }
}
