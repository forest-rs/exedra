---
id: exe-2r7w
title: ScalarField trait for implicit surface evaluation
status: closed
deps: []
links: []
created: 2026-03-04T07:05:00Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
tags: [v1.0]
---
# ScalarField trait for implicit surface evaluation

Define the abstract oracle trait that decouples isosurface extraction from any specific SDF backend. This is the key abstraction boundary — fidget, analytic primitives, voxel grids, point cloud RBFs, and 3D textures all implement the same trait.

## Design

Trait definition:

```rust
/// A scalar field that can be evaluated for isosurface extraction.
///
/// Implementors provide distance-like values where the zero level set
/// defines the surface. Negative values are inside, positive outside.
pub trait ScalarField {
    /// Evaluate conservative interval bounds for a spatial region.
    ///
    /// Returns None if the field cannot provide interval bounds (falls
    /// back to subdivision without culling). The interval must be
    /// conservative: the true range within the AABB must be contained
    /// within the returned interval.
    ///
    /// Used for octree cell culling: if interval.lower > 0, cell is
    /// entirely outside; if interval.upper < 0, entirely inside.
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]>;

    /// Bulk evaluation of distance values at given points.
    ///
    /// Output slice must have same length as input. Used for corner
    /// sign classification and edge zero-crossing bisection search.
    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]);

    /// Bulk evaluation of distance values and gradients at given points.
    ///
    /// Output: [value, dx, dy, dz] per point. Used for hermite data
    /// at edge intersections (surface position + normal).
    /// 
    /// NaN gradients indicate non-differentiable points (sharp features
    /// in the SDF itself, e.g., CSG min/max corners). The mesher should
    /// handle these gracefully (e.g., snap to intersection point).
    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]);
}
```

Optional extension trait for tape-style optimization:

```rust
/// Extension for scalar fields that support region-based specialization.
///
/// After interval evaluation narrows the active region, the field can
/// produce a simplified version of itself that is cheaper to evaluate
/// within that region. This is how fidget's tape pruning works.
pub trait SpecializableField: ScalarField {
    type Specialized: ScalarField;
    
    /// Produce a specialized field for a sub-region, if beneficial.
    fn specialize(&self, bounds: &Aabb) -> Option<Self::Specialized>;
}
```

Optional extension trait for CSG provenance:

```rust
/// Extension for scalar fields that track CSG provenance.
///
/// During interval evaluation, tracks which branch of CSG operations
/// (min/max → union/intersection) dominated. This information can be
/// used to assign FACE_REGION and EDGE_SEAM during mesh extraction.
pub trait ProvenanceField: ScalarField {
    type Provenance;
    
    /// Evaluate interval with provenance tracking.
    fn eval_interval_with_provenance(&self, bounds: &Aabb) 
        -> Option<([f32; 2], Self::Provenance)>;
    
    /// Query provenance at a specific point.
    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance;
}
```

Design decisions:
- Bulk evaluation API (slices, not single points) for SIMD friendliness.
- Output via mutable slice, not return value, to allow caller to manage allocation.
- Interval evaluation returns Option to gracefully degrade when a backend can't provide bounds.
- NaN gradient convention for non-differentiable points documented in the trait contract.
- SpecializableField and ProvenanceField are separate extension traits, not required for basic operation.
- Trait is object-safe for the base ScalarField (no associated types or generics in required methods).

This trait lives in exedra_isosurface (or a small exedra_scalar_field crate if we want it independent of the mesher).

## Acceptance Criteria

- ScalarField trait with eval_interval, eval_points, eval_gradients
- SpecializableField extension trait for tape-style optimization
- ProvenanceField extension trait for CSG provenance tracking
- Trait is object-safe (base ScalarField)
- At least one simple reference implementation (analytic sphere or box) for testing
- Documentation with usage examples for each method
- Design validated against fidget's API surface to ensure the adapter is thin


## Notes

**2026-03-23T19:22:59Z**

Added a new exedra_isosurface crate that owns the ScalarField seam plus SpecializableField and ProvenanceField extension traits, wired to exedra_spatial::Aabb. Landed a tiny public analytic::SphereField as the first reference implementation so the base trait is exercised as an object-safe, documented evaluation boundary from day one. Added crate docs and a crate-local ADR documenting that this crate currently owns implicit-field evaluation seams rather than a full mesher. Validation: typos crates/exedra_isosurface/src/lib.rs crates/exedra_isosurface/src/analytic.rs crates/exedra_isosurface/README.md crates/exedra_isosurface/docs/adr-0001-scalar-field-scope.md .tickets/exe-2r7w.md; cargo fmt --all; cargo test -p exedra_isosurface; cargo clippy -p exedra_isosurface --all-targets --all-features -- -D warnings; cargo doc -p exedra_isosurface --no-deps.
