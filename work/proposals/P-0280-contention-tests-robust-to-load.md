---
id: P-0280
title: Reference-size contention tests robust to machine load
from: T-0065 review-1
date: 2026-11-03
size: S
outcome: merged → T-0064
---
Under 32 parallel heavy processes 3/48 (main) and 7/48 (branch) category-wipe runs fail "database is locked".
Assert the wait against the measured wipe time, or that the writer's retry stores the batch.
