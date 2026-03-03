---
id: exe-1qo5
title: Add MeshRevision (monotonic version counter)
status: closed
deps: [exe-cbv1, exe-dey4]
links: []
created: 2026-03-03T07:24:23Z
type: Add MeshRevision (monotonic version counter)
priority: P1
assignee: Bruce Mitchener
---
# Add MeshRevision (monotonic version counter)

Add a monotonic revision counter to exedra::Mesh that increments on every successful Txn::commit(). Expose as MeshRevision(u64) with Mesh::revision() -> MeshRevision. Intended for cache keys and incremental systems above the kernel (Cambium/operator runtime, render cache, adjacency caches) so they can cheaply detect mesh-changed without diffing IDs or examining ChangeSet contents. Does not replace ChangeSet — this is a cheap scalar for cache invalidation; ChangeSet still carries fine-grained dirtiness and created/deleted IDs.

## Design

MeshRevision is a lightweight Copy + Clone + Debug + Eq + Ord value type wrapping u64. Mesh stores revision: u64, initialized to 0 on construction. Txn::commit() increments revision exactly once per commit. Mesh::revision() returns MeshRevision(self.revision). Increments are deterministic and platform-independent (just +1). Revision is invalidated by compaction by simply being part of the new mesh state (compacted mesh has its own revision value). No special handling needed — it is just a field on Mesh.

## Acceptance Criteria

Mesh stores revision: u64. Txn::commit() increments mesh revision exactly once per commit. Mesh::revision() returns current revision as MeshRevision. Revision increments are deterministic and platform-independent. Document that revision is part of mesh state and carried through compaction naturally.

