---
id: exe-qs69
title: Boolean broad phase (AABB/BVH)
status: open
deps: [exe-tezb, exe-0a9w]
links: []
created: 2026-03-03T05:42:04Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.9, boolean]
---
# Boolean broad phase (AABB/BVH)

Implement broad-phase culling for boolean operations. Reduces candidate triangle pairs from O(n*m) to a manageable set using bounding volume hierarchies.

## Design

AABB tree or BVH over triangulated faces of each input mesh.
- Query: find overlapping face pairs between mesh A and mesh B
- Must report candidate reduction ratio for diagnostics
- Scratch buffer friendly (BVH built into reusable scratch)
- Deterministic: same input produces same candidate list in same order

## Acceptance Criteria

- BVH/AABB construction for a mesh
- Overlap query between two BVHs returns candidate face pairs
- Candidate list is deterministically ordered
- Stats: total pairs, candidates after culling, reduction ratio
- Scratch-friendly (no per-query allocations in hot loop)


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/exedra/docs/briefs/06_staged_booleans_with_artifacts.md
