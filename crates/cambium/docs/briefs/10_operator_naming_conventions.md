# Brief: Operator naming conventions and stable ID alignment

## Decision
Cambium operator IDs use a stable dot-separated grammar:

- `<family>.<domain>.<action>` for editing/inspection families
- `<family>.<projection>` for UV projection families
- `<family>.<domain>.<attribute>` for mark/tag families

Allowed v0.1 families are frozen by `09_operator_taxonomy_v01_freeze.md`:
- `edit`
- `inspect`
- `mark`
- `tag`
- `uv`

## Naming grammar

### Stable operator ID (`EditOperator::name`)
- lowercase ASCII segments
- `.` separator
- no spaces / no uppercase / no hyphens
- stable once published

Examples:
- `edit.delete.faces`
- `edit.face.extrude`
- `inspect.validate.mesh`
- `mark.edge.seam`
- `tag.face.region`
- `uv.planar`

### Rust type naming
- operator type: `PascalCase` verb phrase (`DeleteFaces`, `InsetFaces`)
- params type: `<Operator>Params`
- output type: `<Operator>Output`
- plan type (when custom): `<Operator>Plan`

### Module placement
- `delete.rs` for `edit.delete.*`
- `face_edit.rs` for `edit.face.*`
- `bounds.rs` / `validate.rs` for `inspect.*`
- `seam.rs` / `sharp.rs` for `mark.*`
- `region.rs` for `tag.*` and region-query helpers
- `uv_*.rs` for `uv.*`

## Audit (v0.1 frozen set)

- `DeleteEdges` -> `edit.delete.edges` (aligned)
- `DeleteFaces` -> `edit.delete.faces` (aligned)
- `DeleteVertices` -> `edit.delete.vertices` (aligned)
- `ExtrudeFaces` -> `edit.face.extrude` (aligned)
- `InsetFaces` -> `edit.face.inset` (aligned)
- `InspectBounds` -> `inspect.bounds` (aligned)
- `ValidateMesh` -> `inspect.validate.mesh` (aligned)
- `MarkEdgeSeam` -> `mark.edge.seam` (aligned)
- `MarkEdgeSharp` -> `mark.edge.sharp` (aligned)
- `TagFaceRegion` -> `tag.face.region` (aligned)
- `UvPlanar` -> `uv.planar` (aligned)
- `UvBox` -> `uv.box` (aligned)
- `UvCylinder` -> `uv.cylinder` (aligned)

## Rename outcome
No operator renames are required by this pass for the current frozen v0.1 set.

Follow-up discoverability/catalog work should use this naming contract:
- `cam-bwcd`
- `cam-nyws`

