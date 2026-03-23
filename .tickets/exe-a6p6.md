---
id: exe-a6p6
title: Analytic ScalarField implementations for testing
status: closed
deps: [exe-2r7w]
links: [exe-gosk]
created: 2026-03-04T07:19:34Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
tags: [v1.0]
---
# Analytic ScalarField implementations for testing

Reference ScalarField implementations for analytic primitives (sphere, box, cylinder, torus) and CSG combinators (union, intersection, difference). These serve as test oracles for the DC mesher independent of fidget, and as simple standalone SDF sources.

## Design

Implementations:

Primitives:
- Sphere { center, radius } — exact SDF, analytic gradient
- Box { center, half_extents } — exact SDF, piecewise gradient (NaN at edges/corners of the box SDF itself)
- Cylinder { axis, radius, half_height } — exact SDF
- Torus { center, major_radius, minor_radius } — exact SDF
- HalfSpace { point, normal } — trivial SDF, useful as CSG operand

CSG combinators (implement ProvenanceField):
- Union<A, B> — min(a, b), provenance tracks which operand won
- Intersection<A, B> — max(a, b)
- Difference<A, B> — max(a, -b)
- SmoothUnion<A, B> { k } — smooth minimum with blend radius

All implement:
- eval_interval via interval arithmetic on the analytic formula
- eval_points via direct formula evaluation
- eval_gradients via analytic partial derivatives

These are useful beyond DC testing:
- Quick SDF evaluation without fidget compilation overhead
- Ray marching / sphere tracing against analytic SDFs
- Teaching/documentation examples
- Composition building blocks for procedural geometry

## Acceptance Criteria

- Sphere, Box, Cylinder, Torus, HalfSpace implementations of ScalarField
- Union, Intersection, Difference CSG combinators
- At least SmoothUnion for blended operations
- CSG combinators implement ProvenanceField
- All implementations provide correct interval bounds (verified against brute-force sampling)
- All provide correct gradients (verified against finite differences)
- Unit tests for each primitive and combinator

## Notes

**2026-03-24T16:15:11Z**

Expanded `exedra_isosurface::analytic` from a single sphere reference into a fuller analytic test-oracle layer: `SphereField`, `BoxField`, `CylinderField`, `TorusField`, and `HalfSpaceField`; `TaggedField` for constant provenance; and `Union`, `Intersection`, `Difference`, and `SmoothUnion` combinators with branch-selecting gradients and binary provenance reporting. The primitive intervals are conservative and the tests now check them against brute-force grid samples, while the gradient tests compare the reported derivatives against finite differences at differentiable sample points. Updated crate docs so the public surface reflects the larger analytic module. Validation: `typos crates/exedra_isosurface/src/lib.rs crates/exedra_isosurface/src/analytic.rs crates/exedra_isosurface/src/hermite.rs crates/exedra_isosurface/README.md .tickets/exe-a6p6.md`; `cargo fmt --all`; `cargo test -p exedra_isosurface`; `cargo clippy -p exedra_isosurface --all-targets --all-features -- -D warnings`; `cargo doc -p exedra_isosurface --no-deps`.
