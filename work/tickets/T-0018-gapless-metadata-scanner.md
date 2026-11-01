---
id: T-0018
title: Extract gapless metadata (M4A iTunSMPB/elst, MP3 LAME/Xing) per file and pass it to the reader
type: feature
epic: E-01
status: backlog
priority: P1
size: S
depends_on: [T-0016]
scope:
  - crates/tinwave-library/   # scanner/tag-read part (exact paths set when the scanner ticket exists)
owner:
branch:
review_round: 0
created_by: from P-0024
---

## Context
Symphonia ignores MP4 edit lists and iTunSMPB, so AAC/M4A has an 18-20 ms boundary gap without our own trim
(T-0003: +887 frames at 48 kHz; 0 with an external trim). Spec §7 gapless.

## Scope
- Parse iTunSMPB and `edts/elst` for M4A, LAME/Xing for MP3; store encoder delay + padding per file; hand them to the
  engine reader as an explicit trim.
- Untagged MP3: no heuristic (lead decision) — the 24-48 ms gap remains and is documented.

## Acceptance criteria
- [ ] An M4A split pair with known priming plays with a 0-frame gap — test.
- [ ] Unknown/odd encoders never fail the scan (tolerant parsing) — fixture tests.

## Signals & storage
None. Small per-file metadata in library.db.
