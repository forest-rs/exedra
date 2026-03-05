# Brief: Operator taxonomy and v0.1 frozen set

## Decision
Cambium adopts a curated operator taxonomy and freezes a minimum v0.1 operator set.

The taxonomy is informed by broader mesh-operator ecosystems (Blender/OpenMesh/CGAL/Unreal-style families), but v0.1 remains intentionally small and deterministic. Exedra stays focused on kernel edits/invariants; Cambium is the workflow/operator surface.

This brief is a contract for ticket sequencing and discoverability work. It does not rename APIs by itself.

## Taxonomy (Cambium SDK surface)

### 1. `inspect.*`
Read-only diagnostics/measurement over mesh state.

### 2. `select.*`
Deterministic selection/query helpers used by operators and fluent flows.

### 3. `tag.*`
Domain tagging/material/region assignment.

### 4. `mark.*`
Authoring tags on existing topology (seams, sharpness).

### 5. `uv.*`
UV projection/writing operators.

### 6. `edit.*`
Topology/attribute editing operators (delete/extrude/inset and future edits).

### 7. `construct.*` (deferred)
Primitive/patch construction operators at Cambium layer.

### 8. `repair.*` (deferred)
Hole-fill, manifold cleanup, orientation/consistency repair.

### 9. `remesh.*` (deferred)
Subdivide/simplify/remesh/relax style operators.

### 10. `boolean.*` (deferred)
Boolean orchestration over Exedra kernels.

## Frozen v0.1 operator set

The following set is frozen as the minimum curated v0.1 surface.

### Inspect
- `inspect.validate.mesh` (`ValidateMesh`)
- `inspect.bounds` (`InspectBounds`)

### Selection/query
- `select.faces.by_region` (`select_faces_by_region` helper)

### Tag/mark
- `tag.face.region` (`TagFaceRegion`)
- `mark.edge.seam` (`MarkEdgeSeam`)
- `mark.edge.sharp` (`MarkEdgeSharp`)

### UV
- `uv.planar` (`UvPlanar`)
- `uv.box` (`UvBox`)
- `uv.cylinder` (`UvCylinder`)

### Edit
- `edit.delete.faces` (`DeleteFaces`)
- `edit.delete.edges` (`DeleteEdges`)
- `edit.delete.vertices` (`DeleteVertices`)
- `edit.face.extrude` (`ExtrudeFaces`)
- `edit.face.inset` (`InsetFaces`)

This frozen set is the basis for:
- naming/alignment work (`cam-suy5`),
- namespace/discoverability pass (`cam-bwcd`),
- rustdoc catalog (`cam-nyws`).

## Explicit defers (v0.5+)

### Face-edit semantic expansion
- `cam-xnoi` (semantics matrix)
- `cam-1xjc` (adjacent multi-face support)
- `cam-7u7l` (extrude modes)

### Region/selection growth
- `cam-5xnn` (region loop/flood operations)

### Construction/profile/sweep
- `cam-9eop` (profile section model)
- `cam-d1td` (loft)
- `cam-j034` (sweep)

### Remesh/subdivision/normals
- `cam-inpo` (Catmull-Clark)
- `cam-26fl` (dart handling)
- `exe-o4iu` / `exe-z9pv` (derived/custom normals)

### Boolean pipeline
- `cam-kksz` with Exedra boolean tickets (`exe-qs69`, `exe-h16i`, `exe-ikdf`, `exe-imti`, `exe-qa74`, `exe-v41q`)

### Performance/quality guardrails
- `cam-v9q2` (cambium wind tunnel)

## Implications
- New v0.1 features should extend this set conservatively and only with explicit ticket-level justification.
- Naming changes are performed in `cam-suy5`.
- Discoverability and rustdoc catalog work (`cam-bwcd`, `cam-nyws`) must use this taxonomy/frozen set as source of truth.

## Non-goals / deferrals
- Defining all future operators now.
- Full semantic canonicalization across equivalent operator sequences.
- Moving Exedra kernel APIs to mirror operator families.
