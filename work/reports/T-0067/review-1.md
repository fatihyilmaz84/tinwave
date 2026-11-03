REVIEW T-0067 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass (exit 0), clippy pass (`--workspace --all-targets -D warnings`, exit 0), tests pass (nextest: "793 tests run: 793 passed (4 slow), 6 skipped", exit 0), deny pass ("advisories ok, bans ok, licenses ok, sources ok")

I ran everything in D:\dev\Tinwave-worktrees\T-0067-exclusive-setting-signal at f97fd0f. Merging with current main is clean. `git merge-tree` gives 881c5f7, which differs from the branch only in `work/` (main has moved by work-only commits since 4b657f4), so these results also hold for the merged tree.

Stress runs:
- One nextest run of `--test controller_exclusive --test controller_signals --stress-count 30 --test-threads 16`: 30 of 30 iterations passed (61 tests each).
- Four copies of the same run at the same time, `--stress-count 15` each: all four passed 15 of 15 (about 89 s each).

T-0066 trial merge: I merged t/T-0067 with t/T-0066 at fde6b88 with `git merge-tree`. It is textually clean (tree e948518) and the two branches change no file in common. I built that tree in a scratch copy with its own target dir: `nextest -p tinwave-playback` gave 352 passed and 5 skipped, and `clippy -p tinwave-playback --all-targets -D warnings` was clean. The scratch copy is deleted.

acceptance criteria:
- [x] Turning exclusive off and on during a fallback to Shared emits exactly one signal each — verified by: `controller_exclusive::turning_exclusive_off_and_on_while_the_device_has_us_in_shared_is_one_toggle_each` (tests/controller_exclusive.rs:152). The opens `[48000, 44100, 48000]` show the fallback is real. `only(...)` asserts exactly one signal for off. A second poll emits nothing. There is exactly one signal for on, and a refused `switch_device` with the setting unchanged emits nothing. It passed every stress iteration.
- [x] Fallbacks and rate reopens emit nothing, and the T-0057 tests still pass — verified by: `reopening_at_the_file_rate_is_not_a_change_of_exclusive_mode` and `losing_exclusive_mode_to_a_refusing_device_...` are unchanged and pass, and the rewritten `controller_signals::the_mode_of_a_reopened_stream_alone_never_emits_exclusive_mode_toggled` passes. In the code, the `OutputOpened`/`OutputReopened` events now only reach `Self::output_stream(&info)` (mapper.rs:554-556 and 1047-1055), which only logs, so a stream event cannot emit a signal.
- [x] The T-0057 review-1 finding (a fallback erasing a deferred toggle) is covered — verified by: `a_toggle_made_before_a_session_survives_a_fallback_reopen` (controller_exclusive.rs:190). The opens `[48000, 48000, 44100, 48000]` and the signal order `SessionStarted, ExclusiveModeToggled(on), PlayStarted` match catalog §8. The old line `reported_mode = info.mode` is gone, and `reported_exclusive` is now written only in `report_exclusive` (mapper.rs:653-662).
- [x] fmt, clippy, nextest and deny pass — verified by: the reviewer runs above.

Wrapper safety (lead's question: is it safe now?): yes.
- **Every method is forwarded.** The `AudioOutput` trait (output/mod.rs:55-73) has exactly two methods: `open` and the default `wants_exclusive`. `SharedOutput` forwards both (setting.rs:27-39). `info()` and `is_lost()` belong to `OutputStream`, not `AudioOutput`, so the wrapper does not touch them. `CpalOutput` keeps the default and `WasapiOutput` overrides it; both behave as before. T-0066 at fde6b88 adds no trait method, only the `OpenRequest::exclusive_only` field and free functions, and the wrapper passes `request` through whole.
- **Threads.** The engine keeps `output` on its own struct and calls it only from `&mut self` methods (engine.rs:221/232 at start, :934 in reopen). The decoder thread gets a `Decoder` without the output. The audio callback has only the `Renderer`. `ControllerHandle` runs the `Controller`, and so the engine and `ExclusiveSetting::wanted`, on its one thread (runtime.rs:101-107).
- **No deadlock and no contention.** Each lock guard is a temporary that ends with its statement. `drain()` (mod.rs:604-605) runs only after the engine call has returned, and the engine cannot call back into the controller. So `open` can never wait behind a reader, and a reader can never wait behind `open`.
- **Other properties.** Lock poisoning is tolerated (`into_inner`). The `Send` bound holds. No output type implements `Drop`, so it does not matter that the inner output now lives until `setting` is dropped.

Signal semantics checked:
- The launch value seeds both `exclusive` and `reported_exclusive` (mod.rs:257, mapper.rs:245-247), so it is not a toggle.
- `set_exclusive_setting` returns early on an unchanged value. When the value changes it reports in a session, or defers to `report_preferences` after `SessionStarted`.
- The setting is read before the drain's events, so a session opened in the same drain reports the current value.
- "A change reverted before any session is not reported" is the existing catalog §8 rule (signal-catalog.md:141-143). Turning the setting on and back off within one 100 ms reading is a gesture with no net change. That fits the coalescing rule and does not breach the hard rule: the setting is a checkbox, and every net change is reported.

blocking findings:
- none

design check: n/a

non-blocking findings:
- crates/tinwave-playback/src/controller/mod.rs:286-287 — the comment is stale. It says the initial drain makes "the starting mode ... known before any command", but the mapper no longer tracks stream mode. The drain is still harmless, since `OutputOpened` only logs now.
- crates/tinwave-playback/src/controller/mod.rs:604-605 and mapper.rs:484-492 — an in-session toggle is stamped `clock.now()` at the drain and sent before that drain's engine events, which carry earlier engine timestamps. A `TrackLeft` or `PlayStarted` in the same drain can therefore be emitted after the toggle while its timestamp is a few ms earlier. This has no effect on tests (fake clock). It is a small ordering inversion only, the same pattern `set_repeat` already has.
- The rule "a toggle and its undo within one reading emit nothing" and the stamp being up to one poll late are written only in the controller module docs (mod.rs:133-141), not in the catalog row or §8.

proposals:
- P: Add a sentence to catalog §8: an exclusive setting change is read at the controller cadence, and an on/off flip within one reading is no change | why: the catalog is the contract the Signal Center and taste engine read, and this behaviour is currently documented only in the code | size: S

notes for lead:
- Keep T-0072 (engine passthrough) on the board. Until it lands, the hazard is a default method added to `AudioOutput` that `SharedOutput` does not forward. Nothing would fail to compile; the engine would silently get the default answer. T-0066 at fde6b88 adds no such method. If its next round adds one (for example an `exclusive_only` capability query), check setting.rs at merge time.
- Merging T-0067 and T-0066 in either order should be clean: there are no file overlaps, and the trial merge passes playback tests and clippy. T-0066's `ManualOutput` still records a refused exclusive-only open in `opens()`, so T-0067's `opens()` assertions still hold after the merge, as the passing trial shows.
- The default test run opens no audio device: the new tests use `ManualOutput` and Rig.
- Cleanup: I deleted only my own scratch folder (merge66) and the four stress logs (`st1.out`/`.err` to `st4.out`/`.err`) from C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad. No cargo, nextest, tinwave-desktop or image-stress process is still running. I did not touch the T-0066 or T-0064 worktrees; I only read T-0066 with git commands.

