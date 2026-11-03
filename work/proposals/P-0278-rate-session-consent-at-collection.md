---
id: P-0278
title: RateSession consumer re-checks Speed consent; queue cleared on toggle-off or wipe
from: T-0062 review-1
date: 2026-11-03
size: S
outcome: merged → T-0070
---
Values queued before an opt-out must not be stored after it. Evidence: controller/rate_session.rs:35.
