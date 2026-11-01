---
id: D-0001
title: Which desktop OS do we prove and measure on first?
status: decided
blocks: []
date: 2026-10-29
---

## Question for the PO
Should Phase 0 spikes and early E-01 work target **Windows first**, with macOS/Linux validation later in E-00/E-01?

## Context
The development machine is Windows 11. The spec requires measurements on all three desktop OSes (§12, §17),
but agents can only measure on machines they run on. OS-specific signal providers (system volume, device,
idle, foreground app) differ per OS.

## Options
1. **Windows first** — fastest feedback; macOS/Linux providers get their own tickets; cross-platform measurement
   ticket before E-01 acceptance. Risk: Windows-only assumptions creep in.
2. **All three from the start** — needs Mac and Linux machines (or CI runners) now; slower.

## Lead recommendation
Option 1, with two guards: every OS-specific module sits behind a trait with explicit `Unsupported` on other OSes,
and CI builds/tests on Linux and macOS runners as soon as a remote exists.

## PO decision
2026-11-01: **Option 1 — Windows first**, with the lead's two guards: OS-specific modules behind traits that
return `Unsupported` elsewhere, and Linux/macOS CI as soon as a remote exists. macOS/Linux providers get their
own tickets; a cross-platform measurement ticket must be done before E-01 acceptance (spec §12, §17).
