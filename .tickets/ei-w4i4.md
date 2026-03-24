---
id: ei-w4i4
status: closed
deps: []
links: []
created: 2026-03-24T03:06:38Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
---
# Phase-2 dual contouring cleanup

Improve exedra_isosurface meshing quality beyond the phase-1 max-depth lattice extractor. Focus on correctness and visibly cleaner surfaces before benchmarking.

## Design

Own mesher follow-up work after the first field-to-mesh path: tighten face emission/orientation, reduce obvious lattice artifacts where practical, and document the remaining limits before fair benchmarking. Do not take on full manifold or mixed-depth adaptive DC in one step unless the implementation naturally reaches it.

## Acceptance Criteria

- reproduces and fixes at least one current extraction defect visible in the one-off OBJ exports
- improves output behavior or documentation enough to justify deferring benchmark work to a later ticket
- updates ADR or ticket notes to reflect the new extractor boundary and remaining limitations

## Notes

**2026-03-24T03:17:42Z**

Implemented the first post-phase-1 mesher cleanup in exedra_isosurface. The extractor no longer stores primal-edge patches as quads and relies on later fixed-diagonal fan triangulation during render extraction; it now emits explicit triangles during dual-contour face emission and chooses the shorter quad diagonal deterministically. This keeps the change local to the mesher, fixes the hidden warped-quad artifact path seen in the one-off OBJ exports, and makes the actual mesh topology match the rendered topology. Added regression coverage for deterministic diagonal selection and for triangle-only emission on the CSG union case, updated the phase-1 ADR/README, and captured a short implementation plan in crates/exedra_isosurface/docs/plans/ei-w4i4-phase-2-dual-contouring-cleanup.md. Validation: cargo fmt --all; typos crates/exedra_isosurface/src/dual_contour.rs crates/exedra_isosurface/README.md crates/exedra_isosurface/docs/adr-0002-phase-1-dual-contouring.md crates/exedra_isosurface/docs/plans/ei-w4i4-phase-2-dual-contouring-cleanup.md .tickets/ei-w4i4.md; cargo test -p exedra_isosurface; cargo clippy -p exedra_isosurface --all-targets --all-features -- -D warnings; cargo doc -p exedra_isosurface --no-deps. Also regenerated the box-union one-off OBJ as implicit_dual_contour_union_cleanup.obj to verify the updated face emission path outside tests.
