---
id: P-0330
title: Test that the reopen claim raises the position floor to the full total
from: T-0068 review-2
date: 2026-11-04
size: S
outcome: merged → T-0081
---
Mutation `heard_floor.fetch_max(total)` → `let _ = total;` (engine.rs ~927) survives every test; a claim right after idle
crosses latency, with no `position()` read in between, would cover it.
