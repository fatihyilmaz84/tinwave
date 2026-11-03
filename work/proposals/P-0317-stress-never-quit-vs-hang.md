---
id: P-0317
title: Tell "never quit" apart from "hang at exit" in run_stress
from: T-0071 review-1
date: 2026-11-03
size: S
outcome: merged → T-0079
---
No STRESS_DONE in stdout means the scene never called Qt.quit (a starved canary, a dead provider); today it reads as a
teardown hang.
