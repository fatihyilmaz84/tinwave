# T-0041 reproducer

C++-only harness (no Rust) around the T-0002 async image provider, with several provider variants selectable by
`TW_PROVIDER`. Built three ways: Debug (Qt debug DLLs), Debug + MSVC AddressSanitizer, Release.

Build:   `build.cmd build-dbg` | `build.cmd build-asan asan` | `set BT=Release && build.cmd build-rel`
Run:     `run-batch.ps1 -Build build-rel -Provider queued -Count 300 -Parallel 4 -Threads 1 -ExitMin 200 -ExitMax 900`
Matrix:  `run-prod-matrix.ps1` (acceptance runs of the recommended pattern)

Variants (`providers.cpp`): `olda`, `oldb` (reconstructed first shim), `queued` (T-0002 final), `autoconn` (Qt doc example),
`guarded` (mutex-guarded target pointer), `prod` (recommended: queued + quiesce), `sync`, `selfq`, `silent` (controls).

Crash capture: vectored exception handler in `main.cpp` writes `results/<build>-<provider>/crash-<pid>.txt` (module + RVA
for every frame, source lines for our own code) and a minidump. Qt ships no PDBs in this install, so Qt frames show the
nearest export (misleading names) plus the RVA. Dumps and per-run fail files are git-ignored.

Everything runs with `QT_QPA_PLATFORM=offscreen` and the software scene graph unless `-Platform windows` is given.
No system setting is touched (crash dialogs are suppressed per process with `SetErrorMode`, no WER/IFEO registry keys).
