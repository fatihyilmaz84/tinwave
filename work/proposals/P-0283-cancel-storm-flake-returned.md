---
id: P-0283
title: cancel_storm flake returned once in a full-workspace run
from: T-0065 review-1
date: 2026-11-03
size: S
outcome: accepted → T-0071
---
`tinwave-desktop::image_provider cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` failed "nothing was
cancelled" once (T-0049/P-0169 considered it fixed); passed on rerun.
