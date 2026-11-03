---
id: P-0314
title: Raise HANG_AFTER in tests/image_provider.rs from 60 s to ~180 s
from: T-0071 build-1
date: 2026-11-03
size: S
outcome: merged → T-0079
---
Healthy image-stress runs exceed 60 s at ~10x machine slowdown (old scene max 69 s, new scene 94 s), giving false
hang failures (tests/image_provider.rs:20).
