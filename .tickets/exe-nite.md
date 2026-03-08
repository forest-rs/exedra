---
id: exe-nite
status: closed
deps: []
links: []
created: 2026-03-08T02:08:17Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Add dissolve_vertices kernel op

Add exedra::op::dissolve_vertices for simplifiable interior vertices.

## Design

Expose exedra::op::dissolve_vertices(session, &[VertexId]) with canonical vertex-set input semantics. Reject stale or non-canonical vertices, reject unsupported boundary/non-manifold/unsimplifiable stars before mutation, and rebuild merged face loops deterministically while preserving authored attrs where possible. Reuse eager edit-scope bookkeeping and do not create a Mesh/EditSession mutation side door.

## Acceptance Criteria

1) exedra::op::dissolve_vertices exists with structured error type. 2) Dissolving one simplifiable interior vertex in a small planar patch produces a valid merged face result. 3) Stale, non-canonical, boundary, and unsupported selections are rejected with typed errors before mutation. 4) Tests cover topology validity and deterministic change tracking.

