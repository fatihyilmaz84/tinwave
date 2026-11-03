---
id: P-0316
title: Expose a cancel counter from the stress-only provider to Storm.qml
from: T-0071 build-1
date: 2026-11-03
size: S
outcome: parked: per-mode retries cover it; revisit if the per-mode check flakes
---
Would let the scene gate on cancels so `cancels > 0` holds per run.
