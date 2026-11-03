---
id: P-0262
title: Wiper runs a best-effort TRUNCATE checkpoint after a wipe
from: T-0061 review-1
date: 2026-11-03
size: S
outcome: accepted → T-0065
---
Makes "zeroed in the file" true when the wipe returns, not up to ~60 s later.
