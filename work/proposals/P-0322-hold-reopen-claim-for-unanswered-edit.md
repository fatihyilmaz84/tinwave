---
id: P-0322
title: Hold the engine's reopen claim back while a replace_after is unanswered
from: T-0068 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0081
---
An edit queued but not yet taken when the old stream has played out loses the claim race and costs one wasted open
at the old successor's rate (audible gap). Needs an edit-acknowledged counter in `Shared`.
