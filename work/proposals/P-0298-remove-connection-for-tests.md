---
id: P-0298
title: Remove SignalWiper::connection_for_tests from the public (hidden) API
from: T-0073 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0074
---
A doc(hidden) pub accessor to the raw Connection lets any crate (the desktop app included) reach SQLite through the
wiper, against the "UI never touches SQLite" rule. Use a `test-support` cargo feature (dev-dependency only) or move the
count helper in-crate. Evidence: crates/tinwave-db/src/wipe.rs.
