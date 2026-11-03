---
id: P-0263
title: Real-device exclusive reopen check (ignored test + PO runbook)
from: T-0057 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0066
---
Nothing in T-0057 touched an endpoint: the real gap and a real 96 kHz refusal are unmeasured.
Evidence: crates/tinwave-playback/tests/device.rs (ignored silent-clip pattern).
