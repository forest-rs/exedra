---
id: exe-5rwj
title: Hermite data representation for isosurface extraction
status: closed
deps: [exe-2r7w]
links: []
created: 2026-03-04T07:06:19Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
tags: [v1.0]
---
# Hermite data representation for isosurface extraction

Define the hermite data types that bridge scalar field evaluation and mesh extraction. Edge intersection position + surface gradient, stored per octree edge. This representation is the shared language between the field oracle and the DC mesher.

## Design

Core types:

```rust
/// Hermite data at an edge-surface intersection.
pub struct HermiteIntersection {
    /// Position of the zero-crossing on the edge.
    pub position: [f32; 3],
    /// Surface normal (gradient direction) at the intersection.
    /// May be [NaN; 3] for non-differentiable features.
    pub normal: [f32; 3],
    /// Parametric t along the edge (0.0 = start, 1.0 = end).
    pub t: f32,
}

/// Hermite data for an octree cell, collected from all sign-change edges.
pub struct CellHermiteData {
    /// Intersection data for each sign-change edge of the cell.
    /// Indexed by edge index (0..12 for a cube cell).
    pub intersections: SmallVec<[HermiteIntersection; 4]>,
    /// Corner sign mask (bit i set = corner i is inside).
    pub corner_signs: u8,
}
```

Edge intersection search:
- N-ary bisection along cell edges with sign changes.
- Configurable search depth (fidget uses depth 4 with branching factor 16).
- Uses ScalarField::eval_points for bulk bisection, then ScalarField::eval_gradients at the final intersection point.

Storage:
- Per-cell hermite data stored in the octree cell payload.
- Shared edges between adjacent cells reference the same intersection data (or re-evaluate — design decision based on memory vs compute tradeoff).

Reuse beyond DC:
- Surface-surface intersection curve extraction.
- Offset surface generation (shift intersections along normals).
- Sharp feature network extraction independent of meshing (detect edge/corner features from hermite data without building a mesh).
- Adaptive sampling — hermite data quality drives further octree refinement.

## Acceptance Criteria

- HermiteIntersection type with position, normal, parametric t
- CellHermiteData aggregating per-cell edge intersections and corner signs
- Edge bisection search using ScalarField bulk evaluation
- Configurable bisection depth
- Handles NaN gradients at non-differentiable intersections
- Unit tests with analytic sphere (known intersection positions and normals)


## Notes

**2026-03-23T19:25:21Z**

Expanded exedra_isosurface with Hermite bridge types: HermiteIntersection, CellHermiteData, deterministic per-edge tagging, and a configurable bisection-based locate_edge_intersection helper that uses ScalarField bulk evaluation and preserves NaN gradients. Updated the crate README and ADR so the crate now explicitly owns the bridge layer between field evaluation and later extraction. Validation: cargo fmt --all; cargo test -p exedra_isosurface; cargo clippy -p exedra_isosurface --all-targets --all-features -- -D warnings; cargo doc -p exedra_isosurface --no-deps.
