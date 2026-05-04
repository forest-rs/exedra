---
id: exe-qs69
title: Boolean broad phase (AABB/BVH)
status: closed
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

**2026-05-04T17:19:31Z**

Implementation summary: added the public exedra::boolean broad-phase module with Aabb, BooleanBvh, BooleanScratch, BooleanTriangleRef, BooleanCandidatePair, and BooleanBroadPhaseStats; BVHs are built over deterministic fan triangles and query output is sorted for stable downstream processing. Key decisions/tradeoffs: this is an additive public API, so no migration is required; ADR 0007 records that broad phase owns only AABB candidate discovery and explicitly defers narrow phase, classification, splitting, and stitching. Validation: cargo fmt --all; cargo test -p exedra --all-features; cargo test -p exedra --no-default-features --features libm; cargo clippy -p exedra --all-targets --all-features -- -D warnings; cargo doc -p exedra --no-deps; typos crates/exedra/src crates/exedra/README.md crates/exedra/docs .tickets/exe-qs69.md; taplo fmt --check; cargo fmt --all --check; git diff --check.
