---
id: P-0085
title: Drop the UNIQUE index on context_snapshot.hash
from: T-0007 builder
date: 2026-11-02
size: S
outcome: rejected: the index is what makes snapshot dedupe correct across restarts; ~20 B/snapshot (~1.2 KB/day) is affordable
---
