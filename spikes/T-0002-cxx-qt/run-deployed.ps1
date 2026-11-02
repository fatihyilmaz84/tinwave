# Run the deployed copy with Qt REMOVED from PATH to prove it is self-contained.
param([string]$Dir = "$PSScriptRoot\deploy-min", [string]$Mode = "list", [string]$Style = "FluentWinUI3", [string]$Forward = "1")
$env:PATH = "C:\Windows\System32;C:\Windows"
$env:T2_MODE = $Mode; $env:T2_BENCH = "1"; $env:T2_STYLE = $Style; $env:T2_FORWARD = $Forward
$env:QT_FORCE_STDERR_LOGGING = "1"
$sw = [Diagnostics.Stopwatch]::StartNew()
$out = [IO.Path]::GetTempFileName()
$p = Start-Process -FilePath "$Dir\t0002-cxx-qt.exe" -WorkingDirectory $Dir -PassThru -NoNewWindow -RedirectStandardOutput $out -RedirectStandardError "$out.err"
$null = $p.Handle
if (-not $p.WaitForExit(120000)) { $p.Kill(); "TIMEOUT-KILLED" }
"wall_ms=$($sw.ElapsedMilliseconds) exit=$($p.ExitCode)"
Get-Content $out; Get-Content "$out.err"; Remove-Item $out, "$out.err"
