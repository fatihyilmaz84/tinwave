---
id: P-0254
title: Listening check: gapless album junctions while stretched
from: T-0027 review-1
date: 2026-11-03
size: S
outcome: parked: PO listening pass, with P-0251 and the ADR 0006 pass
---
Each track gets finish() and a cold begin(); the cold start overshoots ~4-5 % for ~60 ms, so a junction at != 1.0 may blur.
