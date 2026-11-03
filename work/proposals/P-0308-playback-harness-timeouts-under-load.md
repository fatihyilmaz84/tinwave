---
id: P-0308
title: Decouple the playback harness timeouts from CPU load
from: T-0063 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0078
---
`settle`, `wait_buffered`, `expect_edit_applied` give up after ~3 s of 100 µs sleeps; under heavy load correct
tests fail there, main's rate binary too (tests/common/mod.rs:278, 321, 360).
