# exedra_spatial

Deterministic spatial indexing primitives for the Exedra workspace.

Current scope:

- `Aabb` utilities for axis-aligned spatial bounds,
- a flat adaptive octree with deterministic child ordering,
- visitor-driven construction,
- deterministic depth-first and breadth-first traversal,
- leaf-neighbor queries by spatial adjacency,
- incremental leaf refinement without rebuilding the whole tree.

This crate is intentionally geometry-agnostic. It does not know about scalar
fields, meshes, or implicit-surface extraction.
