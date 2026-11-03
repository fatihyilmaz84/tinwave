---
id: P-0236
title: Context hub writes context_snapshot.key through ContextKey::raw()
from: T-0029 build-1
date: 2026-11-03
size: S
outcome: parked: requirement for the context-hub ticket (E-03); recorded in T-0029 notes so it is not lost
---
Without it every real play stores context_key 0 (an invalid key is rejected with a warn and stored as 0).
Evidence: crates/tinwave-db/src/play.rs `context_key`.
