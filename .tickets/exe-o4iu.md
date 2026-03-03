---
id: exe-o4iu
status: open
deps: [exe-k3nb, exe-qcmn]
links: []
created: 2026-03-03T05:37:58Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.5]
---
# Derived corner normals

Implement angle/area-weighted derived corner normals that respect sharp edges. Normals are computed from geometry and sharpness data, stored as corner-domain derived data. This makes Exedra produce real shading.

## Design

NormalParams { auto_sharp_angle_degrees: Option<f32>, weight_mode: NormalWeightMode }
NormalWeightMode: Angle, Area, AngleArea

Computation:
- For each corner, accumulate weighted face normals from the vertex one-ring
- Sharp edges act as boundaries for normal accumulation (create hard shading breaks)
- auto_sharp_angle_degrees: edges where dihedral angle exceeds threshold are treated as sharp
- Weight mode controls how face normals are weighted in the accumulation

This is compute-heavy for large meshes — design for future parallelization.
Determinism: identical mesh + params = identical normals.

## Acceptance Criteria

- Corner normals computed from geometry respecting sharp edges
- Angle, Area, and AngleArea weight modes work
- Auto-sharp angle threshold works
- Deterministic output
- Golden tests for smooth sphere, sharp cube, mixed sharpness cases

