---
id: T-0008
title: "Spike: Windows system-volume and output-device change notifications"
type: spike
epic: E-00
status: done
priority: P1
size: S
depends_on: []
scope:
  - spikes/T-0008-win-volume/
owner: tw-spike (round 1)
branch: t/T-0008-spike-windows-volume-device
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

## Outcome
2026-11-01: answered yes. Merged 1479562 (prototype + PO-RUNBOOK). Accepted as ADR 0004. Follow-ups: T-0014 (provider), P-0006 (PO runbook), P-0009 (E-01 playback constraint), P-0010 (T-0007 AC).
