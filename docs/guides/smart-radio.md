# Smart Radio

Personal listening sessions built from the [taste engine](taste-engine.md), over the local library and
permitted server sources. Lives in `tinwave-radio`.

## Session engine

1. **Seed**: artist, album, genre, track, playlist ("mood"), or **"Right now"** (pure context).
2. **Candidate pool** (≤ 2 000): similar tracks to seed + top context-affinity tracks + discovery sample
   (never/rarely played, similarity ≥ threshold). Paginated DB reads, never the whole library.
3. **Score** with the context snapshot at pick time. Use Thompson sampling on context Betas so the
   radio keeps exploring.
4. **Constraints**: no artist twice in 4 tracks, no album twice in 8, no track repeated within the
   repetition window (mode-dependent), source available offline if the session is offline.
5. **Pick lookahead 3**, re-score after every signal (skip ⇒ immediately re-plan).
6. Each pick stores its explanation; radio skips/removals feed back as `RadioFeedback`.

## Modes (spec §10)

| Mode | decayed mix (now/season/alltime) | novelty | diversity |
|---|---|---|---|
| Familiar | 0.5 / 0.3 / 0.2 | low | low |
| Discovery | 0.2 / 0.3 / 0.5 | high | medium |
| Diverse | 0.3 / 0.3 / 0.4 | medium | high |
| Balanced | 0.35 / 0.35 / 0.3 | medium | medium |

## The crazy features

- **Auto-loudness memory** — Tinwave learns the volume you like for each track/genre in each context and
  glides to it at track start (opt-out). "Tinwave remembers you blast this one."
- **Hook starts** — in radio and previews, start tracks at your `hook_section` when you've shown intro
  intolerance for that artist/genre. Toggleable per session.
- **Energy arc** — sessions follow the learned energy trajectory for this context (e.g. Monday-morning ramp
  up, late-night wind down). Shown as a small curve in Now Playing.
- **Fatigue detector** — rising `session_skip_rate` or repeated volume-downs ⇒ shift energy/genre, or
  offer "Change of pace?".
- **Obsession care** — detects repeat-one phases and rests the track before burnout, then brings it back
  as a "comeback" weeks later.
- **Auto-stations** (generated daily when idle, shown on Home):
  - *Right Now* — pure context
  - *Weather Radio* — "Rainy Afternoon", "First Snow", "Heatwave" (T3 only)
  - *Sunset Radio* — starts ~45 min before sunset, follows the light
  - *Monday Morning*, *Friday Night*, *Sunday Slow* — from weekly rhythms
  - *Focus* — low vocals, steady BPM, from your coding/office sessions (T2)
  - *A Year Ago Today* — anniversaries
  - *Forgotten Favorites* — high all-time, zero now
  - *Wishlist-adjacent* — owned tracks closest to things you searched for but don't have
- **Insights ("Wrapped, anytime")** — local dashboard: hour × weekday heatmap, weather lift chart,
  most-turned-up songs and their sections, most-skipped intros, preferred loudness, taste drift timeline,
  obsession timeline, wishlist. Shareable as an image only on explicit user action.
- **Teach it** — "More like this", "Not now", "Never in this context", "Forget this play" on every
  radio track.

## Smart playlists

Rule-based (`SmartPlaylistRule`) and may reference signal aggregates:
`hook_section IS NOT NULL`, `rainy_lift > 1.3`, `now_score > x`, `context = Focus`, `skipped_intro_rate > 0.5`…
