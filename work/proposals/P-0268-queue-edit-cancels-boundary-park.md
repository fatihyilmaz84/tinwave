---
id: P-0268
title: Re-evaluate a boundary park on a queue edit
from: T-0057 review-1
date: 2026-11-03
size: S
outcome: accepted → T-0068
---
A "play next" in the last ~1 s before a rate change opens a stream for a track that no longer plays (opens
[48k, 44.1k, 96k, 44.1k]) and puts ~510 frames of silence between same-rate tracks. Evidence: decoder.rs:497-517, 615-636.
