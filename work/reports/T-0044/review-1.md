REVIEW T-0044 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer, worktree at 868ebf6, Qt 6.12 env): fmt pass (exit 0), clippy pass (`--workspace --all-targets -D warnings`, exit 0), tests pass (`cargo nextest run --workspace`: 417 passed, 6 skipped, 0 failed, 44 s; all 19 tinwave-desktop unit tests plus image_provider and startup integration tests passed), deny pass (advisories, bans, licenses and sources ok)
stress (run by reviewer, Release `image-stress`, offscreen, `run-stress.ps1`): 350 runs, 0 crashes, 0 hangs, 0 bad outputs
- 150 runs, storm, pool 1, quit 200-900 ms, sequential: ok=150 (559 s; 2,426,110 responses, 2,386,549 cancels, 17,826 tasks run)
- 100 runs, scroll, pool 2, quit 50-900 ms, 4 in parallel: ok=100 (20,381 tasks run)
- 100 runs, storm, pool 1, quit 200-900 ms, 4 in parallel: ok=100
- Afterwards: 0 image-stress or tinwave-desktop processes left, no `target/stress-failures` files, worktree clean.
merge: `git merge-tree --write-tree main t/T-0044-artwork-image-provider` is clean (tree dce8159). The only desktop files main changed since b76192d are qml/ and screenshots.

acceptance criteria:
- [x] Dropping the UI owner quiesces providers before the engine (order asserted): `app::ui_owner::tests::drop_quiesces_every_provider_before_the_engine` and `explicit_shutdown_does_not_drop_the_engine_and_drop_repeats_it_before_the_engine` assert the exact event log. `UiOwner::drop` quiesces, then `engine` drops, then `providers` (ui_owner.rs:29-34, 71-76). `Shell` drops `ui` before `artwork` before `app` (wiring.rs:84-88). `run()` and the stress binary call `shutdown()` explicitly after `exec()` (wiring.rs:174-177, tools/stress/main.rs:95-96).
- [x] Headless cancel storm + quit exits cleanly: `tests/image_provider.rs::cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` (10 s). Each of 7 runs asserts exit 0 within 60 s, the teardown marker, `duplicate_finishes=0` and `cancels>0`; `tasks_run>0` summed.
- [x] Stress target: >= 300 Release runs, 0 crashes, 0 exit hangs. Builder 950; reviewer 350, 0 crashes, 0 hangs.
- [x] No pointer to a response leaves the reader thread; exactly one finished(): cpp/image_provider.cpp:92-143. Response created in `requestImageResponse` on the reader thread; worker sees only the `shared_ptr<atomic_bool>` and `ProviderShared`; queued `done` with the response as context, `emit done(img)` last in `run()` (:87); `cancel()` only sets the flag (:120-124); `complete()` the single `finished()` emitter, guarded by `m_completed` (:128-137). Test: responses=64, finished=64, duplicates=0; `duplicate_finishes=0` in all 350 stress runs.
- [x] fmt, clippy -D warnings, tests, deny pass.

Six ADR 0008 lifetime rules: 1 responses on reader thread — pass; 2 queued `done`, emitted last — pass; 3 one `finished()`, cancel sets a flag, cancelled task still emits `done()` — pass; 4 bounded pool in shared state, newest-first (:112), closing checked in the response constructor (:99) and in `run()` (:77) — pass; 5 quiesce on the GUI thread after `exec()` and before the engine drops, enforced by `UiOwner` Drop and repeated in `~ThumbProvider` (:149) — pass; 6 stress target present and re-run — pass.

Rust context after quiesce: `rustCtx` (`Arc<ProviderCore>` + injected `ArtworkSource`) is released only in `~ProviderShared` (:49-56), when the last `shared_ptr` holder goes (handle, provider, responses, tasks). Any task that could call `tw_render_thumbnail` holds a reference, so no use-after-free. `ArtworkSource: Send + Sync`, so a release on another thread would also be sound.

