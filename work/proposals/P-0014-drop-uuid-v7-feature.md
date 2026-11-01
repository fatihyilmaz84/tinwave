---
id: P-0014
title: Drop the uuid v7 feature from workspace dependencies
from: T-0013 builder
date: 2026-11-01
size: S
outcome: accepted → T-0015
---
## Evidence
Cargo.toml `uuid` features `["v7","serde"]`; after T-0013 nothing calls `new_v7`/`now_v7` (T-0004 branch checked: no uses).
