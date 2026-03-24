---
id: ei-fq6w
status: closed
deps: []
links: []
created: 2026-03-24T01:47:03Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
tags: [v1.0]
---
# Implicit field transform wrappers

Add reusable transform wrappers for exedra_isosurface fields so analytic and later backend fields can be translated, rotated, and scaled without bespoke primitive variants.

## Design

Introduce thin generic wrappers around ScalarField that apply inverse transforms at evaluation time. Keep the first slice small and explicit: Translate<F>, UniformScale<F>, Transform3<F>. Transform3 should own a rigid or affine transform expressed in a calm, no_std-friendly representation and should transform gradients correctly back into world space. This stays in exedra_isosurface as field construction infrastructure, not a separate math crate.

## Acceptance Criteria

1. exedra_isosurface exposes documented transform wrappers for ScalarField implementations. 2. Interval, point, and gradient evaluation behave correctly for translated and rotated analytic reference fields. 3. Tests cover at least translated sphere and rotated cylinder/half-space cases. 4. Public docs/examples show how wrappers compose with existing CSG combinators. 5. Full quality gates for the touched crates pass.


## Notes

**2026-03-24T01:53:21Z**

Implemented exedra_isosurface::transform with Translate, UniformScale, RigidTransform3, and Transform3 wrappers plus forwarding SpecializableField/ProvenanceField support. Kept the transform scope rigid-only for now: translation, uniform scaling, and orthonormal frame transforms cover the common field-edit cases without introducing a general implicit scene graph. Updated crate docs/ADR/branch plan to make field construction part of the explicit scope. Validation: typos crates/exedra_isosurface/src/transform.rs crates/exedra_isosurface/src/lib.rs crates/exedra_isosurface/README.md crates/exedra_isosurface/docs/adr-0001-scalar-field-scope.md docs/plans/implicit-surface-branch.md .tickets/ei-fq6w.md .tickets/ei-912r.md .tickets/ei-7kzh.md; cargo fmt --all; cargo test -p exedra_isosurface; cargo clippy -p exedra_isosurface --all-targets --all-features -- -D warnings; cargo doc -p exedra_isosurface --no-deps
