---
id: exe-jctb
status: open
deps: [exe-cbv1, exe-8hfg, exe-mid7]
links: []
created: 2026-03-03T07:06:36Z
type: Ngon/polygon construction API (MeshBuilder + add_face)
priority: P1
assignee: Bruce Mitchener
---
# Untitled

Provide a MeshBuilder (or from_polygons constructor) that accepts arbitrary polygon faces as loops of vertex IDs, not just indexed triangles. Required for real quad faces in exedra_primitives (quad primitive, cylinder caps) and for any operator that creates non-triangular faces. MeshBuilder collects positions + face loops, then builds the half-edge mesh in one shot. add_face takes a slice of VertexIds and wires the half-edge loop. This is the primary construction path for non-triangle geometry.

## Design

MeshBuilder pattern: caller pushes vertices, then pushes faces as &[VertexId] loops. Builder validates manifold constraints and emits Mesh on build(). Alternative: Mesh::from_polygons(positions, face_loops) convenience function that wraps MeshBuilder. Both paths must be deterministic. Winding convention: CCW with outward normals. Non-manifold input is an error, not silently fixed.

## Acceptance Criteria

MeshBuilder can construct a single-quad mesh. MeshBuilder can construct a box (6 quads). Cylinder caps built as ngon fans. All built meshes pass validate_fast(). Deterministic output for identical input. from_indexed_triangles reimplemented atop MeshBuilder internally (or coexists).

