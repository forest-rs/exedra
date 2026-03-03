---
id: exe-o33y
status: closed
deps: [exe-dey4]
links: []
created: 2026-03-03T12:47:14Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [techdebt, api, understory]
---
# Redesign DirtySet API around deterministic drain semantics

Current Txn/ChangeSet dirty API added in exe-dey4 predates practical understory_dirty integration details. We currently expose deterministic snapshot helpers that allocate+sort (dirty_faces/dirty_vertices/dirty_corners) plus unordered iterators for lower overhead. This split is acceptable for v0.1 but creates contract risk and extra copying in hot paths.

## Design

Redesign Exedra DirtySet surface to make deterministic drain-oriented consumption the primary path, aligned with understory_dirty capabilities. Keep public API deterministic by default for externally-visible outputs while minimizing per-call allocations/sorts. Provide explicit API boundaries between: (1) deterministic/public order-sensitive consumption and (2) internal/perf-oriented unordered access. Ensure naming and docs make misuse difficult. Consider migration helpers and compatibility strategy for cambium/extraction call sites.

## Acceptance Criteria

1) New public DirtySet API centers deterministic drain/consume semantics with clear ordering guarantees. 2) Snapshot allocation helpers are either removed, demoted, or explicitly documented as convenience-only with cost notes. 3) Internal fast-path access cannot be confused with stable-order outputs (naming/visibility/doc constraints). 4) Exedra and Cambium call sites updated to the new contract. 5) Tests cover deterministic ordering guarantees and non-regression of dirty tracking behavior.

