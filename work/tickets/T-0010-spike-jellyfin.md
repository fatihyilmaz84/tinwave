---
id: T-0010
title: "Spike: Jellyfin connect, browse, and stream"
type: spike
epic: E-00
status: backlog
priority: P2
size: S
depends_on: []
scope:
  - spikes/T-0010-jellyfin/
owner:
branch:
review_round: 0
created_by: lead
---

## Context
Spec §4.2, §4.5, §16 Phase 0. Desktop focus puts connectors later (D-0002), but the connector trait shape
should be validated early so source identity (spec §4.7) and signal `source` ids are right.

## Question
What does a minimal `Connector` trait need to support Jellyfin auth, paginated music browsing, and streaming,
and how do server item ids map to Tinwave `SourceId`?

## Time box
Half a working session. Needs a real Jellyfin instance (PO to provide or use a local Docker instance).

## Acceptance criteria
- [ ] Authenticate, list artists/albums/tracks with pagination, stream one track to a file.
- [ ] Credentials never logged; token storage approach for Windows Credential Manager noted.
- [ ] Draft trait signature and source-identity mapping in the draft ADR.

## Signals & storage
Notes on how `report_playback()` relates to Tinwave signals (spec §4.10).
