---
id: P-0321
title: Record the rate restart index before the callback that plays new audio
from: T-0063 review-2
date: 2026-11-03
size: S
outcome: accepted → T-0080
---
Under load the restart index lands one 256-frame callback late; it causes the click test's load-dependent margin.
