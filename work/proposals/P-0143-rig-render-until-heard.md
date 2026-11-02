---
id: P-0143
title: Rig::render_until_heard test helper (wait for the pending command before counting frames)
from: T-0034 build-2
date: 2026-11-02
size: S
outcome: merged → T-0023
---
hear() in the sync rig has the stale-frame hazard found in T-0034 round 2; avoided today only by render_exact calling settle() first.
