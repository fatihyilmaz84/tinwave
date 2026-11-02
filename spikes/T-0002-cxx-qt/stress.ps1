# Concurrency stress: N instances of the same deployed build at once (CPU/GPU contention widens race windows).
param([string]$Dir = "$PSScriptRoot\deploy-min", [int]$N = 4, [int]$Rounds = 1)
$env:PATH = "C:\Windows\System32;C:\Windows"
$env:T2_MODE = "list"; $env:T2_BENCH = "1"; $env:T2_STYLE = "FluentWinUI3"; $env:T2_FORWARD = "1"
$env:QT_FORCE_STDERR_LOGGING = "1"
$crashes = 0; $runs = 0
for ($r = 0; $r -lt $Rounds; $r++) {
    $ps = @()
    for ($i = 0; $i -lt $N; $i++) {
        $o = [IO.Path]::GetTempFileName()
        $p = Start-Process -FilePath "$Dir\t0002-cxx-qt.exe" -WorkingDirectory $Dir -PassThru -NoNewWindow -RedirectStandardOutput $o -RedirectStandardError "$o.err"
        $null = $p.Handle
        $ps += [pscustomobject]@{ P = $p; Out = $o }
    }
    foreach ($x in $ps) {
        if (-not $x.P.WaitForExit(150000)) { $x.P.Kill(); "TIMEOUT" }
        $runs++
        $code = $x.P.ExitCode
        $err = Get-Content "$($x.Out).err" -ErrorAction SilentlyContinue
        $done = (Get-Content $x.Out | Select-String "T2 final").Count
        if ($code -ne 0 -or $done -eq 0) { $crashes++; "round $r exit=$code completed=$done"; $err | Select-Object -First 14 }
        Remove-Item $x.Out, "$($x.Out).err" -ErrorAction SilentlyContinue
    }
}
"stress: runs=$runs failed=$crashes"
