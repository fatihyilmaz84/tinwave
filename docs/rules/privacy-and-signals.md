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
- No telemetry, no analytics SDKs, no crash reporters that upload by default (spec §15).
- Never put signal data, recording names or file paths in logs above `debug`, URLs, or crash reports.
- Hashing for places: `blake3(device_secret || ssid || gateway_mac)`, `device_secret` in the OS keychain.

## Interpretation ethics

- Context first: a negative-looking action during an interrupting context (call, notification, lock screen,
  device change) is not scored as dislike.
- Insights are for the user only and phrased neutrally. No mood or health diagnoses
  ("you seem depressed") — describe music, not the person.
- Users can correct the engine: "Not now", "Never in this context", "Forget this play".
