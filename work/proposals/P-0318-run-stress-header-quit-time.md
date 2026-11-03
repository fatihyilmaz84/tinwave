---
id: P-0318
title: run-stress.ps1 header and ADR 0008 rule 6: quit time counts from the first answer
from: T-0071 review-1
date: 2026-11-03
size: S
outcome: merged → T-0079
---
The header says "random quit time" and `-ExitMin 50` implies 50 ms after load; it now counts from the canary's answer
plus 3 jumps.
