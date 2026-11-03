---
id: P-0242
title: Guard the [licenses.private] gap for path crates
from: T-0058 review-1
date: 2026-11-03
size: S
outcome: merged → T-0054
---
`ignore = true` lets a path crate marked `publish = false` skip the allow-list. Git deps are already denied and registry
crates are never private, so only a deliberately added in-repo path crate is exposed. Guard: a check that every path
package in `cargo metadata` is a workspace member under crates/ or apps/.
