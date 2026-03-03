---
id: exe-cbv1
title: Mesh struct and boundary model
status: open
deps: [exe-nca7, exe-2752, exe-203r]
links: []
created: 2026-03-03T05:26:03Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# Mesh struct and boundary model

Implement the Mesh struct that owns three arenas (vertices, half-edges, faces) and provides core traversal accessors. Implement the locked boundary model: explicit boundary with Outside Face.

## Design

Mesh owns:
- Arena<Vertex> (vertices)
- Arena<HalfEdge> (half_edges)
- Arena<Face> (faces)
- Attributes (see attribute ticket)

Boundary model (locked architectural decision):
- Every topological edge has exactly two half-edges
- HalfEdge.twin is always valid (no Option)
- A reserved FaceId::OUTSIDE represents the outside/boundary region
- Boundary loops are half-edge cycles attached to OUTSIDE

OUTSIDE representation is an open question (exe-e5kx):
- Option A: OUTSIDE is a real arena entry with a valid Face record
- Option B: OUTSIDE is a sentinel constant treated specially
- Must choose one and document; code and validation must align

Core traversal accessors (at minimum):
- twin(h), next(h), prev(h) (prev may walk the loop)
- face(h), to_vertex(h), from_vertex(h)
- vertex_out(v) — one outgoing half-edge
- face_edge(f) — one half-edge in the face loop
- Vertex star iteration (walk around a vertex, handling boundaries)
- Face loop iteration (walk corners of a face)

Mesh must be Clone (required for Cambium preview path in v0.1).

## Acceptance Criteria

- Mesh struct owns three arenas and provides typed accessors
- Boundary model implemented: every edge has twin, OUTSIDE face exists
- Core traversal works: twin, next, face, to_vertex, from_vertex
- Vertex star walk works (including boundary vertices)
- Face loop walk works
- Mesh: Clone is implemented
- Unit tests for traversal on simple meshes (single triangle, quad, open edge)


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/02_outside_face_boundary_model.md, crates/exedra/docs/briefs/12_half_edge_vs_alternatives.md
