REVIEW T-0014 (round 2)
verdict: APPROVE
checks (run by reviewer, at 3ee67fc): fmt pass; clippy Windows `--workspace --all-targets -- -D warnings` pass (also fresh in a separate target dir for `-p tinwave-context --all-targets` so the new `undocumented_unsafe_blocks = "deny"` lint was really checked); clippy Linux `-p tinwave-context --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` pass; tests pass (tinwave-context 51 passed, 0 failed, 1 ignored — live test not run; every other crate green incl. merged tinwave-playback pairing tests); deny pass (same duplicate-syn warning).

acceptance criteria:
- [x] Classifier covers every ADR 0004 row (unchanged).
- [x] 60-step burst ⇒ one SystemVolumeChanged (unchanged).
- [x] Mute-only / scalar-only (unchanged).
- [x] Own-GUID changes produce no event (unchanged; live check in build-1).
- [x] 3 default callbacks ⇒ one OutputDeviceChanged, all 6 role orders (unchanged).
- [x] Idle CPU ≈ 0, RSS ≤ 4 MiB — re-measured release audio_probe (read-only): base 6.46 MiB, +3.80 MiB, 0 growth, 0 ms CPU over 15 s.
- [x] Non-Windows compiles and reports Unsupported.
- [x] fmt, clippy, tests, deny — round-1 blocker resolved: 29 unsafe blocks, 29 SAFETY comments, lint enforces it.
- [ ] PO-RUNBOOK R1–R3, R5–R9 — open by design (epic acceptance).

blocking findings:
- none

Round-1 items verified:
- SAFETY comments (com.rs:284, 379, 384, 390, 399-400, 403, 409-410) all accurate (GetId string freed by take_pwstr; RegisterControlChangeNotify callback only copies and try_sends; error-path Unregister undoes the registration just made).
- ComGuard::init (com.rs:63-66) correct for both callers; every success incl. S_FALSE paired with one CoUninitialize on the same thread; guard private inside non-Send WindowsBackend/LiveEndpoint, so it can't leave its thread.
- Resync (runtime.rs:38-49, 214-224): no lost wakeup (token set after the drop; thread takes token, drains, re-queries); a second drop while a token is pending is covered by one re-query; a drop after the token was taken sets a new token; no loop (resync only re-binds, capacity 1, no-change resync emits nothing); dropped volume step doesn't resync (`is_device_event`); thread-level test deterministic (gate, 1-slot queue, callback_dropped == 1, resyncs == 1, cache → Bluetooth).
- Unplug ordering test (engine_tests.rs:574-596) models the real order (bind fails NotActive ⇒ one Unknown, then one Speakers); both orders documented on Engine::on_state_changed.
- Path comments, trace! with no value, Result-returning live reads, redacted OutputTarget Debug — fixed.
- Scope: crates/tinwave-context/ + root Cargo.toml + Cargo.lock; playback lines come from the clean main merge.

design check: n/a

non-blocking findings:
- monitor_tests.rs `a_lost_volume_step_does_not_trigger_a_resync`: waits for stale_events >= 1 then checks resyncs == 0; a regression could race past it. Stronger: hold the gate and assert the resync receiver is empty, or wait a second event round.
- live_tests.rs:43: ~14 literal spaces in an eprintln! format string (cosmetic, ignored test).
- A dropped *last* volume step of a gesture leaves `to` and the cache one step behind until the next notification (counted; harmless).

proposals:
- P: Signal Center diagnostics for callback_dropped/resyncs/bind_failures (same as builder's) | size: S
- P: Catalog/context doc alignment (carried over) | size: S

notes for lead:
- P2 wiring: Some while paused; exclude/bucket the 1 % volume from the ContextHub snapshot hash.
- RSS margin 3.80 of 4 MiB.
- Branch contains the T-0028 merge; merge into main should be clean.

---
Lead (2026-11-02): merged. Catalog alignment and the paused-session decision were already done (P-0074, T-0024);
diagnostics → E-01 (P-0077). Non-blocking 1-3 parked as P-0078 (fix when the crate is next touched).
