---
id: P-0128
title: ControllerConfig::initial_repeat (seed a restored repeat mode without a signal)
from: T-0024 review-2
date: 2026-11-02
size: S
outcome: merged → T-0034 (same "no hand-passed initial state" theme, same crate; must land before any ticket restores repeat at launch)
---
Without it, once the shell restores the saved repeat mode via `set_repeat_mode`, every launch emits a false
RepeatModeChanged right after the first SessionStarted. Seed both `repeat` and `reported_repeat`, as `initial_mode` does.
