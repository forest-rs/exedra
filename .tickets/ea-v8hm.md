---
id: ea-v8hm
status: closed
deps: []
links: []
created: 2026-03-16T02:30:32Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: cam-t6z7
tags: [analytic, topology, openings]
---
# Analytic shells with explicit openings

Strengthen the exedra_analytic spike by letting one analytic face carry an explicit opening/hole instead of representing frames as several faces.

## Design

Extend the planar analytic MVP to represent an outer loop plus opening loops for a face, then tessellate deterministically into Exedra mesh. Keep scope narrow: planar faces and line edges only.

## Acceptance Criteria

1. Analytic face topology can represent at least one opening loop. 2. Tessellation handles the opening deterministically. 3. Tests cover the opening case. 4. Existing simple planar cases keep working.

## Notes

**2026-03-16T03:38:00Z**

Extended `exedra_analytic` so `PlanarFace` owns `outer + openings`, added `AnalyticShellBuilder::add_planar_face_with_openings`, and taught `to_exedra_mesh` to keep simple faces as single polygons while triangulating opening-bearing faces deterministically. `rect_frame_xy` now represents one analytic face with one opening instead of four separate faces, and the crate-local ADR now reflects that widened MVP scope.

Migration note:
- `cambium::convert::AnalyticToMeshOutput::mesh_face_for` was replaced by `mesh_faces_for` because analytic faces with openings now tessellate to multiple mesh faces.
- `exedra_analytic::PlanarFace` is no longer `Copy`; callers should clone it explicitly if needed.

Validation:
- `typos crates/exedra_analytic/src/lib.rs crates/exedra_analytic/docs/adr-0001-planar-mvp-scope.md crates/cambium/src/convert.rs .tickets/ea-v8hm.md`
- `taplo fmt`
- `cargo fmt --all`
- `cargo test -p exedra_analytic --all-features`
- `cargo test -p cambium --all-features`
- `cargo clippy -p exedra_analytic -p cambium --all-targets --all-features -- -D warnings`
- `cargo doc -p exedra_analytic -p cambium --no-deps`
