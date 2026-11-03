---
id: P-0299
title: Pin the writer's write_batch statement count per batch size
from: T-0073 build-1
date: 2026-11-03
size: S
outcome: merged → T-0074
---
A per-row regression in the hot write path is cheaper to catch by count than by timing.
