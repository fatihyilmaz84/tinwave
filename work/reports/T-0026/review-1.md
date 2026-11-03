REVIEW T-0026 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy pass (`--workspace --all-targets -D warnings`); tests pass (`cargo nextest run --workspace`: 557 passed, 6 skipped, incl. 14/14 in `tinwave-playback::stretch`); deny pass

acceptance criteria:
- [x] Builds with only MSVC + Rust, clean log — fresh `cargo build -p tinwave-playback -vv` in a `git archive` scratch copy with its own CARGO_TARGET_DIR: MSVC 14.50 compiled `tw_stretch.cpp` and `alloc_probe.cpp`, no cl warnings, no `cargo:warning`. No bindgen/libclang. Cargo.lock change is only `cc` in tinwave-playback's deps. A second build reported `Fresh tinwave-playback`; a .cpp mutation triggered a rebuild (`rerun-if-changed` works).
- [x] 1.25x stretch keeps pitch within 2 cents and length within 1 block — `sine_round_trip_keeps_pitch_within_2_cents_and_length_within_one_block` (tests/stretch.rs:125): both channels (1000/440 Hz) at 1.25, 0.75, 1.5; end within 1024 frames. Passed.
- [x] Zero allocations in `process` after construction — `process_flush_and_reset_allocate_nothing_after_construction` (tests/stretch.rs:167; Rust heap via `CountingAlloc`, C++ via the `operator new` probe) and `the_allocation_probe_counts_construction_and_drop`. Reviewer mutation (`new int` + `delete` in `tw_stretch_reset`) failed with "C++ heap operations after construction: left 4, right 0". Vendored paths checked: `tmpBuffer.resize` in process/flush stays within configure's capacity; `peaks.emplace_back` within `reserve(bands/2)` (bands = `fastSizeAbove(..)` always even, pow2 ≥ 16); Linear's `Temporary` fallback unused by the STFT. No locks.
- [x] `cargo deny check` passes; licence files present.

Vendored code: the six files downloaded from raw.githubusercontent.com at stretch `c5987267e6ad42ff9b620b2802c4600472e85818` and linear `157b448e390663e78d845ca9f8dad65140f686ad`; all six SHA-256 match the README table and the checked-in blobs (LF in git). Only extra file: the README. Only includes outside the set are `./platform/*`, behind `SIGNALSMITH_USE_ACCELERATE`/`_IPP`/`_CBLAS` (never defined).

FFI: 7 C functions with documented pointer/length contracts; every entry calling throwing code is wrapped in `catch (...)` (create → null; process/flush → 2; reset → `std::terminate` — abort, never unwind into Rust); latency getters and destroy can't throw; `/EHsc` forced (build.rs:17-19); Rust validates whole frames and `<= i32::MAX`, C++ re-checks size and null; every `unsafe` block has an accurate SAFETY comment; `#![allow(unsafe_code)]` only in `stretch/mod.rs`, `os.rs` and the test file; `Stretcher` is `Send`, not `Sync`.

Probe isolation: `tw_probe_cpp_heap_ops` absent from `tinwave_stretch.lib` and `libtinwave_playback-*.rlib` (byte search) — can't reach the app.

Latency maths (ADR 0006 decision 4): test pins in=2646, out=2646, discard = out + in/rate. Independent check: 4000 frames of silence then a tone, 2 ms RMS half-amplitude onset after `discard_frames(rate)`: onset vs `4000/rate` −58 (0.75), −23 (1.0), −23 (1.25), −56 (1.5) frames — within 1.3 ms, slightly early (window smear). The formula is right.

Merge: `git merge-tree --write-tree main t/T-0026-signalsmith-wrapper` clean (tree 39c0fea) vs main c5667c4. T-0023 already in main; lib.rs hunks don't overlap. On the merged tree: `cargo clippy -p tinwave-playback --all-targets -D warnings` pass; `cargo nextest run -p tinwave-playback` 208 passed, 4 skipped.

blocking findings:
- none

design check: n/a

non-blocking findings:
- tests/stretch.rs:152-155 — onset assertion loose and one-sided (`start <= BLOCK` allows 1024 frames late; over-discarding is invisible because the source starts at frame 0). Use leading silence (< 2*blockSamples = 10584 frames, so the silence bypass doesn't kick in) and assert the onset within ~±128 frames of `S/rate`.
- Cargo.toml:26 — `cc = "1"` in the crate; docs/rules/rust.md:8 asks for workspace-level versions → move to `[workspace.dependencies]`.
- src/lib.rs:23-36 — outside the listed scope; unavoidable for `pub mod stretch;`.
- build.rs:57 — `rustc-link-arg-tests` links the probe into every tinwave-playback integration-test binary (~22), not only `stretch`; no behaviour change (passes through to malloc/free with a thread-local counter; no other test uses C++). A future C++17 build needs the `align_val_t` overloads.
- tests/stretch.rs:4 — typo "lloc_probe.cpp".
- src/stretch/ffi.rs:1 — comment says "`tests.rs` exercises every function"; the file is `tests/stretch.rs`.

proposals:
- P: Tighten the stretch onset test (leading silence, two-sided tolerance) | size: S
- P: Move `cc` to root `[workspace.dependencies]` | size: S

notes for lead: main moved during the review (e758672 → c5667c4); merge-tree vs c5667c4 clean and green for tinwave-playback. T-0054 must land before any build that links playback ships. Scratch copies deleted. A `python` process (PID 60496, 2026-11-02 10:04) still running; not the reviewer's.

---
Lead (2026-11-03): merge. Trivial items fixed by the lead on main: `cc` moved to `[workspace.dependencies]` (P-0210),
the two comment typos, lib.rs scope accepted. P-0209 (onset test with leading silence, two-sided) merged → T-0027,
which depends on the discard being exact.
