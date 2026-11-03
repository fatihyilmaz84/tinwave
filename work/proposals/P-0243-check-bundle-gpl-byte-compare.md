---
id: P-0243
title: check-bundle compares Tinwave's GPL text byte-for-byte with the root LICENSE
from: T-0058 review-1
date: 2026-11-03
size: S
outcome: merged → T-0054
---
check-bundle.ps1:197-206 only matches the GPL heading and rejects a fixed list of names; compare the hash with the
root LICENSE and reject any other licence file presented as Tinwave's.
