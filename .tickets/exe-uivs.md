---
id: exe-uivs
status: closed
deps: []
links: []
created: 2026-03-08T01:27:02Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Dissolve edges kernel primitive

Add an Exedra kernel operation that removes selected interior edges and merges adjacent faces when legal, preserving surrounding surface rather than creating holes.

## Design

Expose exedra::op::dissolve_edges(session, &[HalfEdgeId]) with canonical undirected-edge input semantics. Reject stale or non-canonical edges, require exactly two interior incident faces per dissolved edge, and rebuild the merged face loop deterministically. Reuse eager edit-scope bookkeeping and authored attribute cleanup; do not create a public Mesh/EditSession mutation side door.

## Acceptance Criteria

1) exedra::op::dissolve_edges exists with structured error type. 2) Dissolving one interior edge in a two-face patch merges the faces into one valid face. 3) Non-canonical, stale, boundary, and unsupported selections are rejected with typed errors before mutation. 4) Tests cover topology validity and deterministic change tracking.

