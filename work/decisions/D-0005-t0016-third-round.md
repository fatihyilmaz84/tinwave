---
id: D-0005
title: T-0016 failed review twice — allow a third round, or should the lead take over?
status: decided
blocks: [T-0016]
date: 2026-11-02
---

## Question for the PO
The playback engine core (T-0016) failed review twice. work/README.md caps reviews at two rounds and escalates a
ticket that failed twice to the PO. How should we finish it?

## Context
- Round 1 problems (flaky tests under load, lost corrupt-packet counts) are fixed and verified by mutation and a
  960-process stress run. Everything else (gapless, sample-exact seek, zero-allocation callback, licensing) passed.
- Round 2 found three small, precisely located problems, all from the new "every TrackStarted gets exactly one
  TrackLeft" rule: seek after the queue ended emits a second "finished"; a failed device switch emits a duplicate
  "started"; one new test can hang under heavy load (1 in 528 stress runs). Each fix is a few lines plus a test.
  Review: work/reports/T-0016/review-2.md.
- The reviewer suggests a property test that drives random command sequences and checks the pairing after each step,
  because both bugs came from interleavings nobody wrote by hand.

## Options
1. **Third round, same builder** — it keeps its context; fixes + the reviewer's repro tests + the property test (P-0059).
2. **Lead takes over** — the lead (Opus) makes the three fixes itself on the branch; a fresh reviewer checks them.
3. **Split** — merge T-0016 as is, with the three issues as a new P0 ticket. Not recommended: main would carry a known
   flaky test and a pairing bug the signal layer would turn into phantom "completed" plays.

## Lead recommendation
Option 1 including the property test: progress between rounds is real, the remaining issues are small and exactly
specified, and the property test is what stops a fourth round.

## PO decision
2026-11-02: **Option 3 — merge now, fix after.** Merged as b150fe6; the three issues and the property
test (P-0059) are P0 ticket T-0028, dispatched immediately to the same builder.
