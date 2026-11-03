---
id: P-0233
title: Emit a play-end signal for stops and quits
from: T-0029 build-1
date: 2026-11-03
size: M
outcome: accepted → T-0059
---
The controller emits nothing for `TrackLeft{Stopped}` below 97 % or at quit, so the writer cannot write `Stopped`/`Quit`
play rows and a stop looks like a failure after the track was heard. Evidence: crates/tinwave-playback/src/controller/mod.rs
mapping table; docs/rules/database.md:81.
