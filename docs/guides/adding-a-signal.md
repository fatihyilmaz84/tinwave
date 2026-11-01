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

## Payload evolution

Never change a payload in place. Bump `payload_version`, keep a decoder for every old version, and add a
migration test that decodes fixtures of each version.
