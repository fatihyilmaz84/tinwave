SPIKE RESULT T-0008
answer: Yes. Use IAudioEndpointVolume::RegisterControlChangeNotify (bound to the active render endpoint) plus IMMNotificationClient. Both are event-driven, need no elevation and no input hooks, and cost about 0 CPU and 3 MiB extra working set. Tinwave must do gesture coalescing, the mute/volume diff and the default-device re-binding itself.
confidence: medium-high. Volume, mute, own-vs-external attribution, latency, overhead and enumeration/classification are measured. Everything that needs a human is unverified (see PO-RUNBOOK.md): keyboard keys, Bluetooth connect/disconnect, default-device switch and the re-bind path, laptop internal speakers, USB DAC, sleep/wake.

evidence:
- Measured on Windows 11 Pro 10.0.26200, Rust 1.92, windows crate 0.62.2, non-elevated desktop PC (Realtek HDA line-out, NVIDIA/AMD HDMI/DP audio, Steam virtual audio, paired but disconnected WH-1000XM4).
- The `selftest` command ends "ALL PASS". I ran it successfully 4 times. Its checks:
  - Own event-context GUID (A) is classified `self`.
  - A foreign GUID (B) and a null GUID (C) are classified `external`.
  - 5 cross-process trials (a child process sets the volume with a random GUID) are all `external`, one event each.
  - Mute on and off arrive in the same notification struct as volume.
  - `ISimpleAudioVolume` session volume/mute changes produce 0 endpoint events, so Tinwave's in-app volume cannot alias into `SystemVolumeChanged`.
  - Original volume and mute are restored every time.
