---
id: exe-dey4
status: open
deps: [exe-cbv1]
links: [cam-l8n1]
created: 2026-03-03T05:30:56Z
type: feature
priority: 0
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# Transactions, ChangeSet, and DirtySet

Implement the mutation boundary contract. All topology/attribute mutations happen inside an explicit transaction. Transactions produce a ChangeSet on commit, which includes a DirtySet. DirtySet is built on understory_dirty (ADR-0002).

## Design

Transaction model:
- Txn borrows Mesh mutably (single-writer)
- Txn records what changed during mutation
- Txn::commit(self) -> ChangeSet

ChangeSet contains:
- dirty: DirtySet
- created_vertices, created_half_edges, created_faces: Vec<Id>
- deleted_vertices, deleted_half_edges, deleted_faces: Vec<Id>

Deterministic ordering rule: all created_*/deleted_* lists are in stable deterministic order (arena slot order or increasing stable ID order). No hash iteration leakage.

DirtySet semantics:
- Conservative: mark more dirty rather than less
- dirty_faces: faces whose triangulation cache is invalid
- dirty_vertices: vertices whose derived data depends on one-ring
- dirty_corners: corners whose corner-domain derived data is invalid
- Built on understory_dirty primitives

DirtySet is consumed by:
- Exedra render extraction (incremental mode)
- Cambium operator caches
- Higher-layer UI/viewport systems

Mutation operations on Txn are the only way to modify topology. This is the enforcement point for invariant preservation.

## Acceptance Criteria

- Txn type exists, borrows Mesh mutably
- Txn::commit() produces a ChangeSet
- ChangeSet contains DirtySet and created/deleted element lists
- All lists are deterministically ordered
- DirtySet integrates with understory_dirty
- understory_dirty is wired as a dependency
- Unit tests for transaction lifecycle and dirty tracking

