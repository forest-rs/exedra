# ADR-0002: Phase-1 Dual Contouring in `exedra_isosurface`

- Status: Accepted
- Date: 2026-03-24

## Context

The implicit branch now has the field seam, Hermite bridge, QEF solver, and
MeshBuilder attribute tagging needed for a first real extractor.

The full target design still includes harder work:

- variable-depth stitching,
- manifold DC,
- richer provenance and seam tagging,
- refinement strategies beyond max-depth-on-intersection.

Waiting for the entire end-state before landing an extractor would delay the
 first point where the workspace can actually turn scalar fields into Exedra
 meshes.

## Decision

`exedra_isosurface` will ship a phase-1 dual-contouring path now with these
constraints:

- build an interval-culled octree via `exedra_spatial`,
- subdivide intersecting cells to a configured max depth,
- place one vertex per active max-depth cell using Hermite samples and
  `exedra_qef`,
- emit explicit triangles from interior sign-changing primal-edge patches on
  that regular max-depth lattice, choosing the shorter quad diagonal
  deterministically instead of relying on later fan triangulation,
- author corner-normal overrides from field gradients using a face-local inset
  sample so render extraction can shade smooth regions more honestly,
- tag `EDGE_SHARPNESS` from the QEF rank and optionally tag `FACE_REGION` when
  the field implements `ProvenanceField<Provenance = u32>`,
- derive a first `EDGE_SEAM` pass from post-build face-region discontinuities
  on shared interior edges,
- expose a deterministic `cell_budget` cap even though it may truncate the
  extracted surface.

## Consequences

- The workspace now has a real field-to-mesh path for spheres, boxes,
  cylinders, and simple CSG references.
- The implementation stays structurally honest about what is still missing:
  no manifold guarantees, no mixed-depth stitching, no seam attribution yet,
  only local diagonal cleanup for warped primal-edge patches, and only
  face-local gradient sampling for authored shading normals. The current seam
  pass marks region boundaries only; it is not a full branch-trace recovery.
- Future work can refine this mesher in place rather than starting from a
  purely architectural sketch.
