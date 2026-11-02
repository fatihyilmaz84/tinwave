# T-0011: startup + RSS matrix. Round-robin over configs (so slow background load hits every config equally).
# Usage: .\run-matrix.ps1 -Exe <path\t0011-host.exe> -Runs 9 -Out results\matrix.csv
param(
    [Parameter(Mandatory)] [string] $Exe,
    [int] $Runs = 9,
    [string] $Out = "results\matrix.csv"
)
. "$PSScriptRoot\run.ps1"

$configs = @(
    @{ name = "Basic";        style = "Basic";        page = "minimal"; extra = @() },
    @{ name = "Fusion";       style = "Fusion";       page = "minimal"; extra = @() },
    @{ name = "Windows";      style = "Windows";      page = "minimal"; extra = @() },
    @{ name = "FluentWinUI3"; style = "FluentWinUI3"; page = "minimal"; extra = @() },
    @{ name = "Basic";        style = "Basic";        page = "controls"; extra = @() },
    @{ name = "Fusion";       style = "Fusion";       page = "controls"; extra = @() },
    @{ name = "Windows";      style = "Windows";      page = "controls"; extra = @() },
    @{ name = "FluentWinUI3"; style = "FluentWinUI3"; page = "controls"; extra = @() },
    @{ name = "FluentWinUI3+Mica"; style = "FluentWinUI3"; page = "controls"; extra = @("--mica") }
)

$rows = @()
# one discarded warm-up run per config (disk cache, shader cache, DLL load)
foreach ($c in $configs) { $null = Invoke-Host -Exe $Exe -HostArgs (@("--style", $c.style, "--page", $c.page, "--idle", "1500") + $c.extra) }
for ($i = 1; $i -le $Runs; $i++) {
    foreach ($c in $configs) {
        $l = Invoke-Host -Exe $Exe -HostArgs (@("--style", $c.style, "--page", $c.page, "--idle", "1500") + $c.extra)
        $rows += [pscustomobject]@{
            config = $c.name; page = $c.page; run = $i
            engine_ms = Get-Metric $l "engine_loaded" "t_ms"
            first_frame_ms = Get-Metric $l "first_frame" "t_ms"
            ws_first_mb = Get-Metric $l "first_frame" "ws_mb"
            cpu_first_ms = Get-Metric $l "first_frame" "cpu_ms"
            ws_idle_mb = Get-Metric $l "idle" "ws_mb"
            private_ws_idle_mb = Get-Metric $l "idle" "private_ws_mb"
            commit_idle_mb = Get-Metric $l "idle" "commit_mb"
            cpu_ms = Get-Metric $l "idle" "cpu_ms"
        }
    }
}
$rows | Export-Csv -NoTypeInformation -Path $Out

function Med($v) { $s = @($v | Where-Object { $_ -ne $null } | Sort-Object); if ($s.Count -eq 0) { return $null }; $s[[int][math]::Floor(($s.Count - 1) / 2)] }
"{0,-20} {1,-9} {2,9} {3,12} {4,10} {5,10} {6,10} {7,10}" -f "config", "page", "engine_ms", "first_frame", "ws_idle", "privWS", "commit", "idleCPUms"
foreach ($g in ($rows | Group-Object config, page)) {
    $r = $g.Group
    "{0,-20} {1,-9} {2,9} {3,12} {4,10} {5,10} {6,10} {7,10}" -f $r[0].config, $r[0].page, (Med $r.engine_ms), (Med $r.first_frame_ms), (Med $r.ws_idle_mb), (Med $r.private_ws_idle_mb), (Med $r.commit_idle_mb), (Med ($r | % { $_.cpu_ms - $_.cpu_first_ms }))
}
