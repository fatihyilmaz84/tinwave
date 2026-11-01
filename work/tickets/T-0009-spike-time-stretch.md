---
id: T-0009
title: "Spike: time-stretch library for 0.75×–1.5× playback (quality, CPU, license)"
type: spike
epic: E-00
status: in_progress
priority: P2
size: S
depends_on: [T-0003]
scope:
  - spikes/T-0009-time-stretch/
owner: tw-spike (round 1)
branch: t/T-0009-spike-time-stretch
review_round: 0
created_by: lead
---

## Context
Signal catalog §2 (speed signals) requires a pitch-preserving rate control. License rule: prefer MIT/BSD; GPL needs an ADR + PO decision.

## Question
Which time-stretch option integrates with the chosen audio backend at acceptable quality and CPU, with a compatible license?

## Time box
Half a working session.

## Acceptance criteria
- [ ] Compare at least Signalsmith Stretch and one other option: CPU at 1.25×, latency, artifacts on music, license.
- [ ] Integration sketch with the T-0003 backend (real-time safe, no allocation in callback).
- [ ] Draft ADR.

## Signals & storage
Enables `RateChanged`.
