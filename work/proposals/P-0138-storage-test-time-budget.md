---
id: P-0138
title: Make the storage-growth speed budget robust to a loaded machine
from: lead (gate run after T-0035 merge)
date: 2026-11-02
size: S
outcome: accepted → T-0047
---
storage_growth.rs:288 asserts wall-clock < 60 s; it took 97 s at 100 % CPU (parallel agent test runs) vs 32 s idle. Sizes were within budget.
