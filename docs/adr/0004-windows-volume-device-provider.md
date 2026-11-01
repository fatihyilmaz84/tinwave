# ADR 0004 — Windows system-volume and output-device provider (WASAPI MMDevice notifications)

- **Status:** Accepted (lead, from spike T-0008; report: work/reports/T-0008/spike-1.md)
- **Date:** 2026-11-01

## Context

`SystemVolumeChanged` (T0), `MuteToggled` (T0) and `OutputDeviceChanged` (T1) need OS-level notifications on Windows
(D-0001: Windows first). Requirements: event-driven (no polling), negligible idle cost, no elevation, no global
input hooks ([privacy rules](../rules/privacy-and-signals.md)), tell our own changes from the user's,
and classify the output device into `Headphones | Speakers | Bluetooth | UsbDac | Hdmi | Builtin`.

Spike evidence (all on Windows 11 Pro 10.0.26200, Rust 1.92, `windows` 0.62.2, non-elevated; desktop PC with Realtek HDA
line-out, NVIDIA/AMD HDMI/DP audio, Steam virtual audio, paired but disconnected Sony WH-1000XM4):

| Measurement | Result |
|---|---|
| Notification latency, in-process (call -> callback) | 1.3 - 1.8 ms |
| Notification latency, other process (child's pre-call QPC -> our callback), 5 trials | min 1.15 / median 1.23 / max 1.41 ms |
| Delivery under a burst (60 sets, 2 ms apart, ~200 ms) | 60 / 60 delivered, in order, **not coalesced by the OS** |
| Own-vs-external classification | own GUID -> `self`; foreign GUID and null GUID -> `external`, 100 % in 8 tests (A, B, C, 5 cross-process trials) |
| Mute | arrives in the same notification struct as volume (`bMuted`), volume field unchanged |
| Per-app session volume (`ISimpleAudioVolume`) | **0** endpoint notifications for 3 changes |
| Idle cost, 90 s, registered (enumerator + endpoint-notification client + one `RegisterControlChangeNotify`) | 0.000 ms CPU (both self-measured via `GetProcessTimes` and externally via `Get-Process` over 70 s), 0 events |
| Memory | working set 6.6 MiB before COM -> 9.6 MiB after registration (+3.0 MiB WS, +0.3 MiB private); 4 threads, 181 handles; no growth over 90 s |
| Registration time | 7.0 ms (cold, includes `CoCreateInstance` of the enumerator) |
| Binary | 378 KiB release (windows crate, LTO, strip) |
| Privileges | process token not elevated; no window, no message pump, no hooks (`grep` for `SetWindowsHookEx`/raw input: none) |

## Decision

Implement the Windows side of the `Output device` and `System volume` providers as follows.

### 1. API surface (no hooks, no elevation, no driver)

- A dedicated provider thread initialises COM as MTA and owns all COM objects.
- `IMMDeviceEnumerator::RegisterEndpointNotificationCallback(IMMNotificationClient)` for device events.
- One `IAudioEndpointVolume::RegisterControlChangeNotify(IAudioEndpointVolumeCallback)` on the **render endpoint
  Tinwave is currently outputting to** (default `eRender/eMultimedia`, or the user's chosen device).
- Callbacks run on RPC worker threads and do only: read `QueryPerformanceCounter`, copy the payload, `send` on a channel.
  **No COM calls inside callbacks** (re-binding, property reads and `Unregister*` happen on the provider thread; calling
  MMDevice API from inside `IMMNotificationClient` can deadlock).
- `ContextProvider::snapshot()` reads an `ArcSwap` cache (device class, volume, muted); it never touches COM.

### 2. Who changed the volume (self vs. user)

- Tinwave owns one process-wide event-context GUID. Any time Tinwave touches *endpoint* volume/mute it passes it as
  `pEventContext`; the callback compares `guidEventContext` and drops matches.
- Tinwave's own in-app volume (`VolumeChanged`) should use the **session** (`ISimpleAudioVolume`) or software gain.
  Spike test G shows session changes never reach the endpoint callback, so the two signals cannot alias.
- Everything else is `external`. The keyboard keys, the flyout and the mixer cannot be told apart by this API alone
  (the `input` field of `SystemVolumeChanged` is `System`). Whether the shell passes a distinguishing GUID is
  unverified: PO-RUNBOOK R1/R3 asks for the GUIDs; if they differ it is a free `input` refinement.

### 3. One notification, two signals

The notification struct carries master scalar + mute together. The consumer keeps the last `(volume, muted)` per endpoint
and emits `MuteToggled` when only `muted` changed, `SystemVolumeChanged` when the scalar changed (both when both did).
Because `fMasterVolume` is the slider scalar (0-1, 51 hardware steps on the dev endpoint, values like
`0.070000015`), quantize to 1 % before storing. For `PerceivedLevel.endpoint_gain_db` read
`GetMasterVolumeLevel()` / `GetVolumeRange()` on the provider thread (dev endpoint: -65.25..0 dB, 0.75 dB inc).

### 4. Gesture coalescing is Tinwave's job

Windows delivers one callback per set (no OS-side coalescing; the 60-set burst gave 60 events in ~200 ms). Slider drags and
held keys therefore produce 10s of events per second. The provider must coalesce with a trailing debounce
(proposal: 400 ms of silence ends a gesture; `from` = value before the first event, `to` = last, `gesture_ms`
= first-to-last), then emit one `SystemVolumeChanged`. This is mandatory for the storage budget.
The "while Tinwave plays" gate lives in the consumer (playback state), but the provider keeps tracking volume while idle
so `from` is always right.

### 5. Device re-binding (`OutputDeviceChanged`)

- `OnDefaultDeviceChanged` fires once **per role** (console / multimedia / communications). Act only on
  `(eRender, eMultimedia)` (or whichever role Tinwave opens its stream with) and ignore repeats whose id equals the bound id.
- On change: unregister from the old `IAudioEndpointVolume` (ignore errors, the device may be gone), activate and register
  on the new one, re-describe the device, emit `OutputDeviceChanged`. If Tinwave plays on a user-selected non-default device,
  bind to that device instead and use `OnDeviceStateChanged/OnDeviceRemoved` for it.
- Watch `OnDeviceStateChanged` for the bound id (UNPLUGGED/NOTPRESENT) to emit a change to "none/fallback".
- `OnPropertyValueChanged` is high-volume noise; ignore it (count only, if at all).
- Device identity: `device_hash = blake3(device_secret || endpoint_id)`; never store the friendly name or container id.
  (The `ContainerId` groups the A2DP and hands-free endpoints of one headset, useful in memory for dedup only.)

### 6. Device classification (class = f(transport, form factor, internal-jack))

Inputs (all readable without elevation):
- `PKEY_AudioEndpoint_FormFactor` ({1da5d803-d492-4edd-8c23-e0c0ffee7f0e}, 0) from the endpoint property store.
- Transport: `IDeviceTopology -> GetConnector(0) -> GetDeviceIdConnectedTo()` returns the adapter's interface path;
  its first segment is the enumerator: `bthenum`/`bthhfenum` (Bluetooth A2DP / hands-free), `usb`, `hdaudio` (onboard codec
  **and** GPU HDMI/DP audio), `root`/`swd` (virtual drivers). `PKEY_Device_EnumeratorName` on the endpoint is useless (always `MMDEVAPI`).
- Internal vs. external: `IKsJackDescription` on the adapter-side connector (`ConnectionType`, `GeoLocation`, `GenLocation`).
  Internal if `GenLocation = Internal` or `ConnectionType = AtapiInternal` or `GeoLocation = InsideMobileLid`.

Rule order (first match wins): transport Bluetooth -> `Bluetooth`; transport USB -> `UsbDac`; transport virtual -> `Virtual`;
form factor `DigitalAudioDisplayDevice` -> `Hdmi`; `SPDIF`/`UnknownDigitalPassthrough` -> `UsbDac` (external digital sink: the class means "external DAC", USB or digital out);
`Headphones`/`Headset`/`Handset` -> `Headphones`; `Speakers`/`LineLevel` -> `Builtin` if HDAUDIO and internal jack, else `Speakers`;
anything else -> `Unknown`.

Mapping table. "Measured" = observed on the dev PC through this prototype; the rest is derived from the documented enums
and is **unverified — runbook provided**.

| Device | FormFactor | Transport (adapter path prefix) | Jack | -> class | Status |
|---|---|---|---|---|---|
| Realtek line-out "Luidsprekers" (desktop, active) | Speakers | `hdaudio#func_01&ven_10ec` | 3.5 mm, Rear, PrimaryBox | `Speakers` | measured |
| Realtek front/rear headphone jack "Headphones" (not present) | Headphones | `hdaudio` | 3.5 mm, Front, PrimaryBox | `Headphones` | measured |
| Monitor over DisplayPort/HDMI "Odyssey G70B" (NVIDIA HDA, active) | DigitalAudioDisplayDevice | `hdaudio#...ven_10de` | OtherDigital, HDMI geo | `Hdmi` | measured |
| AMD/other GPU HDMI outputs (not present) | DigitalAudioDisplayDevice | `hdaudio#...ven_1002` | - | `Hdmi` | measured (not present) |
| Steam Streaming Speakers (virtual driver, active) | Speakers | `root#steamstreamingspeakers` | none | `Virtual` | measured |
| Sony WH-1000XM4 A2DP "Hoofdtelefoon" (paired, disconnected) | Headphones | `bthenum#{0000110b-...}` (A2DP sink) | OtherDigital, N/A, Other | `Bluetooth` | measured (state UNPLUGGED) |
| Sony WH-1000XM4 hands-free "Headset" (capture endpoint, disconnected) | Headset | `bthhfenum#bthhfpaudio` | - | `Bluetooth` | measured (state UNPLUGGED) |
| USB webcam microphone (capture, shows USB path parse) | Microphone | `usb#vid_046d&pid_085c` | - | `UsbDac` (render-only rule: capture endpoints are not classified) | measured |
| USB DAC / USB headset (render) | Speakers / Headphones | `usb#...` | - | `UsbDac` | unverified — runbook R8 |
| Laptop internal speakers | Speakers | `hdaudio` | expected Internal | `Builtin` | unverified — runbook R9 |
| Laptop 3.5 mm headphones | Headphones | `hdaudio` | 3.5 mm | `Headphones` | unverified — runbook R9 |
| Optical/S-PDIF out | SPDIF | `hdaudio`/`usb` | Optical | `UsbDac` (judgement) | unverified |
| Bluetooth speaker (A2DP) | Speakers/Headphones | `bthenum` | - | `Bluetooth` | unverified (same rule as WH-1000XM4) |
| Remote Desktop / cast | RemoteNetworkDevice | `swd`/`ts*` | - | `Unknown` | unverified |

Device states seen: `ACTIVE`, `UNPLUGGED` (paired BT / unplugged jack), `NOTPRESENT` (e.g. ~25 historic HDMI endpoints
on this PC), `DISABLED`. Only `ACTIVE` endpoints can be default; enumerate with `DEVICE_STATE_ACTIVE`, not the
all-states mask the prototype uses for exploration.

### 7. Capability

`capability()` is always `Supported` on Windows 10/11 (the API is user-mode COM, ships with Windows, works for a standard user);
never `NeedsPermission`. Failure to create the enumerator or to bind (no render device at all, e.g. RDP session without audio)
yields `Unknown` fields, not an error to the user.

## Consequences

Easier
- Truly event-driven: no polling timers, 0 CPU idle, 1-2 ms latency, ~3 MiB extra working set.
- No privilege or permission prompts; no keyboard/mouse hooks, so the privacy rules hold and the "no global input hooks"
  line in the Signal Center can stay literally true.
- Self-vs-user attribution is exact when Tinwave passes its GUID.

Harder / to maintain
- Tinwave must coalesce gestures itself and diff mute vs. volume itself.
- The device classifier needs the unverified laptop/USB cases confirmed (runbook) and a table of exceptions will grow
  (virtual cables, GPU audio, docks that present as USB-audio with speaker form factor).
- A one-class enum is lossy: a USB headset is both `Headphones` and `UsbDac`, a BT headset is both `Headphones` and `Bluetooth`.
  Proposal: keep `class` for the catalog, but also keep transport and form factor as separate quantized fields in memory so
  taste features ("headphone-ness" vs. "link") can be derived later without a schema change.
- Endpoint volume is not the whole story for loudness: some devices (HDMI/DP sinks, some BT absolute-volume headsets) apply
  their own gain that Windows cannot see. `PerceivedLevel` should treat `Hdmi` and `Bluetooth` `endpoint_gain_db` as uncertain.
- The `IMMNotificationClient` events are chatty (3 per default switch, many property changes) and the provider needs a small
  state machine with debounce; the audio service restarting is not handled by the prototype (runbook R11).

Not verified by the spike (needs a human, see PO-RUNBOOK.md): real keyboard keys, flyout drag, BT connect/disconnect,
default-device switch and the re-bind path, laptop internal-speaker detection, USB DAC, sleep/wake survival, audio-service restart.

## Lead decisions on the spike's open questions (2026-11-01)

1. **Class plus separate fields.** `OutputDeviceChanged` carries the derived `class` **and** `transport`
   (`Bluetooth | Usb | Hda | Virtual | Other`) and `form_factor` (OS form-factor enum, quantized to `u8`) as separate
   fields, so taste features can use "headphone-ness" and "link" independently without a later schema change.
   Same tier (T1); a few bytes per event, a handful of events per day.
2. **Class enum** gains `Virtual` and `Unknown`. `UsbDac` means "external DAC" — USB **or** S/PDIF/digital passthrough.
3. **Coalescing window** for `SystemVolumeChanged` is a 400 ms trailing window, matching the spike evidence
   (OS delivers one event per set). It reuses the `GestureCoalescer` from T-0005, configured per kind.
4. **`input`** of `SystemVolumeChanged` is `System` on Windows until PO-RUNBOOK R1/R3 shows the shell passes a
   distinguishing event-context GUID for keys vs. flyout.
5. Tinwave's in-app volume never writes endpoint volume (session volume or software gain only); this is a
   constraint on the E-01 playback-engine ticket.
6. The unverified rows (runbook R1–R12) must be run by the PO before the Windows provider ticket is accepted;
   the provider ticket's tests use fixture ids, not live hardware.

## Spec / docs updated

- `docs/guides/signal-catalog.md` §1 `SystemVolumeChanged` / `MuteToggled` (input, coalescing, derivation, APIs) and
  §6 `OutputDeviceChanged` (class enum, `transport`, `form_factor`, classification rule).
- `docs/guides/context-engine.md` provider table rows "Output device" and "System volume".
- Prototype, runbook and the original draft: `spikes/T-0008-win-volume/`.
