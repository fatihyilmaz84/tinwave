# Teardown stress for the artwork image provider (ADR 0008, image provider lifetime, rule 6; testing.md "Bridge
# lifetime must be tested"). Runs the dev-only `image-stress` binary many times with a random quit time and counts
# crashes, exit hangs and counter violations. Re-run it on any change to a provider or to the UI owner.
#
#   cargo build --release -p tinwave-desktop --bin image-stress        # QMAKE must point at the Qt MSVC kit
#   .\run-stress.ps1 -Count 300                                        # Release, 1 pool thread, storm, 200-900 ms
#   .\run-stress.ps1 -Count 300 -Mode scroll -Pool 2 -Parallel 4 -ExitMin 50
#
# A run PASSES when the process exits with code 0 within -TimeoutSec, printed STRESS_DONE and
# STRESS_TEARDOWN_COMPLETE, and reported duplicate_finishes=0. Anything else is a failure:
#   hang  = still running after -TimeoutSec (killed by this script)
#   crash = exited with a non-zero code (an access violation shows as 0xC0000005 / -1073741819)
# Nothing in Windows settings is touched: crash dialogs are suppressed for this PowerShell process and its children
# with SetErrorMode only (no WER / IFEO registry keys).
param(
    [int]$Count = 300,
    [ValidateSet("release", "debug")][string]$Profile = "release",
    [ValidateSet("storm", "scroll")][string]$Mode = "storm",
    [int]$ExitMin = 200,
    [int]$ExitMax = 900,
    [int]$Pool = 1,
    [int]$DelayMs = 3,
    [int]$Parallel = 1,
    [int]$TimeoutSec = 60,
    [string]$Platform = "offscreen",
    [string]$QtBin = "",
    [string]$Out = ""
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..\..")).Path
$exe = Join-Path $root "target\$Profile\image-stress.exe"
if (-not (Test-Path $exe)) { throw "missing $exe - build it first: cargo build --$Profile -p tinwave-desktop --bin image-stress" }
if (-not $QtBin) {
    if ($env:QMAKE) { $QtBin = Split-Path $env:QMAKE } else { $QtBin = "C:\Qt\6.12.0\msvc2022_64\bin" }
}
$failDir = Join-Path $root "target\stress-failures"
New-Item -ItemType Directory -Force $failDir | Out-Null

Add-Type -Namespace Tw -Name Native -MemberDefinition '[DllImport("kernel32.dll")] public static extern uint SetErrorMode(uint mode);'
$null = [Tw.Native]::SetErrorMode(0x0001 -bor 0x0002 -bor 0x8000)   # no GP-fault box, no critical-error box, no open-file box

$env:PATH = "$QtBin;$env:PATH"
$env:QT_QPA_PLATFORM = $Platform
$env:QT_QUICK_BACKEND = "software"
$env:QT_FORCE_STDERR_LOGGING = "1"

$rnd = [Random]::new()
$ok = 0; $hangs = 0; $crashes = 0; $other = 0
$responses = 0; $finished = 0; $cancels = 0; $tasksRun = 0
$failures = @()
$sw = [Diagnostics.Stopwatch]::StartNew()
$i = 0
while ($i -lt $Count) {
    $batch = @()
    for ($k = 0; $k -lt $Parallel -and $i -lt $Count; $k++, $i++) {
        $exitMs = $rnd.Next($ExitMin, $ExitMax + 1)
        $o = [IO.Path]::GetTempFileName()
        $p = Start-Process -FilePath $exe -ArgumentList "--exit-ms=$exitMs", "--mode=$Mode", "--pool=$Pool", "--delay-ms=$DelayMs" `
            -PassThru -NoNewWindow -RedirectStandardOutput $o -RedirectStandardError "$o.err"
        $null = $p.Handle   # keep the handle so ExitCode is readable after exit
        $batch += [pscustomobject]@{ P = $p; Out = $o; ExitMs = $exitMs; N = $i }
    }
    foreach ($x in $batch) {
        $hung = $false
        if (-not $x.P.WaitForExit($TimeoutSec * 1000)) { $x.P.Kill(); $x.P.WaitForExit(); $hung = $true }
        $code = $x.P.ExitCode
        $so = Get-Content $x.Out -Raw -ErrorAction SilentlyContinue
        $se = Get-Content "$($x.Out).err" -Raw -ErrorAction SilentlyContinue
        $done = [regex]::Match("$so", 'STRESS_DONE exit=(\d+) responses=(\d+) finished=(\d+) duplicate_finishes=(\d+) cancels=(\d+) tasks_run=(\d+)')
        $teardown = "$so" -match "STRESS_TEARDOWN_COMPLETE"
        $dup = if ($done.Success) { [int]$done.Groups[4].Value } else { -1 }
        if (-not $hung -and $code -eq 0 -and $done.Success -and $teardown -and $dup -eq 0) {
            $ok++
            $responses += [long]$done.Groups[2].Value; $finished += [long]$done.Groups[3].Value
            $cancels += [long]$done.Groups[5].Value; $tasksRun += [long]$done.Groups[6].Value
        } else {
            if ($hung) { $hangs++; $kind = "hang" } elseif ($code -ne 0) { $crashes++; $kind = "crash" } else { $other++; $kind = "bad-output" }
            $f = Join-Path $failDir ("fail-{0}-{1}-{2}.txt" -f $Profile, $x.N, $x.P.Id)
            "run=$($x.N) kind=$kind exit=$code hung=$hung exit-ms=$($x.ExitMs)`n--- stdout`n$so`n--- stderr`n$se" | Set-Content $f
            $failures += "run $($x.N): $kind exit=$code -> $f"
        }
        Remove-Item $x.Out, "$($x.Out).err" -ErrorAction SilentlyContinue
    }
}
$line = "RESULT profile=$Profile mode=$Mode pool=$Pool delay_ms=$DelayMs parallel=$Parallel platform=$Platform exit_ms=$ExitMin-$ExitMax " +
    "runs=$Count ok=$ok crashes=$crashes hangs=$hangs bad_output=$other " +
    "responses=$responses finished=$finished cancels=$cancels tasks_run=$tasksRun elapsed_s=$([int]$sw.Elapsed.TotalSeconds)"
$line
$failures
if ($Out) { Add-Content $Out $line; if ($failures) { Add-Content $Out $failures } }
if ($ok -ne $Count) { exit 1 }
