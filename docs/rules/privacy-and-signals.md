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
  `plays`: queue and library only, because it counts starts, so the seek/skip/replay wipe keeps it, like the play
  rows; `fav_delta`: queue and library), and `score_sum`, a blend of every
  category's weights that cannot be split, is reset to 0 by **every** category wipe; a rollup row left with only zero
  counters is deleted. A wipe of a signal category does not touch `context_snapshot`, `session` or the id
  dictionaries (shared by every category, no signal of their own) or the context fields (wiped by the context
  categories above). **Wipe everything** deletes every row of `signals.db` data: events, counters, play records,
  rollups, context snapshots, sessions and the dictionaries; the schema and the Signal Center toggles
  (`signal_settings`, a preference) stay. It covers `signals.db` only: `taste.db` does not exist yet, and the change
  that creates it must add its own wipe (the promise above that wiping signals clears the derived taste data applies
  from then on). **Wiping a tier** deletes the signals whose kind is of that tier (`SignalKind::tier`; today
  `WindowState` and `OutputDeviceChanged` are T1, every other stored kind is T0, and T2 and T3 have no signal
  kind) with the same derived-data rule, computed from the wiped kinds: a column is cleared when any signal it is
  folded from is wiped (the start of a play, `PlayStarted`, is T0, so wiping T0 deletes the play records), and the
  score, which blends every signal, is reset by any signal wipe. A column that holds a **derived value** (signal
  catalog class D: today `play.rate`, the `RateSession`) is cleared by the wipe of the derived kind **and of every
  input it is computed from** (`SignalKind::derived_inputs`), whatever the category or tier of the wipe: `play.rate`
  by the Speed category (and by T0, which holds its input `RateChanged` but also `PlayStarted`, so it deletes the play
  rows), and a derived kind with a T1 input and T0 inputs, like `PerceivedLevel`, by a T0 wipe, a T1 wipe, and the
  Volume and Output and sound categories alike. A derived value is also not collected
  while a category or tier of any of its inputs is off, and a value that was queued when its kind was switched off
  is dropped, not stored. Wiping a tier in the Signal Center is one call
  (`SignalWiper::wipe_tiers`): the tier's signals, their derived data and the tier's context fields (T1: clock and
  output device, T2: foreground app and network place, T3: weather) go in **one transaction**.
  Each wipe is one transaction, and right after it commits the wiper folds the WAL into the main
  database file (a best-effort checkpoint that never waits and never fails the wipe), so the deleted content, zeroed
  in the pages (`secure_delete`), is gone from the file when the wipe returns. The checkpoint can be blocked by
  either of two things, and it is logged either way: **a reader that still holds an older snapshot** (the pages of the
  wipe cannot be copied past it, so the old content is also still in the main file), or **the signal writer holding
  the write lock** (a batch that queued behind the wipe; the wipe's pages are already in the main file, and only the
  WAL file keeps the frames from before the wipe). In the writer case the signal writer's next checkpoint finishes the
  job, at most its idle interval (60 s) later. In the reader case it cannot: the writer's checkpoint is blocked by the
  same reader and retries every interval, so the deleted content stays in the main file and the WAL file for as long as
  that reader stays open, and the first checkpoint after the reader ended its read transaction (at most one idle
  interval later) removes it. The signal writer keeps running: its batches
  wait for the wipe and are stored after it (a batch that waits longer than the 5 s busy timeout is kept and
  retried, not lost), and wipe everything makes it drop its id caches (a category or tier wipe leaves
  the dictionaries alone, so the caches stay). A play that is in progress when its `PlayStarted` is wiped ends
  without a play record; so does an outcome whose own `PlayStarted` was never stored when an earlier play of the
  same recording already has its record.
- No telemetry, no analytics SDKs, no crash reporters that upload by default (spec §15).
- Never put signal data, recording names or file paths in logs above `debug`, URLs, or crash reports.
- Hashing for places: `blake3(device_secret || ssid || gateway_mac)`, `device_secret` in the OS keychain.

## Interpretation ethics

- Context first: a negative-looking action during an interrupting context (call, notification, lock screen,
  device change) is not scored as dislike.
- Insights are for the user only and phrased neutrally. No mood or health diagnoses
  ("you seem depressed") — describe music, not the person.
- Users can correct the engine: "Not now", "Never in this context", "Forget this play".
