---
id: P-0275
title: Store RateSession in the play record and feed the taste engine
from: T-0062 build-1
date: 2026-11-03
size: S
outcome: merged → T-0070
---
Today the value is produced but nothing reads `take_rate_sessions()`. Evidence: controller/rate_session.rs, play table.
