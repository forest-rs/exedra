---
id: cam-l8n1
status: open
deps: []
links: [exe-dey4]
created: 2026-03-03T05:57:27Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# understory_dirty integration and channel definitions

Wire understory_dirty into Cambium for fine-grained, multi-channel operator-runtime cache dirtiness. Define the initial fixed channel set.

## Design

Initial channels (fixed enum, expand with justification):
- Selection: selection/region cache invalidation
- Adjacency: adjacency/one-ring cache invalidation
- UvDerived: Cambium UV-related derived caches (not authored UV layer)
- OperatorCache: generic operator-local caches

Rules:
- Channels defined centrally (no ad-hoc per-operator channels)
- Channel additions must document memory impact
- Prefer face granularity for large meshes
- Exedra remains source of truth for topology/attribute dirtiness
- Cambium dirty tracking is for operator-runtime caches and UI/workflow state

Module layout: dirty.rs

## Acceptance Criteria

- Channel enum defined with initial channels
- understory_dirty wired as dependency
- Dirty tracking usable from operator code
- Documentation of per-channel memory impact
- Unit tests for channel set/clear/query


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/cambium/docs/briefs/05_understory_dirty_channels_for_caches.md

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — after UV ops mark UvDerived, after topology edits mark Adjacency, after selections mark Selection.
