# Runs the reproducer N times (sequentially, or P at a time) and counts crashes / hangs / failures.
# usage: .\run-batch.ps1 -Build build-dbg -Provider olda -Count 50 [-Mode mix] [-ExitMin 100 -ExitMax 900] [-Parallel 1] [-Platform offscreen]
param(
    [string]$Build = "build-dbg",
    [string]$Provider = "queued",
    [int]$Count = 20,
    [string]$Mode = "mix",
    [int]$ExitMin = 100,
    [int]$ExitMax = 900,
    [int]$Parallel = 1,
    [int]$Threads = 2,
    [int]$Tick = 4,
    [int]$Drain = 0,
    [string]$Platform = "offscreen",
    [int]$TimeoutSec = 60,
    [string]$Out = ""
)
$root = $PSScriptRoot
$exe = Join-Path $root "$Build\repro.exe"
$asanDir = "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.50.35717\bin\Hostx64\x64"
$env:PATH = "C:\Qt\6.12.0\msvc2022_64\bin;$asanDir;C:\Windows\System32;C:\Windows"
$env:QT_PLUGIN_PATH = "C:\Qt\6.12.0\msvc2022_64\plugins"
$env:QML_IMPORT_PATH = "C:\Qt\6.12.0\msvc2022_64\qml"
$env:QT_QPA_PLATFORM = $Platform
$env:QT_QUICK_BACKEND = "software"
$env:QT_FORCE_STDERR_LOGGING = "1"
$env:TW_PROVIDER = $Provider; $env:TW_MODE = $Mode; $env:TW_THREADS = "$Threads"; $env:TW_TICK = "$Tick"; $env:TW_DRAIN_MS = "$Drain"
$crashDir = Join-Path $root "results\$Build-$Provider"
New-Item -ItemType Directory -Force $crashDir | Out-Null
$env:TW_CRASH_DIR = $crashDir
$env:ASAN_OPTIONS = "detect_leaks=0:abort_on_error=0:halt_on_error=1:symbolize=1"
$rnd = [Random]::new()
$ok = 0; $bad = 0; $log = @()
$sw = [Diagnostics.Stopwatch]::StartNew()
$i = 0
while ($i -lt $Count) {
    $batch = @()
    for ($k = 0; $k -lt $Parallel -and $i -lt $Count; $k++, $i++) {
        $env:TW_EXIT_MS = "$($rnd.Next($ExitMin, $ExitMax))"
        $o = [IO.Path]::GetTempFileName()
        $p = Start-Process -FilePath $exe -WorkingDirectory (Join-Path $root $Build) -PassThru -NoNewWindow -RedirectStandardOutput $o -RedirectStandardError "$o.err"
        $null = $p.Handle
        $batch += [pscustomobject]@{ P = $p; Out = $o; Exit = $env:TW_EXIT_MS; N = $i }
    }
    foreach ($x in $batch) {
        $timedOut = $false
        if (-not $x.P.WaitForExit($TimeoutSec * 1000)) { $x.P.Kill(); $x.P.WaitForExit(); $timedOut = $true }
        $code = $x.P.ExitCode
        $so = Get-Content $x.Out -Raw -ErrorAction SilentlyContinue
        $se = Get-Content "$($x.Out).err" -Raw -ErrorAction SilentlyContinue
        if ($code -eq 0 -and -not $timedOut -and $so -match "TW DONE" -and $se -match "ENGINE_DESTROYED") { $ok++ }
        else {
            $bad++
            $f = Join-Path $crashDir ("fail-{0}-{1}.txt" -f $x.N, $x.P.Id)
            "run=$($x.N) exit=$code timeout=$timedOut exitms=$($x.Exit)`n--- stdout`n$so`n--- stderr`n$se" | Set-Content $f
            $log += "run $($x.N): exit=$code timeout=$timedOut -> $f"
        }
        Remove-Item $x.Out, "$($x.Out).err" -ErrorAction SilentlyContinue
    }
}
$line = "RESULT build=$Build provider=$Provider mode=$Mode drain=$Drain threads=$Threads parallel=$Parallel platform=$Platform runs=$Count ok=$ok failed=$bad elapsed_s=$([int]$sw.Elapsed.TotalSeconds)"
$line
$log
if ($Out) { Add-Content $Out $line; if ($log) { Add-Content $Out $log } }
