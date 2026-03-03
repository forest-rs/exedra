---
id: exe-64i3
title: Minimal traversal iterators (faces, vertices, face_loop, vertex_star)
status: closed
deps: [exe-cbv1]
links: []
created: 2026-03-03T07:06:59Z
type: Minimal traversal iterators (faces, vertices, face_loop, vertex_star)
priority: P1
assignee: Bruce Mitchener
---
# Minimal traversal iterators (faces, vertices, face_loop, vertex_star)

Add boring, minimal iterator/traversal API to Mesh. Four core accessors: mesh.faces() iterates live faces, mesh.vertices() iterates live vertices, mesh.face_loop(face) walks the half-edge loop of a face yielding HalfEdgeIds, mesh.vertex_star(v) walks the one-ring of a vertex yielding neighboring faces/edges. These are the foundation for all higher-level queries and operator logic.

## Design

faces() and vertices() iterate arena slots, skipping dead entries. Return type: impl Iterator<Item = FaceId> / VertexId. face_loop(face) starts at face.half_edge, follows next until wrapping. Returns impl Iterator<Item = HalfEdgeId>. vertex_star(v) walks outgoing half-edges around vertex using twin/next. Returns impl Iterator<Item = HalfEdgeId> (caller can get face from each). All iterators are deterministic (arena-order for faces/vertices, topology-order for loops/stars). No allocation required for iteration.

## Acceptance Criteria

mesh.faces() yields all live face IDs in arena order. mesh.vertices() yields all live vertex IDs in arena order. mesh.face_loop(f) yields half-edges of the face loop. mesh.vertex_star(v) yields one-ring half-edges. All work on quad, box, cylinder, sphere meshes. Deterministic ordering.


## Notes

**2026-03-03T10:33:26Z**

Known limitation (v0.1): current Mesh::vertex_star implementation scans all half-edges and filters via from_vertex(), where from_vertex() derives origin through prev() face-loop walking. Complexity is O(total_half_edges * face_degree) worst-case. Keep for now for simplicity/determinism; revisit with adjacency-driven iteration when traversal performance becomes a bottleneck.
