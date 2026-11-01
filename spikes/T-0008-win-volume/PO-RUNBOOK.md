# T-0008 PO runbook — things only a human can verify

Prototype: `spikes/T-0008-win-volume/` (standalone Cargo project, not part of the workspace).
All commands run from that folder in a normal (non-elevated) terminal. Nothing here needs admin.

```
cargo build --release
$e = ".\target\release\t0008-win-volume.exe"
```

Notes
- `watch` prints one line per event. `origin=` is the Windows event-context GUID of whoever caused the change:
  `self` = Tinwave's own GUID (never true for these manual tests), `external(null-ctx)` = caller passed no GUID,
  `external({guid})` = some other component. **Please copy the exact GUID you see for keyboard keys vs. the
  volume flyout vs. the mixer** — if they differ, we can later tell "keyboard key" from "slider" for free.
- Do not pipe `selftest` into `head`/`more`: closing the pipe kills the process before it restores your volume.
  (`selftest` restores volume/mute in a Drop guard; a killed process cannot.)
- Press Ctrl+C or give `watch` a number of seconds to stop it.
- `watch` follows the default render device (re-binds when it changes) and prints the device class.

Start with an overview of what the prototype sees on your machine:

```
& $e enumerate
```
Expected: every render and capture endpoint with `state`, `form_factor`, `transport`, `jack(...)` and `class`.
(Already run by the spike on the dev PC; run it on a **laptop** and on any machine with a USB DAC/headset.)

## Rows

Each row: start `& $e watch 60` (60 s), do the action, paste the output lines back into the ticket.
Status for every row is **unverified — runbook provided** until you paste output.

| # | Scenario | Do | Expected output | What to record |
|---|---|---|---|---|
| R1 | Keyboard volume keys | `watch 60`; press Volume Up x5, Volume Down x5 | 10 `VOLUME ... origin=external(...)` lines, vol changes by about 0.02 per press, latency feels instant | The `origin=` GUID (null or a shell GUID); whether every press produces exactly one line; key-repeat (hold the key 2 s): how many lines |
| R2 | Mute key | `watch 60`; press Mute twice | Two `VOLUME ... muted=true` then `muted=false` lines with the **same** `vol=` | Confirms mute arrives in the same notification and must be diffed by the consumer |
| R3 | Volume flyout / taskbar slider | `watch 60`; click the speaker icon and drag the slider slowly across a range | A stream of `VOLUME` lines, many per second while dragging | The `origin=` GUID; line count for one drag (gesture coalescing needs this) |
| R4 | Per-app mixer (Settings > System > Sound > Volume mixer) | `watch 60`; move the slider of any single app (browser etc.) | **No** `VOLUME` lines (per-app session volume is not endpoint volume) | Confirms we correctly ignore per-app volume; spike's `selftest` G already shows 0 events for our own session |
| R5 | Bluetooth headset connect | Pair WH-1000XM4 beforehand (it is already known to Windows: both its endpoints show `UNPLUGGED`). `watch 120`; switch the headset on/connect | `STATE id=..` with `state=ACTIVE` (UNPLUGGED -> ACTIVE) for the headset's endpoint(s); then `DEFAULT_CHANGED flow=render role=console` (and multimedia/communications) if Windows auto-switches; then `-> re-bound to new default render device` showing `class=Bluetooth` | The order and time gap between STATE and DEFAULT_CHANGED; whether a hands-free (HFP, `Headset`) endpoint appears too and takes the default for communications |
| R6 | Bluetooth headset disconnect | Continue R5; switch the headset off | `STATE ... UNPLUGGED` (or NOTPRESENT), `DEFAULT_CHANGED` back to the speakers, re-bind line with `class=Speakers` (or Builtin on a laptop) | Same; also check volume lines still arrive on the speakers after the re-bind (press a volume key) |
| R7 | Default device switch in Settings | `watch 60`; Settings > System > Sound > choose another output device (e.g. the monitor `Odyssey G70B`) | Up to three `DEFAULT_CHANGED` lines (one per role); exactly one `-> re-bound` line; `class=Hdmi` for the monitor | Number of DEFAULT_CHANGED lines per switch; whether a volume key now changes `vol=` on the monitor endpoint (HDMI/DP sinks often have software-only volume) |
| R8 | USB DAC / USB headset plug | `watch 90`; plug in, set as default, press a volume key, unplug | `ADDED`/`STATE` lines, `DEFAULT_CHANGED`, re-bound with `class=UsbDac`; after unplug STATE and fallback | The `form_factor` of the device; that `class=UsbDac` is the right bucket for a USB headset (see ADR open question) |
| R9 | 3.5 mm jack on a laptop | `enumerate` with headphones unplugged, then plugged | Internal speakers: `class=Builtin` (expected `jack` tuple with GenLocation=1 Internal or ConnectionType=3). Headphones plugged: Headphones endpoint `ACTIVE`, `class=Headphones`, plus a `DEFAULT_CHANGED` on plug | The full `jack(conn,geo,gen,connected)=` tuple for internal speakers: it is the only input of the Builtin rule that was **not** observable on the dev desktop |
| R10 | Sleep / wake and lock | `watch 600`; sleep the PC for a minute, wake, press a volume key | A `VOLUME` line after wake | Whether events survive sleep/wake without re-binding (if not: re-bind on power-resume) |
| R11 | Audio service restart (optional, needs admin, causes a brief audio glitch) | `watch 120`; in an **elevated** shell `Restart-Service audiosrv -Force`; then press a volume key | Possibly `DEFAULT_CHANGED`/`STATE` storms, possibly no more `VOLUME` lines | If volume lines stop, the provider needs a watchdog (re-bind on any DEFAULT_CHANGED/STATE for the bound endpoint, plus periodic re-register) |
| R12 | Overhead with the real app | Later (needs the player): run `& $e bench 600` beside normal use | `idle CPU` 0.00x %, `working_set` about 10 MiB | Compare with the dev-PC numbers in the ADR |

## Cleanup / safety
- `selftest` changes the endpoint volume by a few percent for a few seconds and toggles mute for ~0.3 s. It restores
  both. If a run is ever killed, set your volume back with: `& $e poke null 0.3` (any scalar 0.0-1.0) and `& $e poke null mute0`.
- Nothing is installed, no registry/service/driver changes, no network access.
