---
id: exe-0r1z
title: Adaptive octree and spatial index crate (exedra_spatial)
status: closed
deps: []
links: []
created: 2026-03-04T07:02:29Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
tags: [v1.0]
---
# Adaptive octree and spatial index crate (exedra_spatial)

Standalone adaptive octree crate with AABB primitives and visitor-pattern traversal. This is the spatial backbone for dual contouring, but designed for broad reuse: ray-mesh intersection, spatial queries, proximity testing, BVH for collision, frustum culling.

## Design

Core types:
- Aabb — axis-aligned bounding box with split, contains, intersects, surface area heuristic.
- OctreeCell — node in the adaptive octree. Stores bounds, depth, child indices, payload slot.
- Octree<P> — generic octree parameterized by cell payload type P. Leaf cells carry P, branch cells carry child indices.
- OctreeVisitor trait — visitor pattern for traversal. Methods: should_subdivide(cell, depth) -> bool, process_leaf(cell) -> P. Allows the consumer to control adaptive depth, cell budget, and what data is stored per cell.
- Incremental refinement — ability to refine specific cells without rebuilding the whole tree. Supports budgeted work (tenet 3).

Traversal modes:
- Depth-first with early termination (for culling)
- Breadth-first level-by-level (for LOD)
- Neighbor queries — find adjacent cells at same or different depth (needed by DC for edge intersections across cell boundaries)

Design constraints:
- no_std compatible (alloc only)
- No SDF/field dependency — purely geometric spatial index
- Cell storage in a flat arena (Vec<OctreeCell>) with index-based addressing, not pointer-based tree
- Designed for cache-friendly traversal

Reuse cases beyond DC:
- Ray casting / ray-mesh intersection acceleration
- Collision detection broad phase
- Frustum culling for rendering
- Spatial hashing for proximity queries
- Point-in-mesh testing
- Voxelization

## Acceptance Criteria

- Aabb type with split, contains, intersects, merge, surface area
- Adaptive octree with configurable max depth and visitor-driven subdivision
- Depth-first and breadth-first traversal
- Cell neighbor queries (face-adjacent, edge-adjacent)
- Incremental cell refinement without full rebuild
- no_std compatible
- Unit tests for subdivision, neighbor queries, traversal order determinism
- Benchmarks for construction and traversal at various depths


## Notes

**2026-03-23T19:19:00Z**

Added a new no_std exedra_spatial crate with Aabb utilities, a flat adaptive Octree, visitor-driven construction, deterministic DFS/BFS traversal, leaf-neighbor queries by adjacency, and incremental leaf refinement without full rebuild. Also added crate docs plus a crate-local ADR documenting that exedra_spatial owns spatial indexing but not scalar-field or extraction semantics. Validation: typos crates/exedra_spatial/src/lib.rs crates/exedra_spatial/README.md crates/exedra_spatial/docs/adr-0001-flat-octree-scope.md .tickets/exe-0r1z.md docs/plans/implicit-surface-branch.md; cargo fmt --all; cargo test -p exedra_spatial; cargo clippy -p exedra_spatial --all-targets -- -D warnings; cargo doc -p exedra_spatial --no-deps.
