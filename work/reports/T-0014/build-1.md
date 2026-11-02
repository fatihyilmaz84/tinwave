HANDOFF T-0014 (round 1)
status: done
branch: t/T-0014-windows-volume-device-provider   commit: e1cdce5
summary: Created the `tinwave-context` crate with the audio providers from ADR 0004. A provider thread (COM MTA) owns the `IMMNotificationClient` and one `IAudioEndpointVolumeCallback`. Callbacks only timestamp, copy and `try_send`, and make no COM call. All logic (own-GUID drop, mute/volume diff, 400 ms coalescing, re-bind state machine, classifier) is plain Rust over a `Backend` trait, tested with a fake OS and `ManualClock`. `unsafe` is confined to `crates/tinwave-context/src/audio/win/com.rs`. Non-Windows targets compile and report `Unsupported`. The only live check not done is the one the ticket defers to the PO (keys, flyout, Bluetooth, default switch).

acceptance criteria:
- [x] Classifier unit tests cover every ADR 0004 table row — `audio::classify::tests::every_adr_table_row_classifies_as_documented` (16 fixtures: all 8 measured rows plus USB DAC, USB headset, laptop speakers, laptop headphones, S/PDIF, BT speaker, RDP); further tests for rule order, transport parsing, internal-jack rule, `form_factor_byte`.
- [x] 60-step burst within 200 ms ⇒ one `SystemVolumeChanged` with correct from/to/gesture_ms — `engine::tests::a_60_step_burst_in_200_ms_is_one_event_with_the_first_from_and_last_to` (60 steps 3 ms apart, 30 %→90 %; nothing at +399 ms, one event at +400 ms, gesture 177 ms, ts = last step).
- [x] Mute-only ⇒ `MuteToggled` only; scalar-only ⇒ `SystemVolumeChanged` only — `a_mute_only_change_is_a_mute_toggle_and_nothing_else`, `a_scalar_only_change_is_a_system_volume_change_and_nothing_else`, `a_notification_that_changes_both_emits_both`; sub-1 % jitter and NaN ignored.
- [x] Own-GUID changes produce no event — `changes_with_tinwaves_own_context_guid_emit_nothing_but_are_tracked` (next external change's `from` correct); null and foreign GUIDs are external; live check below.
- [x] 3 default-change callbacks ⇒ one `OutputDeviceChanged` — `three_default_callbacks_for_one_switch_are_one_output_device_changed` over all 6 role orders (one event, correct class/transport/form_factor/hash, exactly two binds); repeats and capture-flow callbacks ignored.
- [x] Idle CPU ≈ 0 and RSS delta ≤ 4 MiB — `examples/audio_probe.rs`, release, `Get-Process`.
- [x] Non-Windows builds compile and report `Unsupported` — clippy on x86_64-unknown-linux-gnu clean; `cargo check --all-targets --target aarch64-apple-darwin` passes; `unsupported_off_windows` compiled (not runnable here).
- [x] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.
- [ ] PO-RUNBOOK R1–R3, R5–R9 — gate epic acceptance; need a human (P-0006).

checks: fmt pass, clippy pass (Windows workspace + linux-gnu for this crate), tests 47 passed / 0 failed / 1 ignored in tinwave-context (workspace green; ignored = live test), deny pass
files changed: Cargo.toml ([workspace.dependencies]: tinwave-signals, arc-swap, blake3 (`pure`), tracing, windows 0.62, windows-core 0.62), Cargo.lock, crates/tinwave-context/{Cargo.toml (own [lints], `unsafe_code = "deny"`, Windows-only deps with minimal Win32 features), src/lib.rs, error.rs, provider.rs, src/audio/{mod, backend, classify, identity, engine, runtime, stats, fake, engine_tests, monitor_tests}.rs, src/audio/win/{mod.rs, com.rs (all unsafe), live_tests.rs (ignored)}, examples/audio_probe.rs}
signals added/changed: none new; emits T0 `SystemVolumeChanged`, T0 `MuteToggled`, T1 `OutputDeviceChanged` with catalog payloads, `input = System`, 400 ms coalescing.
ui: none
docs updated: none (outside scope)

deviations from ticket:
- `ContextProvider`, `Capability`, `ProviderId`, `ContextFields` live in tinwave-context (architecture.md says tinwave-signals, which was out of scope and had no trait). Shapes follow context-engine.md; doc note in provider.rs.
- `ContextFields` has only `output_device` and `system_volume`; two provider views over one cache: `OutputDeviceProvider` (T1), `SystemVolumeProvider` (T0); `capability()` Supported on Windows, `Unsupported(reason)` elsewhere; failed backend ⇒ unknown fields (ADR §7).
- `device_secret` injected (`DeviceSecret`, generated via `&mut impl CryptoRng`, Debug redacted), not stored; hash = first 32 bits (LE) of `blake3(secret || endpoint_id)`.
- No `OutputDeviceChanged` for the initial bind. Events only while a `PlaybackContextSource` returns `Some` (the consumer's "while Tinwave plays" gate, ADR 4); otherwise tracked, cached, counted `suppressed_not_playing`.
- Transport parsing strictly per ADR (`bth*`, `usb`, `hdaudio`, `root`/`swd`); spike's extra guesses dropped (an RDP device on `swd` would be `Virtual` by rule order; on `ts` ⇒ Unknown). Spike's "Unknown form factor + internal HDAUDIO ⇒ Builtin" extra dropped.
- Optional `OutputTarget::Device(endpoint_id)` + `AudioMonitor::set_target` (ADR §5); a lost chosen device ⇒ Unknown (no fallback).
- Bound default unplugged/removed ⇒ immediate default re-query (one "A to B" event; later OS callback deduped).
- Callbacks stamp the bus clock (not QPC); `OnPropertyValueChanged` only counted; `max_lag_ms` diagnostic; coalescer time from `bus.now()` at handling.
- `endpoint_gain_db` via `GetMasterVolumeLevel` per event batch, quantized 0.25 dB, `gain_uncertain` for Hdmi and Bluetooth; `GetVolumeRange` not read.

measurements and live checks:
- Own-GUID live check (Win 11 10.0.26200, Realtek line-out, non-elevated), ignored test `win::live_tests::live_own_changes_are_dropped_and_external_changes_are_signals`: own-GUID volume and mute ⇒ 0 events; foreign + null GUID steps 1 ms apart ⇒ exactly 1 `SystemVolumeChanged` with real from/to; foreign mute ⇒ `MuteToggled(true)`, null unmute ⇒ `MuteToggled(false)`; restored before 0.15 / unmuted, after 0.15 / unmuted (Drop guard; no panic=abort). Run: `cargo test -p tinwave-context live_ -- --ignored --test-threads=1`. Default `cargo test` never touches the real audio stack.
- Idle cost (release `audio_probe`, 3 runs): working-set delta +3.78 to +3.81 MiB (~+4.2 MiB incl. the bus); private +0.54 to 0.58 MiB; 3–6 threads; idle CPU 0 ms over 15 s and 20 s; WS flat at 10.23 MiB; bound the real default device as class=Speakers, transport=Hda, form_factor=1 (matches ADR measured rows).
- Not done (PO-RUNBOOK): R1/R3 keys vs flyout GUIDs, R5/R6 Bluetooth, R7 default switch with real ordering, R8 USB DAC, R9 laptop, R10 sleep/wake, R11 audio-service restart.

proposals:
- P1: Move ContextProvider/Capability/ProviderId/ContextFields into tinwave-signals (or fix architecture.md) | size: S
- P2: Playback integration: implement `PlaybackContextSource` in the app wiring; persist/load `DeviceSecret` in the OS keychain | size: S
- P3: Watchdog to re-bind on power-resume and audio-service restart once the runbook shows whether registrations survive | size: S
- P4: E-01 playback must not write endpoint volume; any code that does passes `EVENT_CONTEXT_GUID` | size: S
- P5: Non-Windows CI leg for tinwave-context | size: S

questions for lead:
- Run the PO-RUNBOOK R-rows against this provider (`audio_probe` / live test) or the spike prototype?
- Accept `tinwave-signals` as a [workspace.dependencies] entry and tinwave-context's own `[lints]` (`unsafe_code = "deny"`)?

---
Lead answers (2026-11-02): own [lints] with deny + one documented COM module accepted (same pattern as
tinwave-test-alloc); workspace dep entry fine. Trait location: keep it in tinwave-context and update architecture.md
(P-0060). R-rows: the PO runs them against this production code; an `audio_probe` runbook is a follow-up (P-0061).
