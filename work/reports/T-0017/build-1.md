HANDOFF T-0017 (round 1)
status: done
branch: t/T-0017-windows-wasapi-output   commit: 03a0ae7 (on top of main 4cf2b67)
summary: Direct WASAPI output (shared by default, exclusive on request with an integer format and a visible fallback) and a cpal output (macOS/Linux, Windows fallback) implement the T-0016 `AudioOutput`/`OutputStream` contract. The default `cargo test` opens no device and makes no sound. Callback logic, format conversion, exclusive-mode choice and device following are separate pieces tested with fakes. Real-device tests are `#[ignore]`d, silent, and were run here.

acceptance criteria:
- [x] Gapless and seek through both outputs — tests/output_fake.rs (`gapless_pair_reaches_the_device_bit_exactly_in_every_format`, `seek_lands_on_the_requested_frame_at_the_device_in_every_format`): real `Pump` (WASAPI callback logic) on a fake device, wav16/flac16/alac16 in F32, I16, I24In32, I32; bit-exact (error 0.0), lag 0 at the boundary; seeks 12345, 3, 40001, 47000 + load into B exact. cpal `Callback::fill` is a thin pass-through (unit-tested), live path via the ignored device test. MP3 not repeated (covered in T-0016).
- [x] Device re-open resumes at the same frame with a fake notification — `a_default_device_change_reopens_and_resumes_at_the_same_frame`, `a_pinned_device_follows_only_its_own_fate`, `DeviceFollower` unit tests (irrelevant events ignored; default change ⇒ one `OutputLost`; `switch_device(None)` resumes at the heard frame bit-exact; pinned device ignores default changes, follows its own removal). Live: re-open in 25–29 ms.
- [x] Exclusive opens with an integer format, falls back to shared with a visible reason — `plan_exclusive`/`resolve_mode` unit tests (24-in-32, then 32, then 16; never f32; refusal ⇒ `StreamInfo.fallback` "exclusive mode refused: ..." naming rejected formats + `tracing::warn`). Live: Exclusive I24In32, 4 ms period.
- [x] Zero allocations in the output callback — tests/output_rt_alloc.rs (tinwave-test-alloc around `Pump::step` across play, volume, pause/resume, seek, device switch, A|B, end of queue, starved wakeups, all 4 formats ⇒ 0; mutations in `Pump::feed` and `convert_into` ⇒ 480). Caveat: COM calls inside the real WASAPI device aren't counted (per-thread counter; verified by reading the wasapi crate source).
- [x] fmt, clippy, tests, deny.

checks: fmt pass; clippy pass; tests 308 passed / 0 failed / 6 ignored (4 real-device + 2 pre-existing); deny pass (syn 2/3 multiple-versions warning only; cpal passes the three-target license graph). Stress: 13 playback test executables × 6 copies × 8 rounds = 624 processes, 0 failures.

measurements (Windows 11, "Luidsprekers (High Definition Audio Device)", 48 kHz stereo, software volume 0 ⇒ digital silence; 6 runs each; `cargo test -p tinwave-playback --test device -- --ignored --nocapture --test-threads=1`; "load to first frame heard" = `Engine::load` → `position().frame > 0`):
| | open | load to first frame heard | period | buffer | MMCSS |
|---|---|---|---|---|---|
| WASAPI shared f32 | 12.5–13.3 ms (first cold run 45 ms) | 39–50 ms | 480 fr = 10 ms | 1056 fr = 22 ms | active |
| WASAPI exclusive 24-in-32 | 10.4–10.7 ms | 20–24 ms | 192 fr = 4 ms | 192 fr = 4 ms | active |
| cpal (WASAPI under it) | 24–26 ms | 51–62 ms | n/r | n/r | n/a |
Re-open of the default device mid-stream 25–29 ms. `GetStreamLatency` 0 in shared event mode, one period in exclusive.

files changed: Cargo.toml (`cpal = "0.18"`, `wasapi = "0.26"` in [workspace.dependencies]); Cargo.lock; crates/tinwave-playback/Cargo.toml (cpal, tracing, Windows-only wasapi + windows, own lints); src/lib.rs (forbid → deny for unsafe, re-exports); src/output.rs → src/output/mod.rs (`OutputStream::info()` with a default); new src/output/{convert, info, plan, follow, pump, cpal, windows}.rs and windows/os.rs; tests/{output_fake, output_rt_alloc, device}.rs, common/mod.rs fake device.
unsafe: only src/output/windows/os.rs, 3 blocks with SAFETY comments (`AvSetMmThreadCharacteristicsW("Pro Audio")` reverted on drop, `AvRevertMmThreadCharacteristics`, `IAudioClient::GetStreamLatency`); everything else via the safe wasapi crate.
signals: none; enables `ExclusiveModeToggled` (host sets `WasapiControl::set_exclusive` then `switch_device`; host emits). Output never touches endpoint volume/mute (EVENT_CONTEXT_GUID rule vacuous, noted). Notification client only sets `is_lost`.
docs: module docs only.

deviations from ticket:
- tinwave-playback no longer `[lints] workspace = true`: `unsafe_code = "deny"` + `undocumented_unsafe_blocks = "deny"`, only windows/os.rs allows unsafe (tinwave-context pattern); lib.rs `#![deny(unsafe_code)]`; other workspace lints replicated.
- lib.rs has the lint attribute, docs and re-exports beyond registration.
- Mode requested via `WasapiControl::set_exclusive` + `switch_device`, not `OpenRequest` (engine.rs/OpenRequest out of scope).
- Fallback reason via `OutputStream::info()`/`WasapiControl::last_info()`, not an engine event (engine.rs/event.rs out of scope).
- Device selection by exact id (no friendly-name substring match).

notes for the reviewer: real-device tests use `Instant`; silent (volume 0), < 1 s, no system setting changed, exclusive takes the endpoint well under a second. cpal stream on its own thread (OutputStream Send everywhere), Renderer handed back through a 1-slot rtrb ring and dropped by the dropping thread; WASAPI returns it from the render thread's join. Linux/macOS cpal builds not tried (alsa-sys needs libalsa dev + pkg-config); deny covers those graphs.

proposals:
- P: Surface `StreamInfo` (mode, format, latency, fallback reason) in `OutputReopened` or a new event | size: S
- P: Live WASAPI loopback gapless run as an opt-in device test | size: S

questions for lead: none
