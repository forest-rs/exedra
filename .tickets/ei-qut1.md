---
id: ei-qut1
status: closed
deps: []
links: []
created: 2026-03-24T03:20:24Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
---
# Field-authored corner normals for dual contouring

Improve implicit mesh shading by writing corner normal overrides from field gradients during exedra_isosurface extraction.

## Design

Keep the change inside exedra_isosurface. After building the dual-contoured mesh, define CORNER_NORMAL_OVERRIDE and populate each emitted corner from the best available field gradient for that corner position. Preserve determinism, avoid changing exedra's core render policy, and document that these are extraction-time authored shading normals rather than topology semantics.

## Acceptance Criteria

- dual_contour output includes authored corner normals derived from the source field
- regression tests verify the normal layer exists and contains sensible outward normals on a reference shape
- docs/ticket notes explain the new shading boundary and remaining limitations


## Notes

**2026-03-24T03:24:26Z**

Implemented extraction-time authored corner normals for exedra_isosurface. After building the dual-contoured mesh, the extractor now gathers one sample point per emitted corner by nudging the corner position toward its face centroid, batch-evaluates field gradients at those sample points, normalizes them, and writes CORNER_NORMAL_OVERRIDE through Exedra's public edit/op API. This keeps the shading improvement local to the implicit extractor instead of changing exedra's core render policy, and it lets adjacent faces keep distinct normals around hard features because sampling is face-local rather than vertex-global. Added regression coverage on the sphere path to verify the normal layer exists, normals are unit length, and they point outward. Updated the README and phase-1 ADR, and captured the short execution plan in crates/exedra_isosurface/docs/plans/ei-qut1-field-authored-corner-normals.md. Validation: cargo fmt --all; typos crates/exedra_isosurface/src/dual_contour.rs crates/exedra_isosurface/README.md crates/exedra_isosurface/docs/adr-0002-phase-1-dual-contouring.md crates/exedra_isosurface/docs/plans/ei-qut1-field-authored-corner-normals.md .tickets/ei-qut1.md; cargo test -p exedra_isosurface; cargo clippy -p exedra_isosurface --all-targets --all-features -- -D warnings; cargo doc -p exedra_isosurface --no-deps. Also regenerated a Fidget one-off OBJ as fidget_trig_blob_normals.obj to exercise the new shading path in a viewer.
