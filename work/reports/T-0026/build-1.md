HANDOFF T-0026 (round 1)
status: done
branch: t/T-0026-signalsmith-wrapper   commit: 9e713f1
summary: Signalsmith Stretch 1.3.1 and Signalsmith Linear vendored as pinned MIT headers with their licence files and a provenance README. `build.rs` compiles a small C ABI over them with `cc` (MSVC only, no bindgen, no libclang). The safe `tinwave_playback::stretch::Stretcher` wraps it, not wired into the engine (T-0027). `process`, `flush` and `reset` allocate nothing on the Rust or C++ heap. Output is deterministic (the random engine uses a constant seed).

acceptance criteria:
- [x] Builds on Windows with only MSVC + Rust, clean build log — `touch build.rs; cargo build -p tinwave-playback -vv` compiles tw_stretch.cpp and alloc_probe.cpp with zero warnings; vendored headers via `/external:I` + `/external:W0`; `/EHsc` forced (cc-rs didn't enable it). Only `cc` added to Cargo.lock.
- [x] 1.25x stretch of a sine keeps pitch within 2 cents and length within 1 block — `sine_round_trip_keeps_pitch_within_2_cents_and_length_within_one_block` (stereo 1000 Hz / 440 Hz; rates 1.25, 0.75, 1.5; 3 s source, 1024-frame blocks): pitch error ≤ 0.0009 cents; onset after the ADR 0006 lag discard 0 frames late; tone end vs exact stretched length +311 (1.25), +151 (0.75), +351 (1.5) frames, tolerance 1024.
- [x] Zero allocations in `process` after construction — `process_flush_and_reset_allocate_nothing_after_construction`: Rust heap via T-0016 `CountingAlloc`/`count_allocations`, C++ heap via a test-only `operator new/delete` probe; region: reset, 360 process calls of mixed shapes (empty in, empty out, 1:1, big gulp, silence), flush, reset → 0 Rust and 0 C++ heap operations. `the_allocation_probe_counts_construction_and_drop` guards the probe.
- [x] `cargo deny check` passes; licence files present (vendor/signalsmith/signalsmith-stretch/LICENSE.txt, …/signalsmith-linear/LICENSE.txt). Neither deny nor the notices generator reads them (question 1).

Mutation checks (each reverted): a `std::vector` allocation in `process` fails the allocation test; a `random_device` seed fails `bit_identical_even_where_the_library_draws_random_numbers` (rate 0.4, where the library draws randoms); removing the flush `memset` fails `flush_returns_the_tail_and_zero_fills_the_rest`; dropping `in_latency/rate` from `discard_frames` fails the latency and round-trip (onset) tests; all channels reading channel 0 fails the round-trip test and `mono_and_many_channels_keep_their_own_tones`.

checks: fmt pass, clippy pass, tests 557 passed / 0 failed (6 skipped; 14 new in `tests/stretch.rs`), deny pass

files changed (crates/tinwave-playback/ except Cargo.lock): build.rs; Cargo.toml (build-dependency `cc = "1"`, lint comment); Cargo.lock (+`cc`); src/lib.rs (`pub mod stretch;` + 2-line `deny(unsafe_code)` comment); src/stretch/{mod.rs, ffi.rs, tw_stretch.h, tw_stretch.cpp, alloc_probe.cpp}; tests/stretch.rs; vendor/signalsmith/README.md; vendor/signalsmith/signalsmith-stretch/{signalsmith-stretch.h, LICENSE.txt}; vendor/signalsmith/signalsmith-linear/{fft.h, linear.h, stft.h, LICENSE.txt}
signals added/changed: none
ui: none
docs updated: the vendor README (version, source, SHA-256 of every file, what was left out, update procedure).

deviations from ticket:
1. Tests in `crates/tinwave-playback/tests/stretch.rs` (not in the scope list): `rustc-link-arg-tests`, which links the test-only C++ allocation probe, applies only to `tests/` targets (a lib unit test gave a link error); the probe replaces global `operator new` and must never reach the application. Stretch API used by the tests is `pub`.
2. `pub mod stretch;` (a private, unused-until-T-0027 module would fail `-D warnings` on dead code).
3. `cc = "1"` in the crate's Cargo.toml, not root `[workspace.dependencies]` (root out of scope).
4. `StretchError` is module-local (error.rs out of scope; T-0023 may edit it).
5. Headers are Stretch 1.3.1 as ADR 0006 measured, not upstream tag 1.4.0: untagged commit `c5987267e6` (2025-06-14) between 1.3.0 and 1.4.0; Linear commit `157b448e39` (2025-06-12); all six files checked by SHA-256 against raw upstream; `platform/` back ends and `cmd/web/tests` left out.
6. Adds `Stretcher::flush` (end of track) and `Stretcher::discard_frames(rate)` (ADR 0006 `out_latency + in_latency/rate`, validated by the onset assertion).

proposals:
- P: Notices generator covers vendored C/C++ code (reads `LICENSE*` under `crates/*/vendor/**`, like the icons pass) | why: the Signalsmith MIT texts won't appear in THIRD-PARTY-NOTICES | size: S | evidence: New-ThirdPartyNotices.ps1:151, 167, 186-191
- P: Evaluate Signalsmith Stretch 1.4.0 (needs the listening pass; ADR 0006 numbers are 1.3.1) | size: S
- P: Rate-driver helper for T-0027 (the ADR 0006 decision-3 fixed-output-block / `floor(out*rate + carry)` arithmetic exists only as the test helper `drive()`) | size: S
- P: Cross-platform check of build.rs and the probe link (non-MSVC branches untested) | size: S

questions for lead:
1. Who updates the packaging notices for vendored code?
2. Is the test file outside the listed scope acceptable?
3. Left other builders' processes alone; everything started has finished.

---
Lead (2026-11-03):
1. P-0204 accepted → T-0054 scope (release-readiness packaging): the generator must read `LICENSE*` under
   `crates/*/vendor/**` for every crate in the shipped closure. tinwave-playback is not in the desktop closure yet,
   so nothing ships without it today; T-0054 must land before a build that links playback.
2. Test file in tests/: accepted (forced by `rustc-link-arg-tests`; keeps the probe out of the app).
- P-0205 (Stretch 1.4.0) parked → after the PO's ADR 0006 listening pass. P-0206 (rate driver) merged → T-0027 note.
  P-0207 (cross-platform build) parked → T-0038/T-0039.
