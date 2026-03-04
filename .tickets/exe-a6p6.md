---
id: exe-a6p6
title: Analytic ScalarField implementations for testing
status: open
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

