---
id: exe-pdum
status: closed
deps: []
links: []
created: 2026-03-07T15:14:09Z
type: epic
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, architecture, api]
---
# Strong Exedra mutation fence and op ownership

Finish the strong Exedra mutation boundary so public mutation flows through exedra::op and session/* shrinks to transaction hosting and plumbing.

## Design

Fence: MeshBuilder owns construction, Mesh owns storage/traversal/validation/extraction/session creation, EditSession owns transaction hosting/bookkeeping/internal mutation plumbing, and exedra::op owns public mutation entry points. Remove remaining public mutation helpers and move op bodies out of session/mod.rs into op/* modules.

## Acceptance Criteria

1) Public mutation entry points live in exedra::op. 2) Remaining public mutation helpers on Mesh/EditSession are removed or demoted internal. 3) Topology op bodies live in op/* rather than session/mod.rs wrappers. 4) Cambium composes through exedra::op. 5) ADR/docs match the strong fence.

