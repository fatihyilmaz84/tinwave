REVIEW T-0017 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass; clippy `--workspace --all-targets -D warnings` pass; tests 308 passed / 0 failed / 6 ignored (device tests not run, as instructed); deny pass (duplicate-syn warning); **stress: 360 processes, 0 failures**

Two small blocking issues: a timed-out WASAPI open leaves a render thread that can still use the abandoned renderer, and the real-device tests are not guaranteed silent. Everything else holds.

Lead's checks:
1. RT safety: `Pump::step` zero allocations (Vec mutation ⇒ "F32: ... allocated or freed 480 times"; scratch buffers sized in `new`). wasapi crate `write_to_device` (bounds check, GetBuffer, copy_from_slice, ReleaseBuffer), `get_available_space_in_frames`, `wait_for_event` don't allocate on the success path but contain `log::trace!` (non-blocking 3). cpal `Callback::fill` (cpal.rs:76-94, closure :230-240): no lock/alloc/log; `tracing::warn!` only in the error callback; renderer returned through a pre-allocated 1-slot rtrb. MMCSS set in `render_thread` (windows.rs:223), reverted by `drop(mmcss)` (:254) on the same thread after COM objects drop, before deinitialize.
2. Drop contract: WASAPI `Stream::drop` sets stop and joins (windows.rs:142-152), loop wakes ≤ 100 ms, device loss ends the loop; renderer returned as the join value. cpal: stop, unpark, join (cpal.rs:126-135); renderer handed back on the next callback; if no callback within 250 ms the renderer drops on cpal's teardown (not in a callback, acceptable); timed-out cpal open safe (callback checks stop first). Gap: WASAPI open-timeout path (blocking 1).
3. unsafe: only output/windows/os.rs, 3 blocks, comments true (`w!` static NUL-terminated, `task_index` writable; revert once in Drop on the creating thread; `GetStreamLatency` after `initialize_client`); `undocumented_unsafe_blocks = "deny"` effective (deleting a comment fails clippy); `forbid`→`deny` + module-level allow in one 48-line file acceptable (same as tinwave-context); narrower: per-function allow, or `!Send` `MmcssGuard` (non-blocking 4).
4. Exclusive: `EXCLUSIVE_CANDIDATES` I24In32, I32, I16, never F32 (plan.rs:8, tested); `is_supported_exclusive_with_quirks` changes only WAVEFORMATEX representation/channel mask, never the container; refusal ⇒ `StreamInfo.fallback` "exclusive mode refused: ..." + `tracing::warn`; failed exclusive client replaced before shared open (windows.rs:303-308); no silent fallback.
5. Device following: `DeviceFollower` (follow.rs:81-99) counts only Render Console/Multimedia default changes for `FollowTarget::Default` and not when the new default is current; removal/non-Active only for the current id; Added never; pinned ignores default changes; notification closures only set `lost` (windows.rs:409-446); no signals; nothing touches endpoint volume/mute.
6. Conversion (convert.rs): I16 `round(s × 32768)` saturating (1.0 ⇒ 32767, NaN ⇒ 0); I24In32 round + clamp to [−2^23, 2^23−1] then << 8; I32 in f64 ×2^31 saturating; 16/24-bit sources round-trip bit-exactly; exhaustive I16 and edge tests. Round-to-nearest, no dither: acceptable now (shared F32 default; exclusive prefers I24In32, error ~−144 dBFS; only exclusive I16 last resort with gain ≠ 1 or 24-bit material requantises audibly). T-0019 should own TPDF dither.
7. Default `cargo test` opens no device: real-output tests `#[ignore]`d (device.rs:117/164/190/211); `creating_an_output_does_not_open_a_device` only constructs; output_fake.rs/output_rt_alloc.rs use `FakeDeviceOutput`; cpal unit test drives `Callback` directly.

acceptance criteria:
- [x] Gapless and seek through both outputs (output_fake.rs; cpal pass-through unit-tested).
- [x] Device re-open resumes at the same frame with a fake notification.
- [x] Exclusive integer format with visible fallback (plan.rs tests; windows.rs:298-328).
- [x] Zero allocations in the WASAPI callback (output_rt_alloc.rs, mutation 480).
- [x] fmt, clippy, tests, deny.
- [ ] Ticket "Test safety (lead)": real-device tests quiet (≤ −30 dBFS) — not guaranteed (blocking 2).

blocking findings:
1. **Timed-out WASAPI open leaves a live render thread that can still render** — windows.rs:195-201 with :230-236. On `OPEN_TIMEOUT`, `open` sets `stop` and returns `Err` without joining; if `open_device` then completes (slow activation, e.g. Bluetooth), `render_thread` calls `pump.feed()` unconditionally before checking `stop`, running `Renderer::render` on a renderer the engine considers gone — it can store `flush_ack` and `fetch_add` `consumed` on shared state while the engine has moved on (retry `switch_device`, new ring/renderer): a seek acked by the wrong ring (stale audio) or position drift; renderer then dropped on that thread. cpal not affected. Fix: atomic ownership (thread CAS Opening→Running before the first `feed`; `open` on timeout CAS Opening→Abandoned; loser backs off; abandoned thread never touches the renderer).
2. **Real-device tests not guaranteed silent** — tests/device.rs:45-67, :70-74. `Engine` starts at volume 1.0; `set_volume(0.0)` is followed microseconds later by `load`; the flush-ack callback returns before updating `gain`; in shared mode the decoder usually pushes before the next callback, so the first block ramps from 1.0 to 0.0: the first ~10 ms of the −20 dBFS sweep (peak ~−21 dBFS) reaches the speakers, breaking the ≤ −30 dBFS rule and the "digital silence" claim (exclusive less likely). Fix: all-zero `short_clip` WAV (measurements only need `position().frame > 0`); optionally one starved callback before `load`.

design check: n/a

non-blocking findings:
1. windows.rs:142-152: `Stream::drop` joins the render thread; a driver hung in GetBuffer/ReleaseBuffer would hang Engine drop and app exit despite the Engine's "never hangs exit" note. Inherent to the drop contract; document it on `WasapiOutput`.
2. convert.rs:1-8: no dither (round-to-nearest); record exclusive I16 with gain ≠ 1 as a T-0019 input.
3. wasapi crate hot-path `log::trace!`: free with no logger; if the app bridges `log` to tracing at trace for wasapi, the render thread would format/allocate. Cap the `log` level for wasapi in the app's subscriber setup.
4. os.rs:19-21: `MmcssGuard` holds a `HANDLE`; if `HANDLE` is `Send`, nothing stops a drop on another thread. `PhantomData<*const ()>` makes it type-enforced.
5. windows.rs:351: `realtime_priority` derived from `notes` by string prefix ("MMCSS"); a bool from `mmcss.is_ok()` would be sturdier.

proposals:
- P: Surface `StreamInfo` (mode, format, latency, fallback reason) in an engine event | size: S

notes for lead:
- Both blocking items small and in scope (windows.rs, tests/device.rs).
- Because of blocking 2, the PO's real device may already have played a ~10 ms quiet blip when the builder ran the ignored tests; the handoff's "silent" claim was optimistic.
- Scratch: session scratchpad mut5/, stress-t17/.

---
Lead (2026-11-02): round 2 sent with blocking 1–2 plus non-blocking 1, 4, 5; no device tests in round 2.
Non-blocking 2 → T-0019, 3 → E-01 app subscriber note, StreamInfo proposal → T-0024 (P-0082, P-0083).
