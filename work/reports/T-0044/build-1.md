HANDOFF T-0044 (round 1)
status: done
branch: t/T-0044-artwork-image-provider   commit: 868ebf6
summary: The desktop app now has an async artwork image provider (`image://art/<key>`) built on the T-0041 lifetime pattern. A C++ shim in `cpp/` holds the provider, response and task. The Rust side has an injected `ArtworkSource`, a byte-budgeted LRU and counters. `UiOwner` owns the engine and every provider, and its Drop quiesces all providers before the engine is dropped. `Shell::shutdown()` is also called explicitly after `exec()`. A dev-only `image-stress` binary with a script reproduces crash-T-0041/run-batch.ps1 against the app's own provider. 950 Release stress runs gave 0 crashes and 0 exit hangs. The bridge file is in `src/app/`, not `src/image/` (see deviations).

acceptance criteria:
- [x] Dropping the UI owner quiesces providers before the engine (order asserted) — proof: unit tests `app::ui_owner::tests::drop_quiesces_every_provider_before_the_engine`, `explicit_shutdown_does_not_drop_the_engine_and_drop_repeats_it_before_the_engine`, `an_owner_without_providers_just_drops_the_engine`. They use a recording fake engine and fake providers and assert the exact event sequence, passing under nextest.
- [x] Headless cancel storm + quit with tasks in flight exits cleanly (offscreen) — proof: `tests/image_provider.rs::cancel_storm_with_requests_in_flight_then_quit_exits_cleanly`. 7 runs (storm and scroll modes, quit at 150-800 ms, 1 pool thread) on the offscreen platform with software rendering. Each run asserts exit 0 within 60 s, the `STRESS_TEARDOWN_COMPLETE` marker, `duplicate_finishes=0`, `cancels>0`, `finished<=responses`, and `tasks_run>0` summed over the runs. About 10 s.
- [x] Stress target: >= 300 Release runs, 0 crashes, 0 exit hangs — proof: `tools/stress/run-stress.ps1`, Release build, offscreen platform, random quit time. 950 runs, 0 crashes, 0 hangs, 0 bad outputs:
  - 300 runs, storm mode, 1 pool thread, quit 200-900 ms, sequential: ok=300 (837 s; 5,234,557 responses, 5,155,462 cancels, 41,165 tasks run).
  - 300 runs, same settings, 4 in parallel: ok=300 (264 s).
  - 150 runs, storm, 2 pool threads, quit 50-900 ms, 4 in parallel: ok=150.
  - 200 runs, scroll mode, 1 pool thread, quit 100-900 ms, 4 in parallel: ok=200 (24,274 tasks run).
- [x] No pointer to a response leaves the reader thread; exactly one finished() per response — proof:
  - Review check: structure documented at the top of `cpp/image_provider.cpp`. The only things that cross threads are the task's queued `done(QImage)` signal and a `shared_ptr<atomic_bool>` cancel flag. `cancel()` only sets that flag. `complete()` is the single `finished()` emitter and suppresses and counts a second one (`duplicate_finishes`).
  - Test: `image_provider::every_response_gets_exactly_one_finished_and_the_cache_serves_repeats`. 64 images all settle: responses=64, finished=64, duplicates=0. Cache: 14 distinct keys plus 8 keys without artwork, so source_calls=22, cache_hits=42, misses=8.
