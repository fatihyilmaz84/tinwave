HANDOFF T-0014 (round 2)
status: done
branch: t/T-0014-windows-volume-device-provider   commit: 3ee67fc (merge of main 2c5833c, no Cargo.lock conflict)
summary: Blocking item and six cheap items fixed. Every unsafe block in com.rs has its own true SAFETY comment; [lints.clippy] undocumented_unsafe_blocks = "deny" enforces it. Lost device events trigger a resync (deterministic thread-level test). The ignored live test was NOT run.

review items:
1. [blocking] per-block SAFETY comments on all 7 flagged blocks (take_pwstr CoTaskMemFree; in bind: GetState, GetId, Activate, RegisterControlChangeNotify, scalar/mute read, error-path UnregisterControlChangeNotify). ComGuard::init comment: "the calling thread; every success, S_FALSE included, is paired with one CoUninitialize in Drop on the same thread". Lint added; clippy clean.
2. Resync: EventSender::send fires a capacity-1 resync channel when a device event (default change, state change, removal) is dropped by a full queue (volume steps don't: the next carries the absolute value). The thread drains queued events, then Engine::resync() re-binds to the target and emits OutputDeviceChanged only if the endpoint differs. New stat AudioStats::resyncs. Channel instead of a flag to avoid a lost-wakeup race. Tests: engine::tests::resync_after_lost_device_events_rebinds_to_the_current_default_once; thread-level a_lost_default_change_makes_the_thread_re_query_the_os (callback_dropped == 1, resyncs == 1, cache moves to Bluetooth); a_lost_volume_step_does_not_trigger_a_resync (resyncs == 0).
3. Unplug ordering: unplug_before_the_default_moves_is_two_events_unknown_then_the_new_device (A→Unknown, then exactly one Unknown→B); both orders documented on Engine::on_state_changed.
4. Path comments fixed (Cargo.toml 21, 35 → src/audio/win/com.rs; backend.rs → win::com::WindowsBackend).
5. engine.rs: trace!(%error, ...) with no volume value.
6. LiveEndpoint::scalar()/muted() return Result; the live test reads both first and fails before changing anything.
7. OutputTarget manual Debug prints Device(<redacted>); Engine/config Debug go through it.

checks: fmt pass, clippy pass (workspace + linux-gnu for the crate), tests tinwave-context 51 passed / 0 failed / 1 ignored (live, not run); suite run 5× no flake; workspace green; deny pass.
RSS (release audio_probe, 3 runs): WS +3.79 to +3.81 MiB (base 6.46), private +0.55 to +0.57 MiB, idle CPU 0 ms over 15 s.
files changed: crates/tinwave-context/{Cargo.toml, src/audio/win/com.rs, win/live_tests.rs, engine.rs, engine_tests.rs, runtime.rs, stats.rs, mod.rs, backend.rs, monitor_tests.rs}
signals/ui/docs: none; deviations: none new
proposals:
- P: Surface callback_dropped/resyncs in the Signal Center diagnostics | size: S | evidence: crates/tinwave-context/src/audio/stats.rs
questions: none
