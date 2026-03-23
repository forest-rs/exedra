---
id: exe-gosk
title: Dual contouring mesher crate (exedra_isosurface)
status: closed
deps: [exe-0r1z, exe-5y1f, exe-2r7w, exe-5rwj, exe-0gvz]
links: [exe-a6p6]
created: 2026-03-04T07:08:40Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
tags: [v1.0]
---
# Dual contouring mesher crate (exedra_isosurface)

The core dual contouring mesher that ties together octree traversal, hermite data extraction, QEF vertex placement, and exedra Mesh output. Consumes a ScalarField and produces an exedra::Mesh with full attribute coverage.

## Design

Pipeline:

1. Build adaptive octree via exedra_spatial, driven by ScalarField::eval_interval for cell culling. If the field implements SpecializableField, use specialize() to prune evaluation tapes for child cells.

2. Evaluate hermite data for leaf cells: corner signs via eval_points, edge intersections via bisection + eval_gradients. Store as CellHermiteData in cell payload.

3. QEF vertex placement: feed each cell's hermite data to QefSolver. Get back vertex position + SharpnessClass. Store vertex per cell (or multiple vertices per cell for manifold DC).

4. Dual mesh extraction: walk adjacent cells sharing sign-change edges. For each such edge, connect the vertices of the 4 (or fewer, at boundaries) cells sharing that edge into a quad (or triangle at T-junctions between depth levels).

5. Output via MeshBuilder: each DC face becomes a MeshBuilder face. Vertex positions from QEF solutions. Attribute tagging:
   - EDGE_SHARPNESS from QEF SharpnessClass + eigenvalue ratios
   - FACE_REGION from ProvenanceField if available
   - EDGE_SEAM where FACE_REGION changes across an edge

Manifold DC considerations:
- Schaefer et al. manifold variant: cells with complex topology (multiple surface sheets) get multiple vertices.
- Collapsibility check during octree simplification: only merge child cells if the resulting cell has the same topological type.
- This is the most complex part. Could start with simple DC (one vertex per cell, possibly non-manifold) and upgrade to manifold DC later.

Adaptive depth strategy:
- Min/max depth bounds
- Curvature-driven refinement: cells where hermite normals vary significantly get subdivided further
- Feature-driven refinement: cells classified as Edge or Corner by QEF get subdivided to capture the feature
- Budget-based: max total cell count for incremental/bounded work

Output quality:
- validate_deep on output mesh
- Triangle count and vertex count in extraction stats
- Optional post-process: collapse short edges, flip edges for Delaunay-like quality

Phasing:
- Phase 1: Simple DC (one vertex per cell, may produce non-manifold output)
- Phase 2: Manifold DC with collapsibility checks
- Phase 3: Adaptive refinement, curvature-driven, budget-based
- Phase 4: CSG provenance → FACE_REGION + EDGE_SEAM

## Acceptance Criteria

- Consumes ScalarField, produces exedra::Mesh
- Adaptive octree subdivision with interval-based culling
- QEF vertex placement with sharpness classification
- EDGE_SHARPNESS tagged from QEF rank
- Output mesh passes validate_deep
- Sharp feature preservation on test cases (cube, cylinder, CSG union)
- Deterministic output for fixed input
- Configurable max depth, cell budget, eigenvalue cutoff
- Integration tests with analytic ScalarField implementations

## Notes

**2026-03-24T17:22:31Z**

Landed a phase-1 dual-contouring path in `exedra_isosurface::dual_contour`. The implementation builds an interval-culled octree through `exedra_spatial`, samples a regular max-depth lattice for corner signs, gathers Hermite intersections per active cell, solves one bounded QEF per cell, and emits quads from interior sign-changing primal edges into `exedra::Mesh`. `EDGE_SHARPNESS` is tagged from the QEF rank, `FACE_REGION` can be sourced from `ProvenanceField<Provenance = u32>`, and a deterministic `cell_budget` cap is exposed for bounded work. The current design is intentionally phase-1: no variable-depth stitching, manifold handling, or seam tagging yet. Added integration coverage for sphere, box, cylinder, tagged provenance, budget capping, and CSG union cases, all validating through `Mesh::validate_deep()`. Validation: `typos crates/exedra_isosurface/src/lib.rs crates/exedra_isosurface/src/dual_contour.rs crates/exedra_isosurface/Cargo.toml .tickets/exe-gosk.md`; `cargo fmt --all`; `cargo test -p exedra_isosurface`; `cargo clippy -p exedra_isosurface --all-targets --all-features -- -D warnings`; `cargo doc -p exedra_isosurface --no-deps`.
