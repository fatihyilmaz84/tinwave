# Acceptance matrix for the recommended pattern ("prod" variant) - appends RESULT lines to results\summary-prod.txt
Set-Location $PSScriptRoot
$o = "results\summary-prod.txt"
.\run-batch.ps1 -Build build-dbg  -Provider prod -Count 300 -Parallel 4 -Threads 1 -ExitMin 200 -ExitMax 900 -Out $o | Out-Null
.\run-batch.ps1 -Build build-dbg  -Provider prod -Count 150 -Parallel 4 -Threads 2 -ExitMin 50 -ExitMax 900 -Mode toggle -Out $o | Out-Null
.\run-batch.ps1 -Build build-asan -Provider prod -Count 150 -Parallel 4 -Threads 1 -ExitMin 100 -ExitMax 900 -Out $o | Out-Null
.\run-batch.ps1 -Build build-rel  -Provider prod -Count 300 -Parallel 4 -Threads 2 -ExitMin 50 -ExitMax 900 -Out $o | Out-Null
.\run-batch.ps1 -Build build-rel  -Provider prod -Count 200 -Parallel 8 -Threads 1 -ExitMin 100 -ExitMax 600 -Mode scroll -Out $o | Out-Null
.\run-batch.ps1 -Build build-rel  -Provider prod -Count 40 -Parallel 1 -Threads 2 -ExitMin 100 -ExitMax 900 -Platform windows -Out $o | Out-Null
"MATRIX-DONE" | Add-Content $o
