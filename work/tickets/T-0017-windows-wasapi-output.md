---
id: T-0017
title: Windows output on direct WASAPI (shared + exclusive), cpal fallback output
type: feature
epic: E-01
status: backlog
priority: P0
size: M
depends_on: [T-0016]
scope:
  - crates/tinwave-playback/src/output/
  - Cargo.toml            # [workspace.dependencies] only
  - Cargo.lock
owner:
branch:
review_round: 0
created_by: from P-0023
---

## Context
Spike T-0003 (src/backends.rs; WASAPI median 18 ms start, 22 ms latency, exclusive 24-in-32 works). ADR 0004 (device
and system-volume signals live in tinwave-context, T-0014). D-0001 Windows first.

## Scope
- `output/windows.rs`: WASAPI shared (default) and exclusive (`ExclusiveModeToggled`), MMCSS thread, event-driven.
- Its own `IMMNotificationClient` used **only** to re-open the stream when the default/chosen device changes or
  disappears. It emits no signals: signals come from tinwave-context, so turning off T1 device context never breaks
  device following (lead decision on T-0003).
- `output/cpal.rs`: generic output for macOS/Linux and as Windows fallback.
- In-app volume stays in the renderer (software gain), never endpoint volume (ADR 0004 decision 5); any code that
  ever touches endpoint volume/mute passes tinwave-context's `EVENT_CONTEXT_GUID` (P-0064).

## Notes from the T-0016 review
- Real outputs must honour the `OutputStream` drop contract: the callback has finished when drop returns.
- Never drop the `Renderer` (it holds the last `Arc<Shared>`) on the RT thread.

## Acceptance criteria
- [ ] Gapless and seek tests from T-0016 pass through both outputs on Windows (loopback or callback recording).
- [ ] Device re-open resumes at the same frame — test with a fake notification; live run documented.
- [ ] Exclusive mode opens with an integer format and falls back to shared with a visible reason when refused.
- [ ] Zero allocations in the WASAPI callback — counting-allocator test.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
Enables `ExclusiveModeToggled`; no new signals.
