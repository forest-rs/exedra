---
id: exe-7r9n
status: closed
deps: []
links: []
created: 2026-03-04T04:27:24Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, topology]
---
# delete_edge kernel primitive

Implement deterministic edge deletion primitive in Exedra txn API, including boundary/outside restitch handling, attribute cleanup, and stable ChangeSet bookkeeping.

## Design

Expose Txn::delete_edges(&[HalfEdgeId], policy) with canonicalized undirected-edge input semantics; reject stale/non-canonical input; preflight manifold/boundary continuation before mutation; perform deterministic deletion + outside loop restitch + vertex out-pointer repair; clear corner/edge sparse attrs for deleted IDs; record deleted/created topology and dirty marks. Include Mesh convenience wrapper mirroring delete_faces.

## Acceptance Criteria

- Txn::delete_edges and Mesh::delete_edges exist with structured error type
- Deterministic behavior for sorted/canonical edge input
- Preflight catches ambiguous boundary continuation before mutation
- validate_fast + validate_deep pass on success paths
- Tests cover single-edge delete, boundary-edge delete, adjacent multi-edge delete, stale-id/non-canonical errors
