HANDOFF T-0016 (round 2)
status: done
branch: t/T-0016-playback-engine-core   commit: b41cf11 (merge of main df3f8df on top of the round-2 fix commit 2f61706; merge clean, Cargo.lock needed no regeneration)
summary: All 9 review items are addressed. The suite is deterministic under concurrent load, skipped-packet counts are always reported, and every TrackStarted now has exactly one TrackLeft, with a new `Failed` cause. The counting allocator moved to a dev-only crate, so `tinwave-playback` is back on `[lints] workspace = true`.

acceptance criteria: all unchanged and still met (gapless, sample-exact seek, zero-alloc render, 10 Hz monotonic marks, no IO on the render thread, checks). Lossless asserts are now strict `== 0.0`.

review items:
1. Flaky tests. `Harness::settle` acknowledges the flush with an empty callback (`render(&mut [], latency)`); the rt_alloc rig settles the same way (every acknowledgement counted); `finish` has the 30 000-iteration bound; position.rs filters legitimate `Underrun`. Stress: 8 playback test executables, 6 copies each concurrently. First pass (384 processes) found 48 failures, all from the builder's new failures.rs test (`take()` split an event list before `assert_paired`; deterministic test bug), fixed. After the fix: 10 rounds × 6 copies = 480 processes, 0 failures; 5 rounds × 12 copies = 480 processes, 0 failures (960 total). Script /tmp/stress.sh, not committed.
2. CorruptPackets. New `Decoder::drop_reader` is the only way a reader goes away; it reports the pending count first while `st.cur` still names the item; used for track end, failure, `Load`, `Seek`, `Reconfigure`. Tests in tests/failures.rs use a 3 s tag-less MP3 built at test time from the fixture's frames with `big_values` = 511 in 3 frames; assert `CorruptPackets` at natural end, on skip, on seek inside the track, and on device switch; each first asserts the reader is still open. Mutation: replacing `drop_reader` with `reader = None` in those three paths fails exactly those 3 tests.
3. Leave pairing. New `LeaveCause::Failed`; marks carry `prev_failed` and `Shared.eos_failed`; `announced` keeps (item, frame). Mid-decode failure, open failure after `load`, failed seek and an all-unopenable queue end in `TrackLeft{Failed}`; corrupt packets still count as Finished. Tests: erroring source (middle and last track), open failure, all-unopenable queue, seek past end; `assert_paired` used in these, position.rs and seek-past-end.
4. Spurious `Underrun` on device switch: priming reset right after the old stream is dropped, before Reconfigure; `switching_device_resumes_at_the_exact_frame` asserts only position marks follow the switch.
5. reader.rs seek checks: a container landing after the target returns `PlaybackError::Seek`; `Reader::length` = external valid length, else Symphonia `num_frames` (excludes delay/padding); target ≥ length ⇒ `SeekOutOfRange` detected by the reader; tests for wav, flac, alac, mp3 at the end, +5000, far past, and length − 1.
6. Tests added: `TrackFailed(UnsupportedChannels{3})` from a 3-channel WAV; `GaplessInfo` delay/valid trim on flac and alac (head offset, exact length, `TrackLeft` frame 5000, seek inside the trimmed timeline, seek to the end of the valid range gives `SeekFailed`).
7. gapless.rs and seek.rs lossless asserts `assert_eq!(err, 0.0)`.
8. decoder.rs flush wait backs off from 1 ms to 50 ms (reset on any command or ack).
9. New crate `crates/tinwave-test-alloc` (dev-only; `unsafe_code = "deny"` with a single `#[allow]` on the `GlobalAlloc` impl): `CountingAlloc`, `count_allocations(|| ..) -> (R, count)` counting alloc, alloc_zeroed, realloc, free for the calling thread; 2 unit tests. `tinwave-playback` on `[lints] workspace = true`; tests/rt_alloc.rs has no unsafe. Mutation: a Vec in `render` gives "allocated or freed 774 times". Chosen over `assert_no_alloc` (aborts rather than counts).

checks: fmt pass; clippy `--workspace --all-targets -D warnings` pass; tests 216 passed / 0 failed after the merge (tinwave-playback: 2 blocking_reader, 12 engine, 12 failures, 7 gapless, 5 position, 2 rt_alloc, 3 seek; tinwave-test-alloc 2). deny pass; only MPL-2.0 allowance = 15 per-crate symphonia* exceptions; no lame in Cargo.lock.
files changed (round 2): Cargo.toml (`tinwave-test-alloc` in [workspace.dependencies]), Cargo.lock, crates/tinwave-test-alloc (new), crates/tinwave-playback: Cargo.toml, src/{decoder,engine,event,reader,shared}.rs, tests/{common/mod,engine,gapless,seek,rt_alloc,position}.rs, new tests/failures.rs.
signals added/changed: none. Event change: `LeaveCause::Failed`; every `TrackStarted` has a paired `TrackLeft`. The signal layer should map `Failed` to no `Completed` and no `Skipped`.
deviations from ticket: none new.
decisions to note:
- A corrupt packet is dropped without inserting silence, so a damaged track is slightly shorter than the file (padding needs per-codec timebase conversion).
- docs/rules/architecture.md does not list `tinwave-test-alloc` (outside scope).
proposals: none new
questions for lead: none
