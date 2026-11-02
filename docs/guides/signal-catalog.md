# Signal catalog

Every signal Tinwave collects. This file is the source of truth: if a signal is not here, it must not exist
in code. Tiers are defined in [privacy-and-signals.md](../rules/privacy-and-signals.md).
Weights are starting values; the live table is `crates/tinwave-taste/src/weights.rs` (see [taste-engine.md](taste-engine.md)).

Every event automatically carries: `(device, seq)`, `ts`, `session` (which holds device and timezone),
`recording`, `source`, `pos_ds` (deciseconds), `ctx` (context snapshot). Payloads below list only the extra
fields; they are stored as tiny packed, quantized structs (see [storage-budget.md](../rules/storage-budget.md)).

**Storage class** of each signal: **E** = raw event (hot 90 days, then folded into play records/rollups),
**C** = counter only (never a raw event), **D** = derived (computed, not stored per occurrence).
A derived signal's privacy tier is the **highest tier among its inputs** (e.g. `PerceivedLevel` is T1 because it
uses the output device's gain); the code enforces this through `SignalKind::derived_inputs()`.

Common enums:
- `InputSource`: `Mouse | Keyboard | MediaKey | Tray | MiniPlayer | Remote(device_id) | Auto | System`
- `Section`: from [audio analysis](audio-analysis.md) — `Intro | Verse | Build | Chorus | Drop | Bridge | Breakdown | Outro | Unknown`, plus `section_index`

Flag sets (`RadioFeedback.context_scope`, `MetadataEdited.fields`) and every enum code are frozen by name in
`crates/tinwave-signals/src/frozen.rs`; a retired code is listed there and never reused.

---

## 1. Volume — "what do they turn up?"

| Signal | Tier | Trigger | Payload | Meaning |
|---|---|---|---|---|
| `VolumeChanged` | T0 | In-app volume gesture ends | `from`, `to` (linear 0–1), `gesture_ms`, `input`, `section`, `ms_since_track_start`, `ms_since_section_start` | Up early (< 15 s) ⇒ strong like (+0.5). Up at a section ⇒ hook marker for that section. Down ⇒ weak dislike **only** if context is neutral. |
| `SystemVolumeChanged` | T0 | OS master/endpoint volume gesture ends while Tinwave plays (400 ms trailing coalescing — the OS sends one event per step) | same as above + `endpoint_class`; `input` = `System` on Windows (keys/flyout/mixer not distinguishable, ADR 0004) | Most users use the keyboard volume keys, not our slider. Windows `IAudioEndpointVolume::RegisterControlChangeNotify` with Tinwave's event-context GUID to drop its own changes (ADR 0004), macOS `kAudioDevicePropertyVolumeScalar` listener, Linux PipeWire/PulseAudio sink events. |
| `MuteToggled` | T0 | Mute on/off (app or system); system mute is derived from the same OS notification as `SystemVolumeChanged` (mute changed, scalar unchanged) | `muted`, `input` | Mute + resume within 60 s ⇒ interruption, not dislike. |
| `PerceivedLevel` | derived | Computed per play | `effective_db = 20·log10(volume) + endpoint_gain_db − track_lufs + replaygain_db` | Learns **preferred listening loudness** per track, artist, genre, context. Feeds Auto-loudness memory. |

Interpretation rules:
- Volume down within 3 s of a `VideoCall` foreground category, notification sound, or `OutputDeviceChanged` ⇒ context, weight 0.
- Several volume-ups across plays at the same section ⇒ promote section to **"your hook"** for that track.
- Volume consistently higher in context X (e.g. evenings, speakers) ⇒ context loudness profile, not track preference.

## 2. Speed — "what do they rush, what do they savor?"

Tinwave offers playback rate 0.75×–1.5× with pitch preservation (license check: prefer an MIT/BSD
time-stretch such as Signalsmith Stretch; avoid GPL libraries unless an ADR accepts it).

| Signal | Tier | Trigger | Payload | Meaning |
|---|---|---|---|---|
| `RateChanged` | T0 | Rate gesture ends | `from`, `to`, `input`, `section` | > 1.0 ⇒ impatience with this track (−0.2) unless context is Workout-like. < 1.0 ⇒ studying/learning (neutral, flags "practice track"). |
| `RateSession` | derived | Per play | `time_weighted_rate` | Tracks always played fast ⇒ candidates for "radio edit" starts (skip intro). |

## 3. Seek, skip, replay — "where is the good part?"

