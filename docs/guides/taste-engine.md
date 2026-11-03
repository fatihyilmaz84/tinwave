# Taste engine

Turns raw signals into a taste model that answers: *how much will this listener enjoy this track, right now,
in this context — and why?* Lives in `tinwave-taste`. Everything is rebuildable from `signals.db` at the
granularity still retained (raw events for 90 days, play records for 3 years, monthly rollups beyond).
Aggregates are maintained incrementally, so compaction loses no taste — only the ability to recompute
old history at full detail. Size caps: [storage-budget.md](../rules/storage-budget.md).

Constraints (spec §10, §12): lightweight, explainable, no resident ML model, no cloud.

## 1. Event → implicit score

Each event becomes a score `s ∈ [-2, +2]` for the recording (and, with smaller weight, its artist,
album, genres). Starting weights (`weights.rs`, tunable, replay-tested):

| Event | Score |
|---|---|
| Favorited | +2.0 |
| Completed | +1.0 |
| Replayed (per loop, ×0.7 each extra) | +0.8 |
| SeekBack (replay segment) | +0.6 |
| VolumeChanged up in first 15 s | +0.5 |
| PreviousPressed (back to this track) | +0.5 |
| QueuedNext / QueuedLater | +0.4 |
| Lyrics view open during play | +0.2 |
| Skipped > 80 % | +0.2 |
| RateChanged > 1.0 (non-workout) | −0.2 |
| Skipped 30 s – 50 % | −0.2 |
| RemovedFromQueue | −0.4 (−0.8 if radio pick) |
| Skipped 5 – 30 s | −0.6 |
| Skipped < 5 s | −1.0 |
| Unfavorited | −1.0 |
| "Never in this context" | −2.0 in that context bucket only |

Modifiers:
- **Origin**: manual play ×1.3, album/playlist play-through ×1.0, shuffle/radio ×0.8.
- **Attention**: foreground listening ×1.2, background (`idle_s > 300`) ×0.6 (but background skips ×1.3 — they bothered).
- **Interruption guard**: events within 3 s of a call, device change, mute, or lock ⇒ ×0.
- **Rating**: stars map to a persistent prior, not an event score.

## 2. Time decay — three tastes at once

Each aggregate is kept with three half-lives:

| Name | Half-life | Meaning |
|---|---|---|
| `now` | 7 days | Current obsession |
| `season` | 90 days | This period of life |
| `alltime` | 2 years | Core identity |

`score_h(t) = Σ s_i · 2^(−(t − t_i)/h)` maintained incrementally (store value + last-update time).
Diverging `now` vs `alltime` ⇒ **taste drift** insight. High `alltime`, ~0 `now` ⇒ **rediscovery** candidate.

## 3. Context buckets

A context key is a tuple of coarse dimensions:

```
daypart × weekday_type(Weekday|Weekend|Holiday) × weather_bucket × device_class × activity × place
```

For each `(entity, context_key)` keep a Beta posterior: `α += max(s,0)`, `β += max(−s,0)` (decayed with the
`season` half-life). Sparse buckets back off hierarchically:

```
full key → drop place → drop weather → drop device → (daypart, weekday_type) → global
```

Blend by evidence: `p = Σ w_k · mean_k` with `w_k ∝ (α_k+β_k)` and a minimum-evidence threshold per level.
Unknown context dimensions simply skip that level.

**Keeping it small** — the full key space is ~10 000 combinations; storing that for every track would be
gigabytes. So:
- **Tracks** keep buckets only at the coarse level `(daypart, weekday_type)`.
- **Artists, albums, genres** keep the full key hierarchy. A track's fine-grained context affinity is
  inherited from its artist/genre buckets, adjusted by its own coarse bucket.
- Buckets with decayed evidence `α+β < 0.5` are pruned; max 16 buckets per entity (highest evidence wins).
- Entities with no signals have no rows at all.
- Values stored as `f32`; the context key is a packed `u32`.

### ContextKey layout (version 1)

The packed `u32` is the `ContextKey` type in `crates/tinwave-signals/src/context_key.rs`; it is stored in
`play.context_key` and `context_snapshot.key`, so the layout is frozen (a field never moves, a code is never
reused; a change needs a new layout version). Bit 0 is the least significant bit. **In every field 0 means
"unknown"**; known values start at 1.

