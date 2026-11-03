---
id: P-0303
title: Make cancel_storm robust to a slow QML start
from: T-0074 build-1
date: 2026-11-03
size: S
outcome: merged → T-0071
---
Failed with "no requests at all" in a 150 ms storm on a loaded machine (apps/tinwave-desktop/tests/image_provider.rs:161).
