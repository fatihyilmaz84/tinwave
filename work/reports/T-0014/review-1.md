REVIEW T-0014 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass; clippy pass (`--workspace --all-targets -- -D warnings`; also `-p tinwave-context --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`); tests pass (tinwave-context 47 passed / 0 failed / 1 ignored — live test, not run; every other crate green); deny pass (one duplicate-`syn` 2.x/3.x warning: windows-implement/windows-interface pull syn 2).

acceptance criteria:
- [x] Classifier covers every ADR 0004 row — `every_adr_table_row_classifies_as_documented` (15 fixtures covering all 14 ADR rows incl. all 8 measured), plus `the_link_wins_over_the_form_factor`, `form_factor_rules_in_adr_order`, `anything_else_is_unknown`, `internal_jack_rule`; `classify()` (classify.rs:137-162) follows the ADR rule order exactly.
- [x] 60-step burst ⇒ one `SystemVolumeChanged` — ManualClock test (nothing at +399, one at +400, 30→90 %, gesture 177 ms, ts = last step, session attached); window edges covered.
- [x] Mute-only / scalar-only — tests plus sub-1 % and NaN; diff at engine.rs:229-259.
- [x] Own-GUID changes produce no event — tests; live check documented (not re-run, as instructed).
- [x] 3 default callbacks ⇒ one `OutputDeviceChanged` — all 6 role orders, full payload with hash, exactly 2 binds.
- [x] Idle CPU ≈ 0, RSS ≤ 4 MiB — reviewer re-ran release `audio_probe` (read-only): WS 6.76 → 10.75 MiB (+3.80 MiB), private +0.57 MiB, idle CPU 0 ms over 15 s, bound Speakers/Hda/form_factor 1. Margin thin (3.8 of 4).
- [x] Non-Windows compiles and reports `Unsupported` — linux-gnu clippy clean; `unsupported_off_windows` compiles, can't run here; `capability()` audio/mod.rs:162-171.
- [ ] fmt, clippy, tests, deny pass — all pass, but "unsafe only in a small documented COM module" and rust.md:7 (each `unsafe` block has a `// SAFETY:` comment) are not met.
- [ ] PO-RUNBOOK R1–R3, R5–R9 — open by design (epic acceptance).

blocking findings:
- com.rs:282, 378, 382, 388, 395, 397, 402 — 7 `unsafe` blocks without their own `// SAFETY:` (take_pwstr's `CoTaskMemFree`; `GetState`, `GetId`, `Activate`, `RegisterControlChangeNotify`, `GetMasterVolumeLevelScalar`/`GetMute`, error-path `UnregisterControlChangeNotify` in `bind`). The single "SAFETY (this whole block):" line at :376 covers only the first block and isn't counted by clippy; `-W clippy::undocumented_unsafe_blocks` flags exactly these 7. Blocks because the crate's opt-out from `forbid(unsafe_code)` was accepted on the condition of one documented COM module, and `bind` is the most sensitive function. Fix: true per-block SAFETY comments; reword `ComGuard::init` (:63) — `LiveEndpoint::default_render` (:462) calls it on a test thread too ("the calling thread; paired with `CoUninitialize` in `Drop` on the same thread, S_FALSE included"); add `undocumented_unsafe_blocks = "deny"` under `[lints.clippy]`.

design check: n/a

non-blocking findings:
- COM safety content verified true: CoInitializeEx(MTA)/CoUninitialize pairing incl. S_FALSE and early returns; `ComGuard` last field so interfaces release first; PWSTRs freed once; explicit `PropVariantClear` needed and correct (PROPVARIANT has no Drop in windows 0.62); `IKsJackDescription::from_raw` takes ownership once; callbacks copy + `try_send` only, fields Send + Sync; unregister on re-bind ignores vanished-device errors; `Drop` unregisters before fields drop; engine always unbinds before binding (fake asserts it).
- engine.rs:347-355 — unplug fallback gives one "A to B" event only if Windows has already moved the default; otherwise `A→Unknown` then `Unknown→B`. Both legal (ADR §5); the test scripts only the favourable order; R7 will show the real order.
- runtime.rs:33-40 — a full 1024-slot queue drops every `RawEvent` equally (counted); a dropped DefaultChanged/StateChanged/Removed leaves the binding stale until the next device event (very unlikely).
- Cargo.toml:21,35-36 path comments say `src/audio/windows/com.rs` (actual `win/com.rs`); backend.rs:4 `windows::WindowsBackend`.
- engine.rs:94 logs a volume `percent` at `warn`; rust.md:38 says volume at `trace` only (branch unreachable today).
- com.rs:478-481 — `LiveEndpoint::scalar()` returns 0.0 on a read error; the ignored live test's Restore guard would then restore to 0. Fail first if the initial read fails.
- mod.rs:60-67 — `OutputTarget::Device(String)` derives Debug with the raw endpoint id (reaches Engine/AudioMonitorConfig Debug); redact.
- Privacy otherwise clean: no friendly name/container id read; endpoint id only hashed; no tracing call carries ids/names; `DeviceSecret::generate` needs `CryptoRng`; `from_bytes` only to restore; Debug redacted; 32-bit hash collision ~1e-6 for ~100 endpoints.
- Gate and initial-bind choice acceptable vs catalog §1/§6 (SignalEvent.session is mandatory; catalog says "while Tinwave plays"); initial bind skipped only acceptable if the device at session start is captured elsewhere (context snapshot `ContextFields.output_device`, `endpoint_class` on SystemVolumeChanged). Cost: device switches while paused in a live session are lost unless `PlaybackContextSource` returns Some while paused.
- Dependencies fine (windows features all used; blake3 pure, default features off; tracing default features off).
- Tests exercise the same Engine/runtime code as the Windows backend; only com.rs uncovered. Default `cargo test` never touches the real audio stack.

proposals:
- P: Resync on dropped device events | size: S
- P: Catalog/context doc alignment for audio signals (§6 OutputDeviceChanged, §1 MuteToggled "while a session is active"; §7 lacks output-device class/transport as a context field) | size: S
- P: Unplug-ordering test (Unplugged before default moved; document the two-event outcome) | size: S

notes for lead:
- Only blocking item is mechanical.
- P2 wiring: decide whether `PlaybackContextSource::current()` returns Some while paused in a live session.
- `SystemVolumeFields` 1 % volume in `ContextFields` would multiply distinct context snapshots once hashed (≤ ~60/day target): exclude from the snapshot hash or bucket it in the ContextHub ticket.
- RSS margin thin (3.80 of 4 MiB).
- Lock-file merge order vs T-0006/T-0028.

---
Lead (2026-11-02): round 2 sent with blocking 1 plus resync on dropped events, unplug-ordering test, path comments,
warn→trace, live-test read guard, Debug redaction. Decisions: PlaybackContextSource returns Some while a session is
live, including paused (→ T-0024); catalog alignment done by the lead (P-0074); snapshot-hash note → E-01/E-03.
