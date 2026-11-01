# Context engine

The context engine answers "what is the listener's situation right now?" cheaply, offline-first and
without ever blocking playback. Lives in `tinwave-context`; trait in `tinwave-signals`.

## Trait

```rust
pub trait ContextProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn tier(&self) -> Tier;
    /// How stale a cached value may be before it is refreshed.
    fn max_age(&self) -> Duration;
    /// Must return in < 200 µs. Return cached values; refresh off-thread.
    fn snapshot(&self, out: &mut ContextFields);
    fn capability(&self) -> Capability; // Supported | Unsupported(reason) | NeedsPermission
}
```

- `ContextHub` holds all enabled providers. On every T0 event it builds `ContextFields`, hashes them,
  and reuses the previous snapshot id (`ctx`) if the hash is unchanged.
- Providers that need IO or OS calls (weather, foreground app, network place) refresh on a background
  thread/timer and expose an `ArcSwap` cache. `snapshot()` only reads.
- Disabled tier ⇒ provider is not constructed at all.
- Every missing value is explicit `Unknown`, never a guess.
- Fields are **quantized before hashing** (temperature to whole °C, sun altitude to 5°, battery to 10 %,
  idle to buckets 0/60/300/900 s). Without this every snapshot is unique and dedupe does nothing.
  Target: ≤ ~60 distinct snapshots per day, ~30 bytes each.

## Providers and per-OS APIs

| Provider | Tier | Windows | macOS | Linux | Refresh |
|---|---|---|---|---|---|
| Clock / daypart / weekday | T1 | `time` crate + IANA tz | same | same | per event |
| Holidays | T1 | Bundled dataset (e.g. generated from `holidays` data, license-checked) | same | same | daily |
| Sun position | T1 | Offline NOAA solar algorithm from coarse lat/lon | same | same | 5 min |
| Coarse location | T1/T3 | User-entered city (default) or `Windows.Devices.Geolocation` rounded to 0.1° | CoreLocation (requires permission) | GeoClue2 | daily |
| Power | T1 | `GetSystemPowerStatus` | IOKit `IOPSCopyPowerSourcesInfo` | UPower D-Bus | 1 min + change notifications |
| Idle / lock | T1 | `GetLastInputInfo`, `WTSRegisterSessionNotification` | `CGEventSourceSecondsSinceLastEventType`, `com.apple.screenIsLocked` | `org.freedesktop.ScreenSaver` / `ext-idle-notify-v1` / XScreenSaver | 10 s |
| Output device | T1 | `IMMDeviceEnumerator` + `IMMNotificationClient`; form factor, transport via `IDeviceTopology`, internal jack via `IKsJackDescription`; re-bind on default change (ADR 0004) | CoreAudio default-device listener, transport type | PipeWire/PulseAudio sink props | event-driven |
| System volume | T0 | `IAudioEndpointVolumeCallback` on the bound render endpoint, own changes dropped by event-context GUID, 400 ms coalescing (ADR 0004) | `kAudioDevicePropertyVolumeScalar` listener | PipeWire/Pulse sink volume events | event-driven |
| Fullscreen / focus mode | T1 | `SHQueryUserNotificationState` | Focus status API (if available) else Unknown | Unknown (Phase 0 decision) | 30 s |
| Foreground app category | T2 | `GetForegroundWindow` → process image name only | `NSWorkspace.frontmostApplication.bundleIdentifier` | X11 `_NET_ACTIVE_WINDOW` → `WM_CLASS`; **Wayland: Unknown** | 5 s |
| Network place | T2 | `WlanQueryInterface` SSID + gateway MAC via `GetIpNetTable2` | CoreWLAN (needs location permission on recent macOS — otherwise Unknown) | NetworkManager D-Bus | on network change |
| Weather | T3 | HTTPS to Open-Meteo | same | same | 60 min, on wake |

### Foreground-app categories

`category_map.toml` ships with the app: exe/bundle id → category (`code.exe`, `idea64.exe` → `Ide`;
`zoom.exe`, `Teams.exe`, `com.apple.FaceTime` → `VideoCall`; Steam game dirs / fullscreen DirectX → `Game`…).
Only the **category** is stored. The exe name is used in memory and dropped. Users can override mappings
in Settings. Window titles are never read.

### Weather provider

- Default provider: Open-Meteo forecast API (no key). Behind a `WeatherProvider` trait so it is replaceable
  (verify commercial-use terms before release; spec §15).
- Request contains only `latitude`/`longitude` rounded to 1 decimal and the requested variables.
  The Signal Center shows the exact URL template.
- Cache hourly forecast for 48 h → weather context keeps working offline for two days.
- Map to `WeatherBucket`: `Clear | Cloudy | Rain | Storm | Snow | Fog` × `Cold(<5) | Cool(5–15) | Mild(15–24) | Warm(24–30) | Hot(>30)`.

## Derived context

`ActivityGuess` (deterministic rules, explainable, no ML):

| Guess | Rule |
|---|---|
| `Focus` | `app_category ∈ {Ide, Office, Creative}` and `idle_s < 60` |
| `Gaming` | `app_category = Game` or fullscreen game |
| `Call` | `app_category = VideoCall` |
| `Background` | `idle_s > 300` or `screen_locked` |
| `OnTheMove` | `power = Battery` and place changed today |
| `Leisure` | otherwise, weekend or evening |

Mobile will add `Workout`, `Commute`, `Driving` from motion data.
