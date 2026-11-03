---
id: P-0245
title: Wipe context_snapshot.fields with the ticket that defines its encoding
from: T-0060 build-1
date: 2026-11-03
size: S
outcome: parked: requirement for the context-hub ticket (E-03), together with P-0236
---
The blob will hold weather, place and app values; it has no encoding yet, so nothing writes it and nothing is exposed today.
Evidence: crates/tinwave-db/migrations/signals/0001_init.sql:44; wipe.rs module docs "Not yet covered".
