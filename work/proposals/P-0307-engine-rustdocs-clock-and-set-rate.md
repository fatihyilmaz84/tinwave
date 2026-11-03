---
id: P-0307
title: Reword stale engine.rs rustdocs (EngineConfig::clock, set_rate)
from: T-0063 build-1
date: 2026-11-03
size: S
outcome: merged → T-0076 (set_rate part already fixed by the lead on T-0066's merge)
---
`EngineConfig::clock` says audio never uses it; the decoder's rate throttle now reads it (engine.rs:93).
