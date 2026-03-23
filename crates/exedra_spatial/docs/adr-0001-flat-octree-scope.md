# ADR-0001: Flat Octree Scope

- Status: Accepted
- Date: 2026-03-24
- Owners: Exedra implicit-surface maintainers
- Ticket: `exe-0r1z`

## Context

The implicit-surface branch needs spatial indexing for octree-driven sampling,
but the spatial layer should stay reusable outside isosurface extraction.

## Decision

`exedra_spatial` owns:

- `Aabb`,
- a flat adaptive octree with deterministic child ordering,
- visitor-driven construction,
- deterministic traversal modes,
- spatial neighbor queries over octree cells.

It does not own:

- scalar-field evaluation,
- Hermite sampling,
- QEF solving,
- mesh extraction.

## Consequences

Positive:

- keeps spatial indexing reusable for later ray, culling, and proximity work,
- keeps the implicit extraction stack layered,
- preserves `no_std` viability for the spatial root.

Tradeoffs:

- some octree conveniences needed by later extractors will arrive incrementally,
- neighbor queries are intentionally correctness-first before performance-first.
