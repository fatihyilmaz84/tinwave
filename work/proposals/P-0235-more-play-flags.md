---
id: P-0235
title: Derive more PlayFlags once their sources exist
from: T-0029 build-1
date: 2026-11-03
size: M
outcome: parked: until audio analysis (seek_fwd_intro), lyrics and queue signals exist; vol_*/rate_* revisit after T-0027
---
Only FAVORITED and SEEK_BACK are derived today (2 of 11). Evidence: crates/tinwave-db/src/play.rs `scan_window`.
