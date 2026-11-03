---
id: T-0057
title: Exclusive mode reopens the output at the file rate on a rate change (ADR 0005 decision 5)
type: feature
epic: E-01
status: done
priority: P1
size: M
depends_on: [T-0019]
scope:
  - crates/tinwave-playback/
owner: tw-builder
branch: t/T-0057-exclusive-reopen-at-file-rate
review_round: 2
merge: e1893f5
created_by: from P-0224 (T-0019 build-1)
---

## Context
ADR 0005 decision 5: in exclusive mode the output follows the file's sample rate (bit-perfect); in shared mode the
decoder-thread resampler (T-0019) converts to the device rate. Today the engine fixes the stream rate per engine
(`ring(self.sample_rate)` in `switch_device`), so the exclusive reopen isn't implemented anywhere.

## Scope
- In exclusive mode, a track at a different rate reopens the output at that rate at the track boundary (no resampling);
  if the device refuses the rate, fall back to shared + resampler and report it (StreamInfo, ExclusiveModeToggled rules
  from the catalog).
- `ManualOutput` gains a configurable device rate set so tests can cover reopen and refusal without a device.

## Acceptance criteria
- [ ] Exclusive, 44.1 k → 96 k tracks: the output reopens at 96 k at the boundary; both tracks bit-exact; no callback
      allocation — tests.
- [ ] Device refuses the rate: falls back to shared + resampler, reported once — test.
- [ ] Signals per the catalog (no new kinds); fmt, clippy, tests, deny pass.

## Lead takeover after review-2 (2026-11-03)
Review-2 blocker (switch_device while a Resume is held back closed the working stream) fixed by the lead on the
branch (95f57d4): `switch_device` returns Busy while `unsent_resume` is set; test
`a_device_switch_waits_while_a_resume_is_held_back`, revert-checked (fails without the guard).

## Signals & storage
Uses existing OutputReopened / ExclusiveModeToggled mapping (T-0024/T-0034).
