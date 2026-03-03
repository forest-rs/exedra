---
id: exe-qcmn
title: Render extraction (to_trimesh)
status: closed
deps: [exe-3jxp, exe-2g4u, exe-dey4]
links: []
created: 2026-03-03T05:34:01Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Render extraction (to_trimesh)

Implement to_trimesh / extract: convert the polygonal half-edge mesh into a flat triangle mesh suitable for GPU rendering. This is the primary output path from Exedra. Must handle vertex splitting on UV seams and produce deterministic output buffers.

## Design

extract() or to_trimesh() produces a TriMesh:
- indices: Vec<u32>
- positions: Vec<[f32; 3]>
- normals: Vec<[f32; 3]> (v0.5; stub/placeholder in v0.1)
- uvs: Vec<[f32; 2]>

Render-vertex identity:
- A render vertex is uniquely identified by (VertexId, corner attribute values)
- When two corners sharing a vertex have different UVs, the vertex is split into separate render vertices
- v0.1 splits on UV discontinuity; v0.5 adds normal splitting

Output ordering (deterministic, locked):
- Iterate faces in arena order (excluding OUTSIDE)
- For each face, walk corner loop from Face.edge following next
- Triangulate deterministically
- Emit triangles and vertices in traversal order
- Identical inputs + params produce identical buffers

ExtractMode:
- FullRebuild: ignore dirty, rebuild everything
- Incremental: use DirtySet to update only affected regions (may be coarse in v0.1)

Supporting types:
- RenderCache: opaque caches for triangulation, segment maps
- ExtractScratch: reusable staging buffers
- ExtractParams: normals source, normal params, include_uvs
- ExtractStats: triangle count, render vertex count, split count

v0.1 scope: FullRebuild must work. Incremental may be stubbed or coarse.

## Acceptance Criteria

- to_trimesh / extract produces a valid TriMesh
- Vertices split where corner UVs differ
- Output is deterministic (same input + params = same buffers)
- Positions and UVs populated; normals can be placeholder
- ExtractStats reports triangle count, render vertex count, splits
- FullRebuild mode works end-to-end
- Unit tests: mesh with UV seam produces correct splits, ordering is stable across runs


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/01_corner_attributes_and_extraction.md, crates/exedra/docs/briefs/11_deterministic_triangulation_strategy.md

**2026-03-03T06:36:03Z**

Design brief: crates/exedra/docs/briefs/16_scratch_buffer_protocol.md

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — incremental extraction is driven by ChangeSet.dirty after each commit-mode step. The renderer only re-triangulates affected faces.
