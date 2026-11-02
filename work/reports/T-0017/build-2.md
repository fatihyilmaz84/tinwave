HANDOFF T-0017 (round 2)
status: done
branch: t/T-0017-windows-wasapi-output   commit: 5243dce (round-2 fixes) on 2bd5e6a (one engine fix, see scope note) on round 1's 03a0ae7; no merge needed (main had no tinwave-playback changes).

review items:
1. Abandoned WASAPI open: new `OpenState` in src/output/stream_thread.rs. The render thread CASes Opening→Running after its device is built and before `Pump::new` and its first `feed`; `open` CASes Opening→Abandoned on timeout. The loser backs off: an abandoned thread drops what it built and returns the renderer without rendering, writing, starting the device, acking a flush or advancing `consumed`, and reports nothing. If `open`'s CAS loses, it waits for the thread's result. Thread body moved to OS-independent `run_render_thread<D: RenderDevice>`; `RenderDevice` gained `start`/`stop`; `WasapiDevice` owns the enumerator and registration. Tests: `a_stream_abandoned_while_it_was_still_opening_never_renders` (0 writes, not started, consumed 0, flush_ack 0, ring untouched, nothing reported, renderer usable after), `a_stream_that_claimed_first_runs_and_cannot_be_abandoned`, `a_failed_open_is_reported_and_the_renderer_comes_back_untouched`. Mutation: removing the `claim_run` check fails the abandoned test.
2. Real-device tests silent by construction: `short_clip` is an all-zero WAV; `set_volume(0.0)` kept as a second guard, documented as not relied upon; module doc explains why the old approach wasn't enough. Ignored device tests NOT run this round.
3. `WasapiOutput` docs: dropping a stream joins the render thread; a hung driver in GetBuffer/ReleaseBuffer can delay engine drop.
4. `MmcssGuard` is `!Send` (`PhantomData<*const ()>`).
5. `realtime_priority` from `mmcss.is_ok()`; an MMCSS failure message still goes into `fallback`.

scope note: the first round-2 stress pass found 1 failure in 624 processes in tests/pairing.rs (seed 17) — a real engine race in src/engine.rs: with the ring empty at a track boundary, the next mark starts exactly where the previous ended; a second mark lookup inside `position()` after `poll()`'s snapshot resolved to a track with no `TrackStarted`, and a following load/seek/switch emitted `TrackLeft`/`Seeked` for a never-started track (violates the T-0024 pairing invariant). Fix (own commit 2bd5e6a): while a track is current, `position()` stays on it (clamped to its end) until `poll()` announces the next. Regression test `position_names_the_announced_track_until_poll_announces_the_next` (gated opener; fails pre-fix: "left (2,0), right (1,48777)"). engine.rs is outside T-0017's scope; leaving a known flaky failure was judged worse.

checks: fmt pass; clippy pass; tests 312 passed / 0 failed / 6 ignored (default run opens no device); deny pass. Stress: 13 playback test executables concurrent — 624 (6 × 8) and 780 (12 × 5) processes, 0 failures after both fixes.
files changed (round 2, crates/tinwave-playback): src/output/{stream_thread.rs (new), mod.rs, pump.rs, windows.rs, windows/os.rs}, src/lib.rs, src/engine.rs (position fix only), tests/{common/mod.rs, device.rs, pairing.rs}
signals: none; proposals: none; questions: none