- Latency is 1.3-1.8 ms in-process. Cross-process (child's pre-call QPC to our callback) was min 1.15 / median 1.23 / max 1.41 ms in one run and median 1.58 ms in a later run.
- A burst of 60 sets at 2 ms spacing gave 60/60 events in about 200 ms, in order. The OS does not coalesce.
- Idle overhead over 90 s: 0.000 ms CPU, measured both with GetProcessTimes and externally with Get-Process over 70 s. 0 events.
- Memory: working set 6.6 MiB before COM, 9.6 MiB after registration (+3.0 MiB), private +0.3 MiB, 4 threads, no growth. Registration took 7 ms. The release binary is 378 KiB.
- The endpoint exposes 51 hardware steps, -65.25..0 dB with 0.75 dB increments. `GetMasterVolumeLevel` is available for `endpoint_gain_db`.
- No elevation: `IsAdministrator=False`. No window, no message pump, no hooks. A grep for `SetWindowsHookEx`/raw input found nothing.
- Enumeration of all endpoints (render and capture, all states) is in the `enumerate` command. Key facts:
  - `PKEY_Device_EnumeratorName` is useless (always MMDEVAPI).
  - Transport comes from `IDeviceTopology` → connector 0 → `GetDeviceIdConnectedTo`. The first path segment is `bthenum`/`bthhfenum`, `usb`, `hdaudio`, or `root`/`swd` (virtual).
  - Form factor comes from `PKEY_AudioEndpoint_FormFactor`.
  - Internal vs. external comes from `IKsJackDescription`.
  - The desktop line-out reports FormFactor Speakers on HDAUDIO, the same as laptop speakers, so I added the jack-description check to avoid mislabelling it `Builtin`.
- Unit tests for the classifier pass (`cargo test`).

options compared:
- IAudioEndpointVolume + IMMNotificationClient (user-mode COM) — RECOMMENDED — event-driven, 1-2 ms, no privileges, exact self/external attribution via the event-context GUID.
- Polling the volume — rejected — a timer wakes the CPU and can miss or mis-time gestures; unnecessary since the callback works.
- Global keyboard hook for volume keys — rejected — violates the privacy rule "no global input hooks", and misses flyout/mixer/headset-button changes.
- IAudioSessionManager / ISimpleAudioVolume events — not suitable for this signal — they are per-app session volume, so they correctly do not fire on OS endpoint changes. Use them only for Tinwave's own volume.
- IPolicyConfig (undocumented) to switch the default device programmatically — not used — unsupported API; I did not change the user's default device, so that row goes to the runbook.

licenses: windows and windows-core 0.62.2 — MIT OR Apache-2.0 — no concern. The whole prototype tree is these two crates, 32 lines of `cargo tree`.

risks:
- Unverified rows (PO-RUNBOOK R1-R11): keyboard key GUID, Bluetooth connect/disconnect, default-switch, sleep/wake, audio-service restart.
- `OnDefaultDeviceChanged` fires once per role (3 per switch), so dedupe is needed. This is documented in the ADR, not yet tested live.
- OS delivers one event per set, so slider drag and key repeat flood. The 400 ms trailing coalescing proposed in the ADR is mandatory for the storage budget.
- The `Builtin` rule rests on a jack-description tuple for laptop internal speakers that I could not observe. The desktop jack shows 3.5 mm / Rear / PrimaryBox (not internal), which is the correct negative case. Runbook R9 captures the laptop tuple.
- The one-value `class` enum is lossy: BT or USB headsets are also Headphones. I propose keeping transport and form factor as separate in-memory fields. `Virtual` and `Unknown` are needed in the catalog enum (Steam Streaming Speakers is a real active virtual device here).
- HDMI/DP sinks and Bluetooth absolute-volume headsets apply gain Windows cannot see, so `PerceivedLevel.endpoint_gain_db` is uncertain for those classes.
- The registered endpoint-volume callback may die if the audio service restarts, or when the bound device disappears. The ADR proposes a watchdog/re-bind. Unverified (runbook R11, needs admin).
- Safety incident, resolved: during a test I piped `selftest` into `head`, which killed a release build with `panic=abort` mid-run. The first (killed) run left the system volume at about 0.10. I restored it to 0.02, the value the first `selftest` read at the start of that run. Earlier runs had read and restored 0.09, so the user's true pre-spike volume was probably 0.09 and went to 0.02 sometime during my session; I could not tell whether I or the user changed it. Mute is false. I removed `panic=abort` and added a runbook warning. The final state is 0.020, unmuted. If the lead or the user knows the intended level, adjust with `poke null <scalar>`.

draft ADR: D:\dev\Tinwave-worktrees\T-0008-spike-windows-volume-device\spikes\T-0008-win-volume\ADR-draft.md
Also: D:\dev\Tinwave-worktrees\T-0008-spike-windows-volume-device\spikes\T-0008-win-volume\PO-RUNBOOK.md (R1-R12 human-only rows, marked "unverified — runbook provided"). Prototype source is in the `src` folder beside them. Committed as bf96be3 on t/T-0008-spike-windows-volume-device.

follow-up tickets suggested:
- P1: PO runs the T-0008 runbook (R1-R3 and R5-R9 at minimum) and pastes the output | closes the unverified rows (keyboard GUIDs, Bluetooth, default-switch re-bind, laptop Builtin tuple) before the provider is built | size: S
- P1: Implement the Windows `OutputDeviceProvider` and `SystemVolumeProvider` in `tinwave-context` (provider thread, MTA, channel, ArcSwap cache, re-bind state machine, 400 ms gesture coalescer, mute/volume diff, classifier unit tests with fixture ids) | turns this spike into the T0/T1 signals | size: M
- P2: Update signal-catalog §1/§6 and context-engine table per ADR (add `Virtual`/`Unknown` to class, `input=System`, mute derivation, coalescing window, APIs named) | the ADR lists the exact doc changes | size: S
- P2: Make Tinwave's in-app volume use session volume or software gain (never endpoint volume), and pass Tinwave's event-context GUID whenever it touches endpoint volume/mute | prevents self-aliasing into `SystemVolumeChanged` | size: S
- P3: Add a storage-growth test fixture for volume-gesture bursts (60 events in 200 ms -> 1 coalesced event) | storage budget compliance | size: S

questions for lead / PO:
- Is a single `class` enum acceptable (BT/USB headsets map to `Bluetooth`/`UsbDac`, not `Headphones`), or should the signal carry `transport` + `form_factor` as separate quantized fields? (ADR recommends separate fields plus a derived class.)
- Should `SPDIF`/`UnknownDigitalPassthrough` map to `UsbDac` (my judgement) or be a new `Digital` class?
- Can the PO run the runbook on a laptop, and with the WH-1000XM4 (already paired on this PC)?
- Did the PO or anyone change the system volume from 0.09 to 0.02 during the spike? I could not tell, and the current value is left at 0.02.
