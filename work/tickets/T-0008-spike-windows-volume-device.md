---
id: T-0008
title: "Spike: Windows system-volume and output-device change notifications"
type: spike
epic: E-00
status: ready
priority: P1
size: S
depends_on: []
scope:
  - spikes/T-0008-win-volume/
owner:
branch:
review_round: 0
created_by: lead
---

## Context
Signal catalog §1 `SystemVolumeChanged`, §6 `OutputDeviceChanged`; docs/guides/context-engine.md provider table.
Most users change volume with keyboard keys — capturing it is key to the "what do they turn up" signals.

## Question
Can we reliably receive master/endpoint volume, mute, and default-device changes on Windows with low overhead,
and classify the device (headphones / speakers / Bluetooth / USB DAC / HDMI)?

## Scope
- Console prototype with the `windows` crate: `IAudioEndpointVolume::RegisterControlChangeNotify`,
  `IMMNotificationClient`, endpoint form factor / transport properties.
- Log each change with timestamp; distinguish our own app-volume changes from user changes (event context GUID).
- Measure CPU/RSS overhead idle; test with BT headset connect/disconnect.

## Time box
Half a working session.

## Acceptance criteria
- [ ] SPIKE RESULT with event latency, reliability notes, device-class mapping table.
- [ ] Confirms no global input hooks or elevated permissions are needed.
- [ ] Draft ADR or provider design note at `spikes/T-0008-win-volume/ADR-draft.md`.

## Signals & storage
Confirms feasibility of T0 `SystemVolumeChanged` and T1 `OutputDeviceChanged`.
