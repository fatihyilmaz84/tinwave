---
id: P-0019
title: .gitattributes — CRLF for *.bat/*.cmd, explicit binary for more media/font/db types
from: T-0012 reviewer
date: 2026-11-01
size: S
outcome: merged → T-0015 (must land before the first .bat/.cmd script)
---
## Why
cmd.exe misparses LF-only batch labels; explicit binary avoids relying on content sniffing.
## Suggested approach
`*.bat text eol=crlf`, `*.cmd text eol=crlf`; `binary` for jpeg/gif/webp/ttf/otf/aac/opus/sqlite/db-wal/db-shm.
