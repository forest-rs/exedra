---
id: exe-xgtv
title: Implicit surface meshing epic (dual contouring pipeline)
status: open
deps: []
links: [cam-t6z7, exe-h2rh]
created: 2026-03-04T06:57:13Z
type: epic
priority: 2
assignee: Bruce Mitchener
tags: [v1.0]
---
# Implicit surface meshing epic (dual contouring pipeline)

Epic covering the full implicit surface → exedra Mesh pipeline. Decomposes into independently useful subsystems (spatial index, QEF solver, scalar field trait, DC mesher) plus a thin fidget adapter. Goal: SDF sources produce exedra meshes with full attribute coverage (EDGE_SHARPNESS from QEF rank, FACE_REGION from CSG provenance, EDGE_SEAM at boolean boundaries).

## Design

Architecture decomposes into four reusable crates plus one adapter:

1. exedra_spatial — Adaptive octree, AABB, spatial queries. Visitor-pattern traversal, incremental refinement. Reused by ray tracing, collision detection, proximity queries, frustum culling, BVH construction.

2. exedra_qef — Quadratic Error Function solver with SVD-based rank selection. Outputs vertex placement + sharpness classification (smooth/edge/corner from eigenvalue rank). Reused by mesh simplification (Garland-Heckbert), remeshing, point cloud fitting, feature-preserving smoothing.

3. exedra_isosurface — Dual contouring mesher over a ScalarField trait → exedra::Mesh. Consumes exedra_spatial for the octree and exedra_qef for vertex placement. Hermite data (edge intersection position + gradient) as the shared representation between field evaluation and mesh extraction. Multiple extraction strategies possible (DC, marching cubes) behind the same trait.

4. ScalarField trait — Abstract oracle interface: interval evaluation for cell culling, bulk point evaluation for sign classification, bulk gradient evaluation for hermite data. Fidget is the first backend but any SDF source (3D textures, analytic primitives, voxel grids, RBF point clouds) can implement it.

5. exedra_fidget — Thin adapter: impl ScalarField for fidget::Shape<F>, tape management, simplify() calls during octree traversal. Depends on fidget-core + optionally fidget-jit.

Key design decisions:
- Octree is owned by the mesher, not by fidget. Fidget is a pure evaluation oracle.
- Sharpness is detected during DC vertex placement (QEF eigenvalue rank), not post-hoc.
- CSG provenance tracked via interval evaluation trace (which branch of min/max won).
- MeshBuilder used for output, not from_indexed_triangles, giving full half-edge topology + provenance maps.
- ScalarField trait boundary makes fidget replaceable (tenet 7).

## Acceptance Criteria

- Adaptive octree crate with visitor-based traversal and spatial queries
- QEF solver with SVD rank selection and sharpness classification
- ScalarField trait abstracting implicit surface evaluation
- Dual contouring mesher producing exedra::Mesh with EDGE_SHARPNESS, FACE_REGION, EDGE_SEAM
- Fidget adapter implementing ScalarField
- Integration tests: known SDF → mesh → validate_deep passes
- Sharp feature preservation verified on CSG models with known edge/corner topology