`unsafe`: all 6 blocks in artwork_bridge.rs (:103, :109, :154, :156, :167, :178) have SAFETY comments. C++ and Rust FFI signatures match. A panic in the source is caught before the `extern "C"` boundary (:160). `allow`s only on the `mod artwork_bridge;` declaration (app/mod.rs:11).

LRU: byte-budgeted, `THUMBNAIL_CACHE_BYTES = 8 MiB` documented against spec §12 (provider_core.rs:11-17), enforced and tested (`the_cache_stays_within_its_budget`).

T-0040 behaviour: exit code 2, debug-only `TINWAVE_MAIN_QML`, both startup tests intact — but `cargo run -p tinwave-desktop` no longer works (blocking).

blocking findings:
- apps/tinwave-desktop/Cargo.toml:15-18 — adding `[[bin]] image-stress` without `default-run` makes the package two-binary: `cargo run -p tinwave-desktop` fails with "could not determine which binary to run ... available binaries: image-stress, tinwave-desktop". The command is in README.md:32 and docs/guides/desktop-dev-setup.md:47, and is T-0040's acceptance criterion. Fix: `default-run = "tinwave-desktop"` in `[package]`, then re-run `cargo run -p tinwave-desktop`.

design check: n/a (qml/ untouched; tools/stress scenes are fixtures).

non-blocking findings:
- cpp/image_provider.cpp:99-112 — check-then-act race between `closing.load()` and `pool.start()`: a request reads `closing == false`, `quiesce()` sets closing, `waitForDone()` returns, then the task starts. It takes the skip path and `~ThumbProvider`'s second quiesce waits for it, so no use-after-free, but a worker can be alive during engine teardown (crash-B class, tiny window). Inherited from the reference. Fix: a small mutex over "check closing + start" and "set closing" in `quiesce()`.
- cpp/image_provider.cpp:79-82 — `QImage(m_size, ...)` is allocated at the requested size before Rust applies `MAX_THUMBNAIL_EDGE`, and `img.isNull()` is not checked before `img.bits()` goes to Rust (contradicts provider_core.rs:19-21). Fix: refuse/clamp out-of-range sizes in C++ before allocating; skip the call if `isNull()`.
- cpp/image_provider.cpp:51-55 — the destructor comment claims `waitForDone` guards the Rust context against a worker still using it; it cannot (such a worker holds a reference). Reword.
- src/image/provider_core.rs:16 cites "`cache: false` in `TwArtwork`", which doesn't exist yet.
- src/lib.rs:4 says `unsafe` is allowed in "`app` and `image`", but `image` has none.
- src/app/wiring.rs:123-145 — the provider is registered with the owner only after `load()`; register right after `install` so the order is structural.
- tests/image_provider.rs:172-186 — no run asserts requests were still unfinished at quit (`finished < responses`); only `tasks_run > 0` summed.

proposals:
- P: Quiesce providers before `process::exit` in the `objectCreationFailed` handler (wiring.rs:135-140) | why: once Main.qml shows artwork, a failed async load exits with workers possibly running | size: S
- P: Make the closing check and `pool.start()` atomic with respect to `quiesce()` in the shim and the ADR 0008 pattern | why: closes the remaining teardown window without relying on the destructor's second quiesce | size: S

notes for lead: only blocker is the one-line `default-run` fix. `image-stress` now builds in every `cargo build --release --workspace`; packaging that copies `target/release/*.exe` must exclude it (T-0042). Merge into main 747208e is clean.

---
Lead (2026-11-02): round 2 = the blocker plus all non-blocking items and both proposals (P-0145, P-0146 merged →
T-0044 round 2): mutex over closing-check+start / set-closing, size clamp + isNull check, comment fixes, register right
after install, per-run unfinished-at-quit assertion, quiesce before process::exit. Lead adds the atomic-closing rule to
ADR 0008 on merge; packaging exclusion noted on T-0042.
