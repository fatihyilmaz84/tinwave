REVIEW T-0049 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy -D warnings pass; tests pass (`cargo nextest run --workspace`: 499 passed, 6 skipped, 32.6 s); deny pass. Worktree at 15f7786, clean. `git merge-tree --write-tree main t/T-0049-artwork-provider-followups` clean (tree b4c4780). Since the merge base main changed only `packaging/*`, the README and `work/`; none of the changed signatures (`tw_provider_handle_new`, `ArtworkProvider::install`, `ShellConfig`) has a new caller on main.

acceptance criteria:
- [x] Oversize → Image.Error, no task; no-artwork key → Image.Error; in-range → Image.Ready: `refused_and_missing_artwork_are_image_error_and_served_sizes_are_image_ready` passes (48x48, 1024x8 Ready at size; 2000x2000, 1025x10, 10x1025, missing-1 Error; `tasks_run=3`, `requests=3` of 6 responses; error strings in stderr). Refusal at `cpp/image_provider.cpp:136-140`, before any task. Qt takes `textureFactory()` only when `errorString()` is empty.
- [x] Rust limit drives the C++ refusal, no second literal: `the_edge_limit_comes_from_rust_not_from_a_second_literal_in_the_shim` (32 / default / 2048) and `provider_core::the_edge_limit_is_configurable_and_clamped`. `git grep 1024` finds nothing in `cpp/`. Value flows `with_limits` (1..=i32::MAX) → `core.max_edge()` → `tw_provider_handle_new(..., edge)` → `ProviderShared::maxEdge` (no default) (`artwork_bridge.rs:112-122`, `thumb_task.h:33`).
- [x] 64x0 and 0x64 → 64-edge: `a_one_edge_source_size_keeps_that_edge` (both 64x64; 0x0 → 128x128; 64x32 stays).
- [x] Stress: builder 400 runs; reviewer 250 more (Release, offscreen): storm pool 1 parallel 2 150/150 ok (2,643,524 responses, 189 s); scroll pool 2 parallel 4 quit 50-900 ms 100/100 ok (243,535 responses, 21 s); crashes 0, hangs 0, bad_output 0; `target\stress-failures` empty; no processes left.
- [x] fmt, clippy, tests, deny.
- [x] P-0169 flake: cancel_storm 60 times in a row, 60/60, 10:41:12–10:52:02, while full-workspace nextest ran back to back in another process (17 rounds 10:40:58–10:52:43, each 499/499).

Flake root cause and fix: reproduced with the debug image-stress — scroll at 450 ms and 120 ms ended with `finished == responses` after shutdown, yet `in_flight_at_quit` was 2322 and 726; the old check raced the drain. `in_flight_at_quit` is read right after `exec()` returns and before `shutdown()`/quiesce (`tools/stress/main.rs:104-108`), in the order `responses` then `finished` (finished only grows), so it can only under-report — `> 0` is a conservative proof. The retry covers only the precondition; every extra attempt still asserts exit 0, complete teardown, `duplicate_finishes == 0`, `finished <= responses`; a crash or hang fails at once; bounded at 6 per mode; never triggered in practice (in_flight_at_quit 700–22,000 in every sampled run).

ADR 0008 rules: all hold — `done(QImage, QString)` carries values only (QString implicitly shared, safe queued); `m_error` set in the response constructor (refuse path, reader thread) or in the queued slot right before `complete()`; `complete()` the only `finished()` emitter; refusal before any task; cancelled/closing → empty image, empty error, silent; gate and quiesce unchanged.

Edge limit: `--max-edge` only in `tools/stress/main.rs:86`; production `ShellConfig::production()` uses `image::MAX_THUMBNAIL_EDGE`; packaging ships only `tinwave-desktop.exe` and check-bundle fails on `image-stress.exe`.

One-edge sourceSize → square: acceptable, documented in `resolveSize` (`cpp/image_provider.cpp:49-66`) and the `ArtworkProvider::install` docs.

wiring.rs: one field, its production default, a Debug line, the extra install argument. Startup tests pass; `default-run` unchanged.

blocking findings:
- none

design check: n/a (`qml/` untouched; `tools/stress/Sizes.qml` is a fixture).

non-blocking findings:
- docs/adr/0008-cxx-qt-bridge.md:297 — rule 2 still says `done(QImage)`; P-0177 covers it.
- cpp/image_provider.cpp:113 — the Image.Error message includes the artwork key and Qt logs one QML warning per image; harmless with `NoArtwork`. When the library-backed source lands (P-0142 / P-0178), keys must be opaque IDs, not file paths; decide on log volume.

proposals:
- P: stop alternating cargo invocations from relinking `target/debug/image-stress.exe` (running `cargo nextest -p tinwave-desktop` while `--workspace` runs fails with "failed to remove file ...image-stress.exe (os error 5)"; `tinwave-desktop` recompiles on many invocations — feature unification) | size: S

notes for lead: merge-tree clean. After merge apply P-0177 (rule 2 → `done(QImage, QString)`, Rust-owned limit, Error/silent outcomes). Reviewer logs in scratchpad (`flake.txt`, `load.txt`; `r1/`, `r2/` discarded as invalid). No processes left. The worktree's `target/` (~6 GB) can be deleted after merge.

---
Lead (2026-11-03): merge; P-0177 applied to ADR 0008 and qml-ui.md on merge. Opaque-key note added to P-0142.
P-0184 (relink churn from feature unification) parked — dev nuisance, revisit if it costs gate runs.
