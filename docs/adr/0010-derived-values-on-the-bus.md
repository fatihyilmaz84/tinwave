# ADR 0010 — Derived signal values travel the bus and are stored with their play

- **Status:** Accepted
- **Date:** 2026-11-03

## Context

The signal catalog has a storage class **D** (derived): `PerceivedLevel` and `RateSession`. A derived signal is
computed from raw events and is not a `signal_event` row, so the bus refused it (`SignalsError::NotAnEvent`). The
first one built, `RateSession` (T-0062), was parked in a per-controller queue (`take_rate_sessions`) that nothing
read: the value existed and was never stored. `PerceivedLevel` would have hit the same wall. We need one way for a
derived value to reach the signals writer that

- respects the Signal Center toggles and the privacy tiers of the inputs it is computed from,
- stores the value where the taste engine reads plays (the `play` row), in the transaction that writes the row,
- never makes playback wait, and keeps the data small (storage-budget.md),
- is cleared by a wipe of anything it is derived from.

## Decision

1. **A third item on the bus.** `tinwave-signals` gets `DerivedValue { id, ts, play: PlayRef, payload: DerivedPayload }`
   and `SignalBus::emit_derived(kind, build)`. It is a bounded queue of its own (`BusConfig::derived_capacity`, 256);
   a full queue loses the new value and counts it in `SignalBus::dropped(kind)`. A value takes its `seq` from the
   event counter and the drain merges events and values in `seq` order, so a batch is a prefix of the merged order.
   Derived values are **not** counted in `queued_total` (they are not events).
2. **The value names its play** (`PlayRef`: session, recording, and the time of the `PlayStarted` event), which is
   what the `play` row is made of. The writer finds the row with one equality `UPDATE` and creates nothing for a play
   that has no row.
3. **Stored in the play row, quantized.** `RateSession` becomes `play.rate`, a nullable `u8` (hundredths minus 50, the
   `Rate` type), by migration 0003: about 2 bytes per play. `SignalSink` gets `write_with_derived(events, counters,
   derived)`, the method the drain calls; its default forwards to `write` and drops the values, for sinks that have
   nowhere to keep one (test collectors). `SignalStore` stores them in the same transaction as the events, after the
   play rows of the batch.
4. **Emit just before the outcome.** The controller emits the value right before `Completed` / `Skipped`; the
   outcome flushes the writer at once, so the row is made and filled by one transaction. A value alone never makes the
   writer commit. A value that finds no row (its outcome is a few events behind, or the play was stopped) is held for
   one more commit and then dropped and counted (`WriterStats::derived_unmatched`).
5. **Consent follows the inputs.** `SignalSettings::enabled` for a derived kind requires the category and tier of the
   kind **and of every input** (`derived_inputs`). It is checked when the value is emitted, again when it leaves the
   bus queue (`SignalBus::derived_withdrawn`), and again by the writer just before it commits
   (`WriterStats::derived_withdrawn`), so a value queued before a toggle-off is dropped, not stored.
6. **Wipes.** A column that holds a derived value is cleared by the wipe of the derived kind and of every input it is
   computed from, whatever the category or tier of the wipe (`signal_wipe.rs`, `wipes`): `play.rate` by Speed and by
   T0; a future `PerceivedLevel` column by T0, T1, Volume and Output and sound. A play row deleted by the Queue and
   library wipe takes its value with it.
7. The controller's `take_rate_sessions` and `RateSession` type are removed.

## Consequences

- Adding a derived signal is a `DerivedPayload` variant, a column, a branch in `play::write_derived` and a wipe entry
  (`docs/guides/adding-a-signal.md`); the `match`es have no wildcard, so a missing step does not compile.
- A **wipe does not reach a value that is already in flight**: a value queued or staged (at most one flush interval)
  when a wipe commits is stored afterwards, like an event in the same position. `SignalWiper` is a separate
  connection with no link to the bus; closing that gap needs the composition root to give both the same handle
  (a wipe generation the bus checks); that is not part of this change.
- Stopped plays have no record yet, so their value is counted and dropped until a play-end signal for them exists.
- `PerceivedLevel` has no variant, column or producer yet; the rules (consent, wipe) already cover it.

## Spec / docs updated

- `docs/guides/signal-catalog.md` (RateSession row, derived-signal paragraph, section 9),
  `docs/guides/adding-a-signal.md` (derived signals), `docs/rules/database.md` (migration 0003, `play.rate`),
  `docs/rules/storage-budget.md` (play record), `docs/rules/privacy-and-signals.md` (wipe of derived columns).
