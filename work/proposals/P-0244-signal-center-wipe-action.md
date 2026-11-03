---
id: P-0244
title: Wire the Signal Center wipe action to SignalWiper
from: T-0060 build-1
date: 2026-11-03
size: S
outcome: parked: requirement for the Signal Center screen ticket (E-01 UI; artboard exists on the canvas)
---
Nothing calls `SignalWiper` yet; the UI must expose the context categories and the tier wipe. Evidence: crates/tinwave-db/src/wipe.rs. Run it off the GUI thread (T-0060 review-2: ~0.1-0.2 s at reference size).
