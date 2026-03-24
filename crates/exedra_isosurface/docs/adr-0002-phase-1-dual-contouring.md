# ADR-0002: Phase-1 Dual Contouring in `exedra_isosurface`

- Status: Accepted
- Date: 2026-03-24

## Context

The implicit branch now has the field seam, Hermite bridge, QEF solver, and
MeshBuilder attribute tagging needed for a first real extractor.

The full target design still includes harder work:

- topology-optimal variable-depth stitching,
- manifold DC,
- richer provenance and seam tagging,
- refinement strategies beyond the current conservative leaf-retention heuristic.

Waiting for the entire end-state before landing an extractor would delay the
 first point where the workspace can actually turn scalar fields into Exedra
 meshes.

## Decision

`exedra_isosurface` will ship a phase-1 dual-contouring path now with these
constraints:

- build an interval-culled octree via `exedra_spatial`,
- keep a conservative minimum refinement floor, then allow some simpler leaves
  to stop early instead of forcing every intersecting branch to max depth,
- place one vertex per active octree leaf using Hermite samples and
  `exedra_qef`,
- emit explicit triangles from interior sign-changing primal-edge patches on
  the finest lattice, choosing the shorter quad diagonal deterministically
  instead of relying on later fan triangulation,
- stitch mixed-depth transitions conservatively by reusing coarse leaf
  vertices over the finest covered cells, then suppress duplicate triangles and
  any triangle that would push an undirected edge above two incident faces,
- bias low-rank QEF solves toward the Hermite mass point for each active cell
  instead of always anchoring null-space dimensions to the geometric cell
  center,
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
  cylinders, simple CSG references, and multi-scale adaptive leaves.
- The implementation stays structurally honest about what is still missing:
  no manifold guarantees, no topology-optimal variable-depth stitching, only a
  conservative leaf-retention heuristic, and only face-local gradient sampling
  for authored shading normals. The current seam pass marks region boundaries
  only; it is not a full branch-trace recovery.
- Future work can refine this mesher in place rather than starting from a
  purely architectural sketch.
