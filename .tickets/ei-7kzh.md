---
id: ei-7kzh
status: closed
deps: [ei-fq6w, ei-912r]
links: []
created: 2026-03-24T01:47:25Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
tags: [v1.0]
---
# Lift 2D profiles into 3D implicit fields

Add the first profile-based 3D constructors for exedra_isosurface so simple 2D fields can produce useful 3D implicit geometry without introducing a full implicit scene graph.

## Design

Add explicit lifting operators in exedra_isosurface: Extrude<F2d> and Revolve<F2d>. Extrude should map a planar 2D profile into a finite-height 3D signed-distance field. Revolve should rotate a 2D radius/height profile around a canonical axis and produce a 3D field suitable for meshing. Keep the first pass axis-aligned and deterministic; avoid general swept surfaces or arbitrary frame math in this ticket. The operators should compose with the transform wrappers rather than duplicating orientation logic.

## Acceptance Criteria

1. exedra_isosurface exposes documented Extrude and Revolve field constructors built on ScalarField2d. 2. Tests cover basic profile extrusion and revolution, including gradient sanity on representative points. 3. At least one integration-style test shows a lifted field working with the existing dual contouring path. 4. Public docs/examples show the intended composition with transforms and CSG. 5. Full quality gates for the touched crates pass.


## Notes

**2026-03-24T02:01:03Z**

Implemented a dedicated lift module with Extrude and Revolve field constructors layered on ScalarField2d. Kept the first pass axis-aligned by design: Extrude operates along Z and Revolve around Y, with Transform3 handling later orientation so lift operators stay small and deterministic. Added direct equivalence tests against the existing cylinder and torus reference fields plus a dual-contouring integration-style test for an extruded profile. Updated crate docs and ADR-0001 to make lifting part of the explicit field-construction scope. Validation: typos crates/exedra_isosurface/src/lift.rs crates/exedra_isosurface/src/lib.rs crates/exedra_isosurface/README.md crates/exedra_isosurface/docs/adr-0001-scalar-field-scope.md .tickets/ei-7kzh.md; cargo fmt --all; cargo test -p exedra_isosurface; cargo clippy -p exedra_isosurface --all-targets --all-features -- -D warnings; cargo doc -p exedra_isosurface --no-deps