| Bits | Width | Field | Known values (code = value) |
|---|---|---|---|
| 0-2 | 3 | daypart | 1 `Night`, 2 `EarlyMorning`, 3 `Morning`, 4 `Midday`, 5 `Afternoon`, 6 `Evening`, 7 `LateNight` |
| 3-4 | 2 | weekday type | 1 `Weekday`, 2 `Weekend`, 3 `Holiday` |
| 5-7 | 3 | weather condition | 1 `Clear`, 2 `Cloudy`, 3 `Rain`, 4 `Storm`, 5 `Snow`, 6 `Fog` |
| 8-10 | 3 | temperature band | 1 `Cold` (<5), 2 `Cool` (5-15), 3 `Mild` (15-24), 4 `Warm` (24-30), 5 `Hot` (>30) |
| 11-13 | 3 | device class | the output class plus one: 1 `Headphones`, 2 `Speakers`, 3 `Bluetooth`, 4 `UsbDac`, 5 `Hdmi`, 6 `Builtin`, 7 `Virtual`; `Unknown` is 0 |
| 14-17 | 4 | activity | 1 `Focus`, 2 `Gaming`, 3 `Call`, 4 `Background`, 5 `OnTheMove`, 6 `Leisure`, 7 `Workout`, 8 `Commute`, 9 `Driving` |
| 18-23 | 6 | place slot | 1-63: the place's slot in the device's own small place table; never an SSID, a MAC or a hash of one, so a slot means something only on the device that wrote it |
| 24-27 | 4 | reserved | always 0 |
| 28-31 | 4 | layout version | 1 |

- Every enum fits its field. The daypart and the weekday type use every code their field has; a new value of either
  needs a new layout version. The weather condition has 6 of 7 codes used, the activity 9 of 15.
- **The all-unknown key is the raw value 0** (no version bits). It is also what a play stores when its context was
  never captured: nothing known is one value, never a guess. A key with at least one known field carries version 1.
  Readers accept `0` and version 1 and reject any other version, a non-zero reserved nibble or a code with no
  meaning.
- The hierarchical back-off above drops fields by masking them to 0 (`place` first, then `weather` and `device`),
  so a coarse bucket is the same key with those fields cleared.

## 4. Similarity

Each track gets a normalized vector:
- metadata: genre multi-hot (hashed to 64 dims), decade, artist embedding = mean of co-play neighbors
- audio: bpm (log), energy, brightness, onset_density, vocal_presence, mode, loudness_range
- behavioral: co-occurrence in the same session within 3 tracks (item-item PMI, sparse, top-50 per track)

Cosine similarity, top-20 neighbors per track precomputed incrementally when idle and stored as
`u32` id + `u8` quantized similarity (~100 bytes per track). Vectors themselves are recomputed from
features, not persisted. No neural model.

## 5. Patterns we detect (and show in Insights)

- **Weekly rhythm**: per (weekday, hour) the dominant genres/energy. "Friday 18:00 → high energy."
- **Weather taste**: lift of genres/energy per weather bucket vs baseline. "Rain → +40 % acoustic."
- **Daylight taste**: brightness/energy vs `mins_to_sunset` and `daylight_minutes` (winter darkness effect).
- **Anniversaries**: tracks with a strong play peak exactly 1–3 years ago today (± 3 days, from play
  records), and "this month N years ago" beyond that (from monthly rollups).
- **Obsession phases**: repeat-one + high `now`; predicts burnout — radio rests the track before skips start.
- **Hook preference**: users who seek to choruses ⇒ hook starts enabled automatically.
- **Intro intolerance**: per artist/genre from `SeekForward` over intro.
- **Preferred loudness**: per track/genre/context from `PerceivedLevel`.
- **Wishlist**: frequent `SearchNoResult` queries.

## 6. Final score

```
score(track | ctx) =
    0.35 · context_affinity(track, ctx)        // §3, Thompson-sampled in radio
  + 0.20 · decayed_affinity(track, mode)       // §2, mix of now/season/alltime by radio mode
  + 0.20 · similarity_to_session_seed(track)   // §4
  + 0.10 · artist/album affinity
  + 0.10 · novelty bonus (discovery modes)
  − repetition_penalty(last_played, plays_today)
  − fatigue_penalty(session_skip_rate, track energy)
```

Weights per radio mode (Familiar / Discovery / Diverse / Balanced) live in `weights.rs`.

## 7. Explanations

Every scored candidate keeps its top 2 contributing terms and renders them through templates
(localizable, golden-tested):

- "Rainy Tuesday evening — you've played this 6× in weather like this."
- "You turn this up at the chorus."
- "A year ago today this was on repeat."
- "Your coding music: low vocals, 118 BPM."
- "Because you loved *Album X* — same producer."
- "Something new: similar energy to what you're playing, never heard."

No explanation ⇒ the candidate is not allowed into the queue.
