---
id: T-0068
title: A queue edit during an exclusive boundary park re-evaluates the park
type: fix
epic: E-01
status: backlog
priority: P2
size: S
depends_on: [T-0057]
scope:
  - crates/tinwave-playback/src/decoder.rs
  - crates/tinwave-playback/src/engine.rs
  - crates/tinwave-playback/tests/exclusive_reopen.rs
owner:
branch:
review_round: 0
created_by: from P-0268 (T-0057 review-1)
---

## Context
T-0057: in exclusive mode the decoder parks up to ~1 s before a boundary whose successor has another rate. A
`replace_after` that lands then ("play next") does not cancel the park, so the engine opens a stream for a track that no
longer plays and two same-rate tracks get ~510 frames of silence (replace_after is documented as gapless). A Resume
for a withdrawn park is currently dropped with "nobody to feed it".

## Scope
- On a queue edit during a boundary park, the decoder withdraws the park if the new successor has the stream's rate (and
  continues gapless), or re-parks for the new successor's rate; a Resume for a withdrawn park is ignored safely.

## Acceptance criteria
- [ ] [a 44.1k, b 96k] parked for b, play-next c 44.1k: no 96 kHz open, a→c gapless (bit-exact, no silence) — test.
- [ ] Edit to another rate re-parks for that rate — test.
- [ ] Stable under parallel stress; fmt, clippy, nextest, deny pass.

## Signals & storage
None.
