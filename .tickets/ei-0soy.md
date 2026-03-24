---
id: ei-0soy
status: closed
deps: []
links: []
created: 2026-03-24T03:25:56Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
---
# Tag dual contour seams from face-region discontinuities

Add a first EDGE_SEAM pass to exedra_isosurface by marking extracted edges whose adjacent faces carry different FACE_REGION values.

## Design

Keep seam tagging local to exedra_isosurface after mesh build. Reuse existing FACE_REGION output and Exedra's public edge-seam edit op; do not attempt richer branch-trace seam recovery yet. The goal is a truthful first semantic boundary pass that works for provenance-tagged fields and degrades to no seams when all regions match.

## Acceptance Criteria

- dual_contour_with_regions marks EDGE_SEAM on shared edges between differing face regions
- regression tests cover a tagged CSG case that produces at least one seam
- docs/ticket notes explain that this is region-boundary seam tagging, not full branch-trace recovery

## Notes

**2026-03-24T03:28:37Z**

Implemented a first EDGE_SEAM pass for provenance-tagged dual contour output. After build and after the existing corner-normal post-pass, exedra_isosurface now walks shared interior edges, compares the adjacent FACE_REGION values, and writes EDGE_SEAM through Exedra's public edge-seam edit op whenever the two regions differ. This keeps the seam story honest: it is region-boundary seam tagging derived from face provenance, not a richer branch-trace recovery. Added a regression test using a locally axis-tagged union over the existing overlapping-box CSG shape to verify both region IDs appear and at least one seam edge is emitted. Updated the README and phase-1 ADR, and captured the short execution plan in crates/exedra_isosurface/docs/plans/ei-0soy-region-boundary-seams.md. Validation: cargo fmt --all; typos crates/exedra_isosurface/src/dual_contour.rs crates/exedra_isosurface/README.md crates/exedra_isosurface/docs/adr-0002-phase-1-dual-contouring.md crates/exedra_isosurface/docs/plans/ei-0soy-region-boundary-seams.md .tickets/ei-0soy.md; cargo test -p exedra_isosurface; cargo clippy -p exedra_isosurface --all-targets --all-features -- -D warnings; cargo doc -p exedra_isosurface --no-deps.
