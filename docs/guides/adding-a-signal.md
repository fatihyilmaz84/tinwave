# Adding a signal

Follow every step. A signal missing any of them is not done.

1. **Justify it.** Write one sentence: what recommendation or insight gets better? If you can't, don't add it.
2. **Check the banned list** in [privacy-and-signals.md](../rules/privacy-and-signals.md). If it touches anything there, stop.
3. **Pick a tier** (T0–T3). Anything that reveals what the user does *outside* Tinwave is at least T2.
4. **Add to the catalog**: a row in [signal-catalog.md](signal-catalog.md) with tier, trigger, payload, meaning.
5. **Define the kind** in `tinwave-signals/src/kind.rs`: new enum variant with a **new, never-reused**
   discriminant and a versioned payload struct (`payload_version = 1`).
6. **Emit at the source** — the place where the fact happens (playback engine, view model, context
   provider), with `InputSource` when user-initiated. Emission must be non-blocking and respect the
   category toggle before constructing the event.
7. **Interpret it** — add a weight / interpretation in `tinwave-taste/src/weights.rs` and, if it yields
   a new pattern, an explanation template.
8. **Expose it** — add it to the Signal Center category list (name, description, tier, toggle) and to export.
9. **Test it** — emission, interpretation (including interruption contexts), privacy-off, and update a replay fixture.
10. **ADR** if the tier is T2 or T3, or if it needs a new OS permission.
11. **Size it** — add an events/day and bytes/day estimate for the reference user to the catalog's volume
    table. Over ~50/hour ⇒ make it a counter. Over 20 KB/day ⇒ coalesce, quantize, or write an ADR.
    Decide how it folds into play records (a flag bit?) and monthly rollups before its raw rows expire.
    Add it to the growth-test generator.
12. **Benchmark** if it fires more than once per second.

## A derived signal (class D)

A signal computed from other signals (`RateSession`, `PerceivedLevel`) is not a `signal_event`: the bus refuses it
(`NotAnEvent`). Steps 4, 5 (give it `StorageClass::Derived` and its `derived_inputs`), 8, 9 and 11 apply as above. Then:

- Add a `DerivedPayload` variant in `tinwave-signals/src/derived.rs`; every consumer `match`es it without a wildcard.
- Emit it with `SignalBus::emit_derived`, **just before** the event that ends the thing it describes, so the writer
  stores both in one transaction. It is collected only while the category and tier of the kind and of every input are on.
- Give it a place in `signals.db` (a nullable column on the play record, by a new migration) and a branch in
  `tinwave-db/src/play.rs::write_derived`.
- Register the column in `signal_wipe.rs` (`PLAY_COLUMNS`, `from: &[the derived kind]`): a wipe of the kind or of any
  of its inputs clears it. Test the wipe of every category and tier the kind or an input belongs to.
- Size it (a derived column costs bytes in every play record) and add it to the growth-test generator.

## Payload evolution

Never change a payload in place. Bump `payload_version`, keep a decoder for every old version, and add a
migration test that decodes fixtures of each version.
