HANDOFF T-0023 (round 1, updated for P-0160)
status: done
branch: t/T-0023-live-queue-edits   commit: 57aca71
summary: The only change since the earlier HANDOFF is the session rule you rejected, now done as you decided in P-0160.
         A listener's (non-Auto) queue act made with no open session and no unheard play waits in a 64-slot buffer
         in the mapper. It is reported right after the next SessionStarted. An act on an unheard play still opens
         the session, and Auto stays silent.


**What changed in the mapper:**
- A disabled kind buffers nothing, because `bus.enabled(kind)` is checked at the act.
- When the buffer is full the oldest act is dropped and counted in the new `ControllerDiagnostics::queue_acts_dropped`.
- `report_queue_acts` runs at the end of `ensure_session`, after `report_preferences`. It checks the toggle again at report time, so an act whose kind is off by then is dropped.

**Order I chose, consistent with a deferred RepeatModeChanged:**
- SessionStarted, then deferred RepeatModeChanged / ExclusiveModeToggled, then the deferred queue acts oldest first, then the first PlayStarted.
- Each deferred act carries the session-start time and the recording and source captured at the act.
- Acts still waiting when the controller closes with no session are lost. I documented this.
- The controller docs and catalog §8 are updated.

**Tests in `tests/controller_edits.rs`:**
- **Nothing loaded:** `queue_later` ×2 emits nothing and opens no session. After the first heard play the signals are SessionStarted, RepeatModeChanged, QueuedLater, QueuedLater, PlayStarted. The test checks ts, recording and from_view, and that the acts are reported only once.
- **Overflow:** 65 acts give 64 reported (recordings 101 to 164) and `queue_acts_dropped == 1`.
  - They are sent in 13 calls of 5, because 65 single calls can hit `Busy` (the engine command queue holds 64). A first attempt flaked on that.
- **Session expired:** the edit is reported by the next session, in the order SessionStarted, QueuedNext, Resumed, then the live QueuedNext.
- **Toggles:** an act made while its category is off is never reported. An act made while on but with the category off at report time is dropped, not kept for later.

**Mutations, each caught and restored:**
- Remove `report_queue_acts` from `ensure_session`.
- Change the cap from 64 to 65.
- Remove the `bus.enabled` check at the act. My first toggle test missed this one, so I rewrote it.

**Checks:**
- fmt pass.
- clippy `--workspace --all-targets -D warnings` pass.
- deny pass (advisories, bans, licenses, sources ok).
- tests: 4 full `cargo nextest run --workspace` runs.
  - Three were 505 passed, 0 failed, 6 skipped.
  - One was 504 passed with `tinwave-db::storage_growth` failing at 70 s, the known T-0047 timeout; the crate is untouched.
- Stress: `--stress-count 25 -j 24` over queue_edits, controller_edits, controller_thread, controller_random (48 seeds), pairing, rt_alloc and controller_rt_alloc passed 25 of 25.
- rt_alloc, controller_rt_alloc and output_rt_alloc are green, as are the random walk (with the edit ops and the P-0147 property) and the pairing property.

**Files changed in this commit:**
- `crates/tinwave-playback/src/controller/{mapper,mod,types}.rs`
- `crates/tinwave-playback/tests/controller_edits.rs`
- `docs/guides/signal-catalog.md`

**Signals:** there are no new kinds. Volume is unchanged: the same signals, some now emitted late from the buffer. That is at most 64 per session start, at about 39 B each. There is no UI change.

**Deviations:** the P-0160 buffering replaces my earlier "applied but not recorded" rule. The catalog and docs edits are outside the ticket scope but were requested.

**Proposals:**
- Flush deferred queue acts at shutdown. Acts still waiting when the app closes with no session are lost. The shell could start a session on quit if any are pending, but that needs a policy decision (size S; see `Mapper::finish` and the "Queue edits" docs in `controller/mod.rs`).
- From the earlier HANDOFF, still open: surface `edits_refused` and `queue_acts_dropped` in the Diagnostics screen, folded into P-0135 (size S).

**Questions for lead:** none.
