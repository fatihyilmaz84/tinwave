#!/bin/bash
# Final measurement + soak of the deployed build: 8 runs with UI-counter forwarding on, 5 with it off.
# Output: results/run-deployed-N.txt, results/run-noforward-N.txt, results/SOAK-DONE
cd "$(dirname "$0")" || exit 1
PS=/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe
rm -f results/run-deployed-*.txt results/run-noforward-*.txt results/SOAK-DONE
for i in 1 2 3 4 5 6 7 8; do
  $PS -NoProfile -File run-deployed.ps1 > results/run-deployed-$i.txt 2>&1
done
for i in 1 2 3 4 5; do
  $PS -NoProfile -File run-deployed.ps1 -Forward 0 > results/run-noforward-$i.txt 2>&1
done
echo finished > results/SOAK-DONE
