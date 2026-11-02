---
id: P-0147
title: Session-boundary property in the controller random walk
from: T-0034 review-2
date: 2026-11-02
size: S
outcome: merged → T-0023 (next ticket in the playback crate; it extends the controller tests)
---
No SessionEnded.minutes may exceed the audible wall time between its SessionStarted and the last audible event; would have caught the paused-session audibility bug generically.
