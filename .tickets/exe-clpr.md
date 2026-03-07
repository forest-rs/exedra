---
id: exe-clpr
status: open
deps: [exe-43xn]
links: []
created: 2026-03-07T15:14:25Z
type: feature
priority: 1
assignee: Bruce Mitchener
parent: exe-pdum
tags: [v0.1, architecture, api]
---
# Move topology op bodies into exedra::op

Move topology operation bodies out of session/mod.rs into the op modules so exedra::op owns the operation definitions rather than thin wrappers over *_impl methods.

## Design

Keep session/* as bookkeeping/helper plumbing only. Move add-face/split/delete algorithms into op/* one operation at a time, calling internal session helpers for dirty/change recording, cache invalidation, propagation, and shared topology utilities. Remove the remaining *_impl operation bodies from session/mod.rs once migrated.

## Acceptance Criteria

1) Topology op bodies live in op/* modules. 2) session/mod.rs no longer contains the large operation bodies. 3) Shared helper seam remains centralized in session/*. 4) Tests and docs stay green.
