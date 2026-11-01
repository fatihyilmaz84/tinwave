# Board

_Maintained by the lead. Last updated: 2026-11-01._

## Needs the PO
- Install the Qt **MSVC 2022 64-bit** kit (Qt Maintenance Tool → Qt 6.8 LTS, and optionally 6.10.1).
  Only `mingw_64` is installed; it cannot link with the MSVC Rust toolchain. Blocks T-0002 and T-0011.
- Delete the staging folders `_move-into-.claude/` and `Claude outputs/` (already installed into `.claude/`;
  kept out of git via `.git/info/exclude`).
- Hardware steps for spikes: T-0003 and T-0008 need a second output device (Bluetooth headset or USB DAC)
  plugged/unplugged during the run; T-0011 needs the OS accent and light/dark switched. Agents report these
  rows as unverified unless the PO does them.

## In progress
| Ticket | Owner | Branch / worktree |
|---|---|---|
| [T-0003](tickets/T-0003-spike-audio-backend.md) Spike: audio backend | tw-spike r1 | `t/T-0003-spike-audio-backend` · `D:\dev\Tinwave-worktrees\T-0003-spike-audio-backend` |
| [T-0008](tickets/T-0008-spike-windows-volume-device.md) Spike: Windows volume/device | tw-spike r1 | `t/T-0008-spike-windows-volume-device` · `D:\dev\Tinwave-worktrees\T-0008-spike-windows-volume-device` |

Dispatched 2026-11-01. Worktrees are made by the lead with `git worktree add` under `D:\dev\Tinwave-worktrees\`
(the Agent tool's built-in worktree isolation fails on this machine: the `^{commit}` ref loses its braces).

## In review
| Ticket | Reviewer | Build report |
|---|---|---|
| [T-0001](tickets/T-0001-workspace-and-core-ids.md) Workspace, gates, core ids | tw-reviewer **r2** | [build-1](reports/T-0001/build-1.md) · [review-1](reports/T-0001/review-1.md) CHANGES_REQUESTED · [build-2](reports/T-0001/build-2.md) bff7afb |

## Ready (dispatch order)
| Ticket | Type | Size | Scope | Note |
|---|---|---|---|---|
| [T-0002](tickets/T-0002-spike-cxx-qt.md) Spike: CXX-Qt | spike | M | spikes/T-0002 | needs Qt MSVC kit |
| [T-0011](tickets/T-0011-spike-native-qt-styles.md) Spike: native Qt styles & Theme | spike | M | spikes/T-0011 | needs Qt MSVC kit |

T-0002 and T-0011 go out as soon as the Qt MSVC kit is installed and a slot frees up (max 3 agents in flight,
reviewers included). T-0004 becomes ready when T-0001 is merged.

Design system adopted 2026-10-29: [docs/design/design-system.md](../docs/design/design-system.md), ADR 0003.
Decided 2026-11-01: [D-0001](decisions/D-0001-primary-platform.md) Windows first ·
[D-0002](decisions/D-0002-epic-order.md) recommendations before sync/connectors (spec §16 updated).

## Backlog
| Ticket | Depends on |
|---|---|
| [T-0004](tickets/T-0004-signal-schema-codec.md) Signal schema & codec | T-0001 |
| [T-0013](tickets/T-0013-injectable-clock.md) Injectable Clock + Rng seam in tinwave-core (P-0002, P-0004) | T-0001 |
| [T-0012](tickets/T-0012-gitattributes.md) .gitattributes, LF endings (P-0001) | T-0001 |
| [T-0005](tickets/T-0005-signal-bus.md) Signal bus | T-0004, T-0013 |
| [T-0006](tickets/T-0006-signals-db-writer.md) signals.db & writer | T-0005 |
| [T-0007](tickets/T-0007-storage-growth-test.md) Growth test v1 | T-0006 |
| [T-0009](tickets/T-0009-spike-time-stretch.md) Spike: time-stretch | T-0003 |
| [T-0010](tickets/T-0010-spike-jellyfin.md) Spike: Jellyfin | — (P2) |

## Design gaps to close before E-01 screen tickets
The canvas covers Library (Windows), Album + Queue (macOS), Smart Radio (Linux) and Signal Center (Windows).
E-01 also lists **Artist**, **Now Playing**, Import/scan progress and Settings, which have no artboard yet —
those screen tickets cannot be `ready` until the PO provides an artboard or direction (work/README.md DoR).

## Done
_none_
