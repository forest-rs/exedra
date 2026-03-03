---
id: cam-v5ko
status: open
deps: [cam-ibof, cam-kiqi, cam-l8n1, exe-2g4u, exe-qcmn]
links: [exe-2g4u]
created: 2026-03-03T05:59:00Z
type: feature
priority: 0
assignee: Bruce Mitchener
tags: [v0.1]
---
# uv_planar operator (v0.1 vertical slice)

Implement the uv_planar operator — the v0.1 vertical slice that exercises the entire Cambium stack end-to-end: EditOperator, OperatorRunner, ChangeSet, deterministic extraction, golden tests.

## Design

UvPlanarParams { scope: UvScope, plane: UvPlane, scale: f32, offset: [f32; 2], write_missing_only: bool }
UvScope: WholeMesh, FaceSet (deterministic FaceId list)
UvPlane: WorldXY, WorldXZ, WorldYZ, PerFaceFromGeometry

Determinism rules:
- WholeMesh: iterate faces in arena order
- FaceSet: sorted stable id order, deduplicated (canonical representation)
- Corner walk: Face.edge -> next
- Projection math uses stable operations

PerFaceFromGeometry tie-break:
- Compute geometric normal via fan around first vertex
- Dominant axis by max(|nx|,|ny|,|nz|)
- Tie-break within normal_epsilon: prefer X > Y > Z
- Degenerate faces (max < normal_epsilon): fall back to WorldXY

UvPolicy controls: default scale/offset, allow_overwrite_existing

Stats: faces_processed, corners_written, corners_skipped_existing
Timing: select, compute, attrs, validate
Artifacts: FaceSet of affected faces (bounded, optional)

Golden tests required:
- Small corpus: triangle, quad, ngon, UV seam cases, non-planar face
- For each: run uv_planar with fixed params, verify corner UV layer matches golden
- Run Exedra extraction with UVs, verify TriMesh buffers match golden

This is the proof that the whole stack works.

Module layout: ops/uv_planar.rs

## Acceptance Criteria

- uv_planar implements EditOperator
- All UvPlane variants work correctly
- WholeMesh and FaceSet scopes work
- write_missing_only respected
- PerFaceFromGeometry tie-break is deterministic
- Stats and timing buckets populated correctly
- Golden determinism tests pass
- End-to-end: mesh -> uv_planar -> extraction -> TriMesh matches golden

