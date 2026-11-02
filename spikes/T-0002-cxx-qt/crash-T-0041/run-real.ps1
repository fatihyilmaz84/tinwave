# Stress the REAL spike app (Rust-backed provider, T2_MODE=stress) N times; counts non-zero exits / hangs.
# usage: .\run-real.ps1 -Count 200 -Parallel 4 -Platform offscreen
param([int]$Count = 100, [int]$Parallel = 4, [int]$ExitMin = 150, [int]$ExitMax = 900, [string]$Platform = "offscreen",
      [string]$Exe = "$PSScriptRoot\..\target\release\t0002-cxx-qt.exe", [int]$TimeoutSec = 60, [string]$Out = "")
$exe = (Resolve-Path $Exe).Path
$env:PATH = "C:\Qt\6.12.0\msvc2022_64\bin;C:\Windows\System32;C:\Windows"
$env:QT_PLUGIN_PATH = "C:\Qt\6.12.0\msvc2022_64\plugins"
$env:QML_IMPORT_PATH = "C:\Qt\6.12.0\msvc2022_64\qml"
$env:QT_QPA_PLATFORM = $Platform
if ($Platform -eq "offscreen") { $env:QT_QUICK_BACKEND = "software" }
$env:T2_MODE = "stress"; $env:T2_STYLE = "FluentWinUI3"
$rnd = [Random]::new(); $ok = 0; $bad = 0; $log = @(); $i = 0
$crashDir = Join-Path $PSScriptRoot "results\real-app"; New-Item -ItemType Directory -Force $crashDir | Out-Null
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($i -lt $Count) {
    $batch = @()
    for ($k = 0; $k -lt $Parallel -and $i -lt $Count; $k++, $i++) {
        $env:T2_EXIT_MS = "$($rnd.Next($ExitMin, $ExitMax))"
        $o = [IO.Path]::GetTempFileName()
        $p = Start-Process -FilePath $exe -PassThru -NoNewWindow -RedirectStandardOutput $o -RedirectStandardError "$o.err"
        $null = $p.Handle
        $batch += [pscustomobject]@{ P = $p; Out = $o; Exit = $env:T2_EXIT_MS; N = $i }
    }
    foreach ($x in $batch) {
        $timedOut = $false
        if (-not $x.P.WaitForExit($TimeoutSec * 1000)) { $x.P.Kill(); $x.P.WaitForExit(); $timedOut = $true }
        $code = $x.P.ExitCode
        $so = Get-Content $x.Out -Raw -ErrorAction SilentlyContinue; $se = Get-Content "$($x.Out).err" -Raw -ErrorAction SilentlyContinue
        if ($code -eq 0 -and -not $timedOut -and $so -match "engine_load_returned") { $ok++ }
        else { $bad++; $f = Join-Path $crashDir ("fail-{0}.txt" -f $x.N); "exit=$code timeout=$timedOut exitms=$($x.Exit)`n$so`n$se" | Set-Content $f; $log += "run $($x.N): exit=$code timeout=$timedOut" }
        Remove-Item $x.Out, "$($x.Out).err" -ErrorAction SilentlyContinue
    }
}
$line = "REAL-APP platform=$Platform runs=$Count ok=$ok failed=$bad elapsed_s=$([int]$sw.Elapsed.TotalSeconds) exe=$(Split-Path $exe -Leaf)"
$line; $log
if ($Out) { Add-Content $Out $line; if ($log) { Add-Content $Out $log } }