| Signal | Tier | Trigger | Payload | Meaning |
|---|---|---|---|---|
| `Skipped` | T0 | Next / track change before end | `pos_ds`, `fraction`, `input`, `section`, `queue_origin` (`Manual|Radio|Album|Playlist|Shuffle`) | < 5 s: −1.0 · 5–30 s: −0.6 · 30 s–50 %: −0.2 · > 80 %: +0.2. Skips of radio picks also train radio. |
| `Completed` | T0 | Reached end (or within last 3 %) | `listened_ms`, `paused_ms` | +1.0 |
| `SeekForward` | T0 | Seek gesture whose jumps add up to > 2 s forward (playback between merged seeks does not count) | `from_ms`, `to_ms`, `section_from`, `section_to`, `input` | Over intro ⇒ intro-intolerance for this track/artist. Mid-track ⇒ skipping a section (that section −). |
| `SeekBack` | T0 | Seek gesture whose jumps add up to > 2 s backward. A merged gesture whose net movement disagrees with its jumps (playback outran small back-nudges) is suppressed and counted, so `SeekBack` always has `to < from` and `SeekForward` `to > from` | same | Replay of a segment ⇒ +0.6 and adds to the **replay heatmap** (64 bins per track). |
| `Replayed` | T0 | Previous / restart within 30 s of finishing, or repeat-one loop | `loop_count` | +0.8 per loop, diminishing. |
| `RepeatModeChanged` | T0 | Repeat off/all/one | `mode` (`Off|All|One`) | Repeat-one on a track ⇒ obsession phase marker. |
| `Paused` / `Resumed` | T0 | Pause/resume | `pause_ms` (on resume), `input` | Long pause then resume same track ⇒ neutral. Pause then quit ⇒ session end. |
| `PreviousPressed` | T0 | Previous track | `pos_ds` | Going back to the *previous* track ⇒ +0.5 for that track. |

## 4. Queue & library behaviour — "intent before listening"

| Signal | Tier | Trigger | Payload | Meaning |
|---|---|---|---|---|
| `QueuedNext` / `QueuedLater` | T0 | Play next / add to queue | `from_view` (`Home|Library|Album|Artist|Playlist|Search|Queue|Radio|NowPlaying|Lyrics|MiniPlayer|Insights`) | Explicit intent +0.4 |
| `RemovedFromQueue` | T0 | Removed before it played | `was_radio_pick` | Pre-listen rejection −0.4 (strong for radio picks) |
| `QueueReordered` | T0 | Drag in queue | `distance` (signed) | Moved earlier ⇒ eager (+0.2) |
| `PlayStarted` | T0 | Any play start | `origin` (`Search|Album|Artist|Playlist|Radio|Queue|Resume|Remote`), `input` | Manual plays weigh more than radio/shuffle plays. |
| `Favorited` / `Unfavorited` | T0 | Heart toggle | — | +2.0 / −1.0 |
| `Rated` | T0 | Rating set | `stars` | Mapped −2…+2 |
| `PlaylistAdded` / `PlaylistRemoved` | T0 | Edit playlist | `playlist_id`, `playlist_kind` (`Manual|Smart`) | +1.0 / −0.3; playlist names become user-defined "moods". |
| `MetadataEdited` | T0 | Tag edit | `fields` (names only) | Caring signal +0.3; genre edits also feed similarity. |
| `ArtworkChanged` | T0 | Artwork edit | — | Caring +0.2 |
| `SearchPerformed` | T0 | Search submitted / settled | `query_len`, `query_hash`, `result_count`, `scope` (`All|Tracks|Albums|Artists|Playlists`) | Query text stored locally for search history only if user enables it (default on, T0). |
| `SearchResultOpened` | T0 | Result clicked | `rank`, `entity_kind` (`Track|Album|Artist|Playlist`) | Intent; low-rank clicks reveal ranking misses. |
| `SearchNoResult` | T0 | 0 results | `query` | **Wishlist**: music the user wants but does not own. Shown in Insights. |

## 5. Browsing & attention — "what catches their eye?"

`PageDwell`, `ArtworkHover` and `Impression` are storage class **C**: aggregated in memory into daily
`ui_counter` rows per entity. They fire too often to store individually.

| Signal | Tier | Trigger | Payload | Meaning |
|---|---|---|---|---|
| `PageDwell` | T0 | Leave an album/artist/playlist page | `entity`, `dwell_ms`, `scrolled_fraction` | Long dwell without play ⇒ curiosity candidate for discovery radio. |
| `ArtworkHover` | T0 | Hover ≥ 800 ms | `entity`, `hover_ms` | Weak interest +0.05 |
| `Impression` | T0 | Item visible ≥ 500 ms in a settled list | `entity`, `list` | Many impressions, no clicks ⇒ fatigue (−0.02 each, capped). |
| `ViewOpened` | T0 | Lyrics, Now Playing fullscreen, mini-player, Insights | `view` (`Home|Library|Album|Artist|Playlist|Search|Queue|Radio|NowPlaying|Lyrics|MiniPlayer|Insights`) | Lyrics open ⇒ singing-along/engaged (+0.2). Mini-player ⇒ background listening. |
| `WindowState` | T1 | App focused / unfocused / minimized / tray | `state` (`Focused|Unfocused|Minimized|Tray`) | Foreground vs background listening; skip in background weighs more (user bothered to switch). |

## 6. Output & sound — "how do they listen?"

