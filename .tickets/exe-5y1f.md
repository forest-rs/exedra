---
id: exe-5y1f
title: QEF solver with sharpness classification (exedra_qef)
status: open
deps: []
links: []
created: 2026-03-04T07:03:42Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
tags: [v1.0]
---
# QEF solver with sharpness classification (exedra_qef)

Quadratic Error Function solver with SVD-based rank selection that outputs both optimal vertex placement and a sharpness classification. QEF minimization is the core of dual contouring vertex placement, but the solver is broadly useful for mesh simplification, remeshing, and point cloud fitting.

## Design

Core types:
- QefSolver — accumulates plane constraints (position + normal pairs), solves for the point minimizing squared distance to all planes.
- QefResult — solution point ([f32; 3]), residual error (f32), rank (u8), sharpness_class (Smooth | Edge | Corner).
- SharpnessClass — enum { Smooth, Edge, Corner } derived from SVD rank of the constraint matrix.

Algorithm:
1. Accumulate hermite data: intersection positions + surface normals as plane constraints.
2. Build AᵀA (3×3 symmetric) and Aᵀb from constraints.
3. SVD decomposition of AᵀA → eigenvalues + eigenvectors.
4. Rank selection via relative eigenvalue cutoff (e.g., 1e-3 of largest eigenvalue).
   - rank 1 → Smooth (planar neighborhood, no sharp feature)
   - rank 2 → Edge (two dominant planes meet at an edge)
   - rank 3 → Corner (three or more planes meet at a corner)
5. Solve in the selected rank subspace. For rank < 3, the solution is constrained to the cell centroid in the null-space dimensions (prevents vertex from drifting far from the cell).
6. Clamp solution to cell bounds if it falls outside (boundary safety).

Sharpness output:
- SharpnessClass maps directly to exedra EDGE_SHARPNESS tagging during DC mesh extraction.
- The eigenvalue ratios could inform the f32 sharpness magnitude (sharper features have larger eigenvalue gaps), though the exact mapping is a design decision for the DC mesher, not the solver.

Design constraints:
- no_std compatible (alloc only, no LAPACK dependency)
- Inline SVD for 3×3 symmetric matrices — small enough to do analytically or via Jacobi iteration without pulling in nalgebra.
- No dependency on spatial index types — takes raw positions + normals.

Reuse cases beyond DC:
- Garland-Heckbert mesh simplification: QEF per vertex/edge for optimal placement during edge collapse. SharpnessClass can drive collapse priority (don't collapse corners before edges before smooth regions).
- Remeshing: feature-preserving vertex relocation during isotropic remeshing.
- Point cloud fitting: fit planes to local point neighborhoods, classify sharp features in scan data.
- Surface-surface intersection: locate intersection curves via constraint minimization.

## Acceptance Criteria

- QefSolver accumulates position+normal constraints
- SVD-based rank selection with configurable eigenvalue cutoff
- SharpnessClass output (Smooth, Edge, Corner)
- Solution clamped to provided cell bounds
- Residual error reported for quality assessment
- no_std compatible, no external linear algebra dependency
- Unit tests: planar surface → Smooth, two planes meeting → Edge, three planes → Corner
- Tests for degenerate inputs (collinear normals, single constraint)
- Benchmarks for typical DC cell sizes (4-12 constraints)

