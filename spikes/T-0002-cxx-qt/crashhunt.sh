#!/bin/bash
# Repeats the full benchmark of the deployed build until it crashes (max 8 runs).
# Output: results/crashhunt/run-N.txt (+ DONE marker). The crash handler in metrics.rs prints "T2 CRASH".
cd "$(dirname "$0")" || exit 1
mkdir -p results/crashhunt
for i in 1 2 3 4 5 6 7 8; do
  /c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe -NoProfile -File run-deployed.ps1 > results/crashhunt/run-$i.txt 2>&1
  if grep -q "CRASH" results/crashhunt/run-$i.txt; then echo "crash in run $i" > results/crashhunt/DONE; break; fi
done
echo finished >> results/crashhunt/DONE