- [x] fmt, clippy -D warnings, tests, deny pass — proof: see checks.

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 417 passed / 0 failed (nextest --workspace, 6 skipped as ignored; 21 of them in tinwave-desktop), deny pass (advisories, bans, licenses and sources ok)
files changed (apps/tinwave-desktop/):
- new: `cpp/image_provider.cpp`, `cpp/image_provider.h`, `cpp/thumb_task.h`
- new: `src/app/artwork_bridge.rs`, `src/app/ui_owner.rs`
- new: `src/image/mod.rs`, `src/image/cache.rs`, `src/image/provider_core.rs`, `src/image/source.rs`
- new: `tools/stress/main.rs`, `tools/stress/Storm.qml`, `tools/stress/Settle.qml`, `tools/stress/run-stress.ps1`
- new: `tests/image_provider.rs`, `tests/common/mod.rs`
- modified: `build.rs`, `Cargo.toml`, `README.md`, `src/lib.rs`, `src/app/mod.rs`, `src/app/wiring.rs`, `tests/startup.rs`
signals added/changed: none (artwork display is not a preference interaction)
ui: none. `qml/` is untouched. The only QML added is the dev-only stress scenes under `tools/stress/` (test fixtures).
docs updated: `apps/tinwave-desktop/README.md` (layout table, artwork provider section, how to run the stress target)
deviations from ticket:
- The provider bridge is `src/app/artwork_bridge.rs`, not under `src/image/`: cxx-qt-build panics with "Only one directory is supported per QmlModule for rust_files" (QTBUG-93443) when bridge files sit in different directories, and `app_info.rs` is already in `src/app/`. The logic (source trait, cache, core, counters) is in `src/image/`.
- Also edited `Cargo.toml`, `src/lib.rs`, `README.md` and `tests/startup.rs` (not in the scope list): a `[[bin]]` for the stress target, `pub mod image`, docs, and moving the shared Qt-PATH helper into `tests/common` (startup behaviour unchanged). T-0040 behaviour intact: exit code 2 on QML load failure, debug-only `TINWAVE_MAIN_QML`, both startup tests pass.
- The stress target is a `[[bin]]` named `image-stress` (path `tools/stress/main.rs`), not an example, so the integration tests can use `CARGO_BIN_EXE_image-stress`. Built with `cargo build --workspace`; dev-only, not packaged, README says so.
- `wiring::run()` is now a thin wrapper over `Shell::start` / `exec` / `shutdown`. The stress binary and `main` run the same startup and teardown code.
- `main` wires a `NoArtwork` source (no key has artwork) until the library-backed source exists; real image loads end in an `Image` error state for now.
- `#[allow(unsafe_code, missing_debug_implementations, clippy::missing_safety_doc)]` on the `mod artwork_bridge;` declaration: `missing_debug_implementations` for the opaque C++ type `ProviderHandle`; `missing_safety_doc` because the cxx bridge macro drops the `# Safety` doc of `tw_provider_handle_new`.

proposals:
- P: Add the stress target to CI (nightly or pre-release) | why: testing.md requires re-running it on any provider change | size: S | evidence: `tools/stress/run-stress.ps1`; 300 sequential ≈ 14 min, 4-parallel ≈ 4.5 min
- P: ADR 0008 / qml-ui.md note: all bridge files of one QML module must share a directory | why: every later view-model ticket will hit it | size: S | evidence: cxx-qt-build 0.10.0 src/lib.rs:876
- P: Note for bridge authors: cxx has no `c_void`, and `# Safety` docs are dropped by the macro | why: forces `*const u8` context and a clippy allow on the `mod` declaration | size: S | evidence: `src/app/artwork_bridge.rs:26`, `src/app/mod.rs`
- P: Library-backed `ArtworkSource` returns decoded thumbnails in the contract format off the Qt thread | why: the trait contract is fixed as `QImage::Format_RGB32` (opaque); the library ticket must resize and convert | size: M | evidence: `src/image/source.rs` doc comment

questions for lead:
- None. No leftover image-stress/tinwave-desktop processes (lead confirmed 0). No Windows settings, Credential Manager entries or ignored/real-device tests touched.

---
Lead (2026-11-02):
- Out-of-scope edits (Cargo.toml, lib.rs, README, tests/startup.rs) and the bridge in src/app/: accepted — same crate,
  minimum needed, forced by cxx-qt-build.
- Proposals: P-0139 (stress in CI) parked with the CI proposals; P-0140 (one directory per QmlModule) and P-0141
  (no c_void, dropped `# Safety` docs) accepted → lead adds them to ADR 0008 / qml-ui.md on merge; P-0142 (library
  ArtworkSource contract) parked → input to the library/scanner ticket.
- Merge note: this branch is from b76192d; main has since merged T-0024 and T-0035 (Main.qml/Theme.qml only, no overlap).
