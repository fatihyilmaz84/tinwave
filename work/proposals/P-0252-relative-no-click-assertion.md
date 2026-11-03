---
id: P-0252
title: Relative no-click assertion at rate restarts
from: T-0027 review-1
date: 2026-11-03
size: S
outcome: accepted → T-0063
---
Compare the max step near each restart with the same recording's steady max instead of 1.03x an analytic sine;
catches fades >= 1.5 ms short. Evidence: crates/tinwave-playback/tests/rate.rs:359.
