---
id: P-0267
title: The listener's exclusive setting reaches the controller, so a toggle during fallback still emits a signal
from: T-0057 build-1b
date: 2026-11-03
size: M
outcome: accepted → T-0067
---
With the T-0057 ruling the mapper sees only stream modes: a listener turning exclusive off while the stream is already
in fallback Shared changes no mode, so no `ExclusiveModeToggled` is emitted — a preference change without its signal
(CLAUDE.md hard rule). Evidence: crates/tinwave-playback/src/controller/mapper.rs `output_reopened`.
