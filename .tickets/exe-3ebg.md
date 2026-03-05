---
id: exe-3ebg
status: closed
deps: []
links: []
created: 2026-03-04T17:42:28Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, topology]
---
# delete_vertices kernel primitive

Add deterministic isolated-vertex deletion to Exedra transaction API. This should only delete vertices with no incident half-edges/faces and reject non-isolated/stale/non-canonical input.

## Design

Expose Txn::delete_vertices(&[VertexId]) and Mesh::delete_vertices(...) convenience wrapper. Input must be canonical (sorted + deduped) and contain only live isolated vertices (out == HalfEdgeId::INVALID and no incident edges). Reject any vertex still referenced by topology. Record deleted vertices in ChangeSet, mark dirty deterministically, and preserve existing eager-transaction semantics.

## Acceptance Criteria

- Txn::delete_vertices and Mesh::delete_vertices implemented with structured error type; - canonical/stale/non-isolated validations covered by tests; - successful deletion updates ChangeSet + dirty tracking deterministically; - validate_fast + validate_deep pass on success paths

