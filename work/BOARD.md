# Board

_Maintained by the lead. Last updated: 2026-11-01._

## Needs the PO
- **Check your system volume.** Spike T-0008 left the Windows master volume at 2 % (unmuted). It may have been
  9 % before the spike — the agent could not tell whether you or it changed it. Set it back to whatever you want.
- Install the Qt **MSVC 2022 64-bit** kit (Qt Maintenance Tool → Qt 6.8 LTS, and optionally 6.10.1).
  Only `mingw_64` is installed; it cannot link with the MSVC Rust toolchain. Blocks T-0002 and T-0011.
- Run the T-0008 runbook [spikes/T-0008-win-volume/PO-RUNBOOK.md](../spikes/T-0008-win-volume/PO-RUNBOOK.md),
  rows R1–R3 and R5–R9 at minimum (volume keys, WH-1000XM4 connect/disconnect, default-device switch; R9 needs a
  laptop if you have one), and paste the output to the lead (P-0006). Not urgent: it gates T-0014's acceptance.
- Delete leftovers: the staging folders `_move-into-.claude/` and `Claude outputs/` (installed into `.claude/`), and
  `D:\dev\Tinwave-worktrees\T-0008-spike-windows-volume-device\` (merged; Windows kept a file lock when the lead removed it).

## In progress
| Ticket | Owner | Branch / worktree |
|---|---|---|
| [T-0003](tickets/T-0003-spike-audio-backend.md) Spike: audio backend | tw-spike r1 | `t/T-0003-spike-audio-backend` · `D:\dev\Tinwave-worktrees\T-0003-spike-audio-backend` |
| [T-0004](tickets/T-0004-signal-schema-codec.md) Signal schema & codec | tw-builder r1 | `t/T-0004-signal-schema-codec` · `D:\dev\Tinwave-worktrees\T-0004-signal-schema-codec` |
| [T-0013](tickets/T-0013-injectable-clock.md) Injectable Clock + Rng seam | tw-builder r1 | `t/T-0013-injectable-clock` · `D:\dev\Tinwave-worktrees\T-0013-injectable-clock` |

Worktrees are made by the lead with `git worktree add` under `D:\dev\Tinwave-worktrees\` (the Agent tool's built-in
worktree isolation fails on this machine: the `^{commit}` ref loses its braces).
T-0004 was told mid-flight about the ADR 0004 catalog changes (`OutputDeviceChanged` payload, `InputSource::System`).
T-0013 may change `tinwave-core`'s public id API; T-0004 adapts after whichever merges first.

## In review
_none_

## Ready (dispatch order)
| Ticket | Type | Size | Scope | Note |
|---|---|---|---|---|
| [T-0002](tickets/T-0002-spike-cxx-qt.md) Spike: CXX-Qt | spike | M | spikes/T-0002 | needs Qt MSVC kit |
| [T-0011](tickets/T-0011-spike-native-qt-styles.md) Spike: native Qt styles & Theme | spike | M | spikes/T-0011 | needs Qt MSVC kit |
| [T-0012](tickets/T-0012-gitattributes.md) .gitattributes, LF endings (P-0001) | chore | S | .gitattributes | |

Max 3 agents in flight, reviewers included. The Qt spikes go first once the Qt MSVC kit is installed.

## Backlog
| Ticket | Depends on |
|---|---|
| [T-0005](tickets/T-0005-signal-bus.md) Signal bus | T-0004, T-0013 |
| [T-0006](tickets/T-0006-signals-db-writer.md) signals.db & writer | T-0005 |
| [T-0007](tickets/T-0007-storage-growth-test.md) Growth test v1 | T-0006 |
| [T-0014](tickets/T-0014-windows-volume-device-provider.md) Windows output-device + system-volume providers (E-01) | T-0005, T-0013 |
| [T-0009](tickets/T-0009-spike-time-stretch.md) Spike: time-stretch | T-0003 |
| [T-0010](tickets/T-0010-spike-jellyfin.md) Spike: Jellyfin | — (P2) |

## Design gaps to close before E-01 screen tickets
The canvas covers Library (Windows), Album + Queue (macOS), Smart Radio (Linux) and Signal Center (Windows).
E-01 also lists **Artist**, **Now Playing**, Import/scan progress and Settings, which have no artboard yet —
those screen tickets cannot be `ready` until the PO provides an artboard or direction (work/README.md DoR).

## Decisions & design
Design system adopted 2026-10-29: [docs/design/design-system.md](../docs/design/design-system.md), ADR 0003.
Decided 2026-11-01: [D-0001](decisions/D-0001-primary-platform.md) Windows first ·
[D-0002](decisions/D-0002-epic-order.md) recommendations before sync/connectors (spec §16 updated).
Accepted 2026-11-01: [ADR 0004](../docs/adr/0004-windows-volume-device-provider.md) Windows volume/device provider (from T-0008).

## Done
| Ticket | Merged | Reports |
|---|---|---|
| [T-0001](tickets/T-0001-workspace-and-core-ids.md) Workspace, gates, core ids | 1452c72 | build-1/2, review-1 (changes), review-2 (approve) |
| [T-0008](tickets/T-0008-spike-windows-volume-device.md) Spike: Windows volume/device | 1479562 | spike-1 → ADR 0004 |

Proposals: P-0001 → T-0012 · P-0002, P-0004 → T-0013 · P-0003 parked (CI, needs a remote) · P-0005, P-0008 done by
lead · P-0006 → PO runbook · P-0007 → T-0014 · P-0009 → E-01 playback constraint · P-0010 → T-0007.
