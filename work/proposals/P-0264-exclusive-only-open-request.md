---
id: P-0264
title: Skip the wasted shared open on a refused rate
from: T-0057 build-1
date: 2026-11-03
size: S
outcome: accepted → T-0066
---
WasapiOutput opens a shared stream at the refused rate before the engine closes it; an OpenRequest
"exclusive only, fail instead of falling back" flag saves one open. Evidence: output/windows.rs `open_device`.
