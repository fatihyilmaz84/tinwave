---
id: P-0273
title: Harden controller_thread tests under heavy parallel load
from: T-0057 build-2
date: 2026-11-03
size: S
outcome: accepted → T-0069
---
`dropping_the_handle_ends_the_session` and `queue_edits_go_through_the_handle...` fail ~2-5 % of runs at 40+ concurrent
processes, also on baseline main 7a6cf5d. Evidence: crates/tinwave-playback/tests/controller_thread.rs:297, :343.
