---
id: P-0305
title: Dedupe CorruptPackets after a rate restart
from: T-0063 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0077
---
A rate restart re-reads the item from the audible frame; the reader can count packets already reported
(decoder.rs `drop_reader`, `reader.take_corrupt_packets`).
