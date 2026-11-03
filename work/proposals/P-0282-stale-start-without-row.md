---
id: P-0282
title: A stale start that never got a row can still pair with an orphan outcome
from: T-0065 review-1
date: 2026-11-03
size: S
outcome: merged → T-0059
---
Stopped/failed plays leave a bare PlayStarted; with Queue & library off an orphan outcome of the same recording pairs
with it. The T-0059 play-end signal closes most of it; add the hot-window guard there. Evidence: play.rs:284.
