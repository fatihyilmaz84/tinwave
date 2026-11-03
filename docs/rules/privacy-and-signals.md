# Privacy & signal rules

Tinwave collects far more about listening behaviour than a typical player. That is only acceptable — and
only shippable (GDPR/KVKK, App Store review, antivirus heuristics) — because of these rules.

## Tiers

| Tier | What | Default | Leaves device? |
|---|---|---|---|
| **T0 Core** | Playback and library interactions: play, skip, seek, volume, speed, pause, queue, favorites, ratings, playlist edits, search, browse dwell | On | Only via the user's own device sync (spec §8, channel 5) |
| **T1 Local context** | Clock/daypart, holidays, season, sunrise/sunset (offline calc), power state, idle time, output device, OS focus/DND, fullscreen | On | Same as T0 |
| **T2 Sensitive local** | Foreground-app **category**, network place (salted hash), calendar busy/free | **Off — opt-in** | Same as T0 |
| **T3 Network** | Weather | **Off — opt-in** | Yes: rounded lat/lon (0.1°) to the weather provider |

Onboarding offers T2/T3 with plain-language explanations of what they improve ("Weather-aware radio").
Declining has no nagging and no degraded core features.

## Banned forever

Never collect, even with consent:
- microphone or camera input
- screen captures or screen contents
- window titles, document names, URLs
- keystroke or mouse contents; **no global keyboard/mouse hooks** (idle time comes from OS idle APIs only)
- browser history, other apps' data, contacts
- file contents outside the music library
- precise location (anything finer than 0.1° ≈ 11 km), raw SSIDs or MAC addresses

## Data handling

- Collection respects category toggles **at the source** — disabled signals are never constructed, not filtered later.
- Tiered retention: raw events 90 days → play records 3 years → monthly rollups
  (see [storage-budget.md](storage-budget.md)). Less raw data kept is also less sensitive data kept.
- Export: JSON Lines of all signals + context + derived taste, from the Signal Center.
- Wipe: per category, per tier, everything. Wiping signals also clears derived `taste.db`.
  Wiping a **context** category (clock, output device, foreground app, network place, weather) also clears its
  fields from the `ContextKey` kept in every `play` row (3-year tier) and every `context_snapshot` (hot tier),
  in the same transaction as the rest of the wipe; a key left with no known field becomes the raw 0. Every key
  field has exactly one owning category: clock → daypart, weekday type; output device → device class; foreground
  app (T2) → activity; network place (T2) → place slot; weather (T3) → weather condition, temperature band.
  A derived field is also cleared by every category it is derived from (a wipe removes everything derived from
  the wiped data): the activity guess is derived from the foreground app, the network place and the clock, so a
  wipe of any of the three clears it. Monthly rollups carry no context and are untouched. Turning a category or
  tier off only stops new values; existing rows change only on a wipe.
  Wiping a **signal** category (the seven sections of the catalog: volume, speed, seek/skip/replay, queue and
  library, browsing, output and sound, session) deletes its `signal_event` rows and its `ui_counter` rows, and clears
  **what is derived from them** in the same transaction, by one rule: *a derived column is cleared by the wipe of every
  category it is derived from, and nothing else changes* (a score that blends all categories is cleared by each, as the
  activity guess is). A derived row is **deleted** when what identifies it was wiped, or when its cleared columns were
  all it carried. Concretely: a `play` row exists because a `PlayStarted` (queue and library) named the recording and
  the time, so wiping *queue and library* deletes every `play` row and every monthly rollup row; wiping *seek, skip,
  replay* keeps what was played and when, and resets only how the play ended (`listened_ds` 0, `end_frac` 0,
  `end_reason` `Unknown`, which a reader must treat as "no information", never as a skip) and the flag bits and
  `peak_section` that come from those signals; a flag bit is cleared by the category of the signal that sets it
  (volume: `vol_*`, speed: `rate_*`, browsing: `lyrics`, seek/skip/replay: `replayed`, `seek_back`, `seek_fwd_intro`).
  Monthly rollup counters follow their sources the same way (`completes`, `skips_*`, `listened_s`: seek/skip/replay;
  `plays`: seek/skip/replay and queue and library; `fav_delta`: queue and library), and `score_sum`, a blend of every
  category's weights that cannot be split, is reset to 0 by **every** category wipe; a rollup row left with only zero
  counters is deleted. A wipe of a signal category does not touch `context_snapshot`, `session` or the id
  dictionaries (shared by every category, no signal of their own) or the context fields (wiped by the context
  categories above). **Wipe everything** deletes every row of `signals.db` data: events, counters, play records,
  rollups, context snapshots, sessions and the dictionaries; the schema and the Signal Center toggles
  (`signal_settings`, a preference) stay. It covers `signals.db` only: `taste.db` does not exist yet, and the change
  that creates it must add its own wipe (the promise above that wiping signals clears the derived taste data applies
  from then on). Each wipe is one transaction, deleted content is zeroed in the file (`secure_delete`; in the main database file once the WAL is next checkpointed,
  at most the writer's idle interval later — T-0065 makes the wipe checkpoint itself), and the signal
  writer keeps running: its batches wait for the wipe and are stored after it, and it drops its id caches when a
  wipe committed. A play that is in progress when its `PlayStarted` is wiped ends without a play record. A wipe of
  the *events* of a tier (as opposed to the context of a tier, above) is not implemented yet.
- No telemetry, no analytics SDKs, no crash reporters that upload by default (spec §15).
- Never put signal data, recording names or file paths in logs above `debug`, URLs, or crash reports.
- Hashing for places: `blake3(device_secret || ssid || gateway_mac)`, `device_secret` in the OS keychain.

## Interpretation ethics

- Context first: a negative-looking action during an interrupting context (call, notification, lock screen,
  device change) is not scored as dislike.
- Insights are for the user only and phrased neutrally. No mood or health diagnoses
  ("you seem depressed") — describe music, not the person.
- Users can correct the engine: "Not now", "Never in this context", "Forget this play".
