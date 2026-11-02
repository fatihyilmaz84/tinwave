# T-0011 helper: run the host once, capture stdout, enforce a hard timeout, leave nothing running.
# Usage:  . .\run.ps1 ; Invoke-Host -Exe <path\t0011-host.exe> -Args "--style","FluentWinUI3","--shot","x.png"
$script:QtBin = "C:\Qt\6.12.0\msvc2022_64\bin"

function Invoke-Host {
    param(
        [Parameter(Mandatory)] [string] $Exe,
        [string[]] $HostArgs = @(),
        [int] $TimeoutSec = 30
    )
    $env:PATH = "$script:QtBin;$env:PATH"
    $env:QT_ASSUME_STDERR_HAS_CONSOLE = "1"
    $out = [IO.Path]::GetTempFileName(); $err = [IO.Path]::GetTempFileName()
    $p = Start-Process -FilePath $Exe -ArgumentList $HostArgs -PassThru -NoNewWindow `
        -RedirectStandardOutput $out -RedirectStandardError $err
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        Stop-Process -Id $p.Id -Force      # only ever our own child
        "WATCHDOG-KILLED" | Add-Content $out
    }
    $lines = @(Get-Content $out) + @(Get-Content $err | ForEach-Object { "STDERR $_" })
    Remove-Item $out, $err -ErrorAction SilentlyContinue
    return $lines
}

function Get-Metric {
    param([string[]] $Lines, [string] $Tag, [string] $Key)
    $l = $Lines | Where-Object { $_ -match "^METRIC $Tag " } | Select-Object -First 1
    if (-not $l) { return $null }
    if ($l -match "$Key=([0-9.]+)") { return [double]$Matches[1] }
    return $null
}
