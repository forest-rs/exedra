---
id: exe-0gvz
title: MeshBuilder provenance generalization for procedural sources
status: closed
deps: []
links: []
created: 2026-03-04T07:21:34Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
tags: [v1.0]
---
# MeshBuilder provenance generalization for procedural sources

Generalize MeshBuilder's provenance tracking to carry source metadata (which primitive, which CSG operation) through to FACE_REGION and EDGE_SEAM. Currently MeshBuilder supports vertex_ids/face_ids/face_edge_ids as caller-supplied labels. This ticket formalizes the pattern so DC, primitive generators, and any procedural source can tag attributes during construction.

## Design

Current state: MeshBuilder returns MeshBuildResult with provenance maps (vertex_ids, face_ids, face_edge_ids). Primitive generators (box, cylinder, sphere) manually set FACE_REGION after construction. The DC mesher would need to do the same.

Proposal: Add optional callbacks or a builder pattern that allows attribute tagging during face insertion, not as a post-pass. This keeps provenance close to the construction site and avoids a separate loop over faces.

Options:
1. Builder callbacks: MeshBuilder::on_face(|face_id, face_data| { set_region(...) })
2. Attribute channels on builder: MeshBuilder::with_face_attribute(FACE_REGION, |face_index| region_value)
3. Per-face metadata in add_face: MeshBuilder::add_face_with_attrs(verts, attrs)

The right choice depends on how much metadata flows through and whether it's per-face, per-edge, or per-corner. DC needs all three (region per face, sharpness per edge, potentially UVs per corner if parameterization is added later).

This may also feed into the primitive generators — they could use the same mechanism instead of post-construction attribute setting.

## Acceptance Criteria

- MeshBuilder supports attribute tagging during construction (not just post-construction)
- At least FACE_REGION and EDGE_SHARPNESS taggable during build
- Primitive generators updated to use the new mechanism (if the API is cleaner)
- DC mesher uses the mechanism for attribute tagging
- No regression in existing MeshBuilder usage

## Notes

**2026-03-24T16:44:52Z**

Added a narrow build-time tagging surface to `MeshBuilder` via `FaceBuildAttrs` and `add_face_with_attrs(...)`. The builder now validates per-edge metadata lengths before accepting a face, stores optional `FACE_REGION`, seam, and sharpness payloads alongside the face loop, and applies those attributes directly while assembling the final mesh. This keeps procedural metadata at the construction site without introducing a callback system. Also switched representative primitives (`quad`, `grid`, and `box_primitive`) onto the new path for face-region tagging, and added regression tests covering both successful attribute application and length-mismatch rejection. Validation: `typos crates/exedra/src/lib.rs crates/exedra/src/mesh.rs crates/exedra_primitives/src/quad.rs crates/exedra_primitives/src/grid.rs crates/exedra_primitives/src/box_primitive.rs .tickets/exe-0gvz.md`; `cargo fmt --all`; `cargo test -p exedra -p exedra_primitives`; `cargo clippy -p exedra -p exedra_primitives --all-targets --all-features -- -D warnings`; `cargo doc -p exedra -p exedra_primitives --no-deps`.