| Signal | Tier | Trigger | Payload | Meaning |
|---|---|---|---|---|
| `OutputDeviceChanged` | T1 | Default or chosen device changes | `class` (`Headphones|Speakers|Bluetooth|UsbDac|Hdmi|Builtin|Virtual|Unknown`; `UsbDac` = external DAC, USB or digital out), `transport` (`Bluetooth|Usb|Hda|Virtual|Other`), `form_factor` (OS enum, `u8`), `device_hash` | Splits taste by device: headphone music ≠ speaker music. Class = f(transport, form factor, internal jack) — rule order in ADR 0004. |
| `EqChanged` | T0 | EQ preset/band change | `preset`, `bass_db`, `treble_db` | Bass boost on a track/genre ⇒ energy preference. |
| `ExclusiveModeToggled` | T0 | WASAPI exclusive / bit-perfect | `on` | Audiophile session: favor lossless sources. |
| `CrossfadeChanged` | T0 | Crossfade/gapless setting | `ms` | Party vs album listening style. |

## 7. Context — "when, where, in what state?"

Captured by [context providers](context-engine.md) into a `ContextSnapshot`, deduplicated, and linked from events.

| Field | Tier | Source | Used for |
|---|---|---|---|
| `local_hour`, `daypart` (`Night|EarlyMorning|Morning|Midday|Afternoon|Evening|LateNight`), `weekday`, `is_weekend` | T1 | Clock | Weekly rhythms ("Friday 18:00") |
| `holiday` (name id), `days_to_holiday` | T1 | Offline holiday dataset by user-chosen country | Seasonal/holiday radio |
| `season`, `month` | T1 | Clock + hemisphere | Summer songs, winter songs |
| `sun_altitude_deg`, `is_daylight`, `mins_since_sunrise`, `mins_to_sunset`, `daylight_minutes` | T1 | Offline solar calc from coarse location (user-entered city or OS location rounded) | Sunset radio, dark-winter-evening taste |
| `weather` (`temp_c`, `feels_like_c`, `precip_mm`, `cloud_pct`, `wind_kmh`, `condition`, `is_day`) | T3 | Open-Meteo, hourly cache | Rainy-day / heatwave taste |
| `power` (`Ac|Battery`), `battery_pct` | T1 | OS power API | On-the-move vs desk listening; also gates analysis |
| `idle_s`, `screen_locked` | T1 | OS idle API (no hooks) | Active vs background listening |
| `fullscreen_app`, `focus_mode` | T1 | OS | Gaming / focus sessions |
| `app_category` (`Ide|Office|Browser|Game|VideoCall|Creative|Chat|Media|Other`) | T2 | Foreground exe/bundle id → local category map | Coding music, gaming music, call interruptions |
| `place` (`PlaceA|PlaceB|…` or user label `Home|Work|…`) | T2 | Salted hash of SSID + gateway MAC | Home vs office taste |
| `calendar_busy` | T2 | OS calendar, busy flag only (later phase) | Pre-meeting focus, post-meeting release |
| `session_minutes`, `session_tracks`, `session_skip_rate` | T0 | Session tracker | Fatigue detection |

### Mobile-ready context (schema defined now, collected in Phase 4+)

`motion` (`Still|Walking|Running|Cycling|Driving`), `cadence_spm`, `speed_kmh`, `headphones_plugged`,
`heart_rate_bpm` (HealthKit / Health Connect, opt-in T2). Desktop models must accept these fields as
`Unknown` today so mobile data plugs in unchanged.

## 8. Session & meta signals

| Signal | Tier | Trigger | Payload | Meaning |
|---|---|---|---|---|
| `SessionStarted` / `SessionEnded` | T0 | First play after ≥ 30 min silence / 30 min silence | `start_origin`, `tracks`, `minutes` | Session-level modeling (first manual pick = session mood). |
| `RadioFeedback` | T0 | "Not now", "Never in this context", "More like this", "Forget this play" | `kind` (`NotNow|NeverInContext|MoreLikeThis|ForgetThisPlay`), `context_scope` | Direct corrections, strongest weights. |
| `ExplanationOpened` | T0 | User opens "Why this?" | `reason_kind` | Which explanations people care about. |
| `SignalDropped` | meta | Channel overflow | `kind`, `count` | Health metric shown in Diagnostics (in memory only). |

## 9. Volume estimates (reference heavy user: 8 h/day, ~120 tracks/day)

| Group | Class | Events/day | Bytes/day on disk |
|---|---|---|---|
| Play start/end, skips, completes | E | ~240 | ~14 KB |
| Volume, mute, speed (coalesced gestures) | E | ~60 | ~4 KB |
| Seeks, replays, previous, repeat | E | ~60 | ~4 KB |
| Queue, favorites, ratings, playlists, edits | E | ~40 | ~2.5 KB |
| Search | E | ~20 | ~1.5 KB |
| Output/EQ/window state changes | E | ~30 | ~2 KB |
| Context snapshots (deduped, quantized) | — | ~60 | ~2 KB |
| UI counters (dwell, hover, impressions) | C | ~300 rows | ~5 KB |
| Play records (warm tier) | — | ~120 | ~6 KB |
| **Total** | | | **~40 KB/day ≈ 3.6 MB per 90-day hot window** |

After 90 days raw events are deleted; only play records (~2 MB/year) and monthly rollups (~0.5 MB/year)
remain. Every new signal must add its estimate to this table.
