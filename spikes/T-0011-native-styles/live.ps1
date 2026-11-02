# T-0011 PO runbook helper: open the prototype window for N seconds, print every OS-setting event it sees.
# The window closes itself. This script never changes a Windows setting; YOU change them while it runs.
# Usage:  .\live.ps1                                   (FluentWinUI3, 180 s)
#         .\live.ps1 -Seconds 300 -HostArgs "--mica"
#         .\live.ps1 -HostArgs "--style","Fusion"
param(
    [int] $Seconds = 180,
    [string] $Exe = "$PSScriptRoot\build\t0011-host.exe",
    [string[]] $HostArgs = @()
)
$env:PATH = "C:\Qt\6.12.0\msvc2022_64\bin;$env:PATH"
$env:QT_ASSUME_STDERR_HAS_CONSOLE = "1"
$a = @("--style", "FluentWinUI3") + $HostArgs + @("--live", "$Seconds")
if ($HostArgs -contains "--style") { $a = $HostArgs + @("--live", "$Seconds") }
Write-Host "Starting: $Exe $($a -join ' ')  (closes by itself after $Seconds s)"
& $Exe @a
