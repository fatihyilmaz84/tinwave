---
id: P-0005
title: Correct ticket text that says exclude = ["spikes/*"]
from: T-0001 reviewer
date: 2026-11-01
size: S
outcome: accepted → done by lead (T-0001 ticket text fixed; template has no such text)
---

## Why
The non-working glob came from the ticket wording; other tickets would copy it.

## Evidence
work/tickets/T-0001-workspace-and-core-ids.md scope; review-1 blocking finding.

## Suggested approach
Say `exclude = ["spikes"]` and note that Cargo's `exclude` is a path prefix, not a glob.
