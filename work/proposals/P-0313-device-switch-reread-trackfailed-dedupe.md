---
id: P-0313
title: Treat a device-switch re-read like a rate restart for TrackFailed dedupe
from: T-0063 review-1
date: 2026-11-03
size: S
outcome: merged → T-0077
---
`Reconfigure` -> `restart` clears `reported` (decoder.rs:713), so re-reading a broken spot after `switch_device`
reports a second TrackFailed.
