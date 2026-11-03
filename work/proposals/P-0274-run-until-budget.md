---
id: P-0274
title: controller_support::Rig::run_until budget hardening
from: T-0057 review-2
date: 2026-11-03
size: S
outcome: merged → T-0069
---
`controller_exclusive::losing_exclusive_mode_to_a_refusing_device...` timed out 1/120 at 40 copies (iteration budget,
not a hang). Evidence: crates/tinwave-playback/tests/controller_support/mod.rs:290-300.
