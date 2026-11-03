---
id: P-0284
title: Read the exclusive setting through the engine instead of the controller/setting.rs wrapper
from: T-0067 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0072
---
T-0067 wraps the output in `Arc<Mutex<Box<dyn AudioOutput>>>` (controller/setting.rs) so the controller can read
`wants_exclusive()`, because engine.rs/output were being edited by T-0066. A forwarding `Engine::wants_exclusive()`
removes the wrapper and its "must forward every AudioOutput method" hazard.
