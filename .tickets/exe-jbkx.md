---
id: exe-jbkx
title: Mesh construction from indexed triangles
status: open
deps: [exe-cbv1, exe-8hfg, exe-mid7]
links: []
created: 2026-03-03T05:31:46Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Mesh construction from indexed triangles

Implement Mesh::from_indexed_triangles — the primary way to construct a valid half-edge mesh from external data. Takes positions and triangle indices, builds topology with correct twin linkage, boundary half-edges attached to OUTSIDE, and the required position attribute layer.

## Design

Mesh::from_indexed_triangles(positions: &[[f32; 3]], indices: &[[u32; 3]], params: &BuildParams) -> Mesh

BuildParams { weld_tolerance: Option<f32> }

Construction steps:
1. Create vertices with positions
2. Create faces and half-edges for each triangle
3. Link twins: match half-edges by endpoint pairs
4. Unmatched half-edges are boundary: create twin half-edges attached to FaceId::OUTSIDE
5. Link boundary loops (next pointers for OUTSIDE half-edges)
6. Set vertex.out for each vertex
7. Validate the result

This is the first real integration test of the entire topology + attribute stack. Must handle:
- Shared edges between triangles (interior edges with proper twin linkage)
- Boundary edges (open mesh borders)
- Optional vertex welding by tolerance

Determinism: identical inputs must produce identical topology and ID assignment.

## Acceptance Criteria

- Mesh::from_indexed_triangles constructs a valid half-edge mesh
- Shared edges have correct twin linkage
- Boundary edges have twins attached to FaceId::OUTSIDE
- Boundary loops are correctly linked
- Position attribute layer is populated
- validate_fast() passes on the result
- Unit tests: single triangle, two triangles sharing an edge, open quad, closed tetrahedron
- Deterministic: same input produces same mesh

## Notes

**2026-03-03 — ngon builder relationship**

exe-jctb adds MeshBuilder / from_polygons for arbitrary polygon faces (quads, ngon caps). from_indexed_triangles may be reimplemented atop MeshBuilder or coexist as a convenience wrapper. Both paths must produce identical results for triangle-only input.

