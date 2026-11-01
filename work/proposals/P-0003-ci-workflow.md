---
id: P-0003
title: CI workflow running fmt, clippy, nextest and cargo deny
from: T-0001 builder
date: 2026-11-01
size: S
outcome: parked
---

## Why
desktop-dev-setup.md "Before every PR" lists these gates but nothing enforces them.

## Evidence
docs/guides/desktop-dev-setup.md:46-53.

## Suggested approach
Parked until the repo has a remote (D-0001: Linux/macOS CI as soon as a remote exists). Until then the lead runs
the gates on `main` after every merge (work/README.md Integrate step). Revisit together with the D-0001 CI guard.
