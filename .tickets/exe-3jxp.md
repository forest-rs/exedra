---
id: exe-3jxp
status: open
deps: [exe-cbv1]
links: []
created: 2026-03-03T05:33:35Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Deterministic triangulation

Implement deterministic triangulation of n-gon faces. Triangulation is required for render extraction (to_trimesh). The strategy must be simple, documented, and produce identical output for identical input.

## Design

v0.1 strategy: ear clipping or simple fan triangulation.

Fan triangulation (simplest):
- For face with corners [c0, c1, c2, ..., cn-1]
- Emit triangles: (c0, c1, c2), (c0, c2, c3), ..., (c0, cn-2, cn-1)
- Always starts from Face.edge and follows next
- Deterministic by construction

Limitations of fan: poor quality for non-convex polygons. This is acceptable for v0.1 with documented limitations.

Ear clipping (better quality):
- More complex but handles non-convex polygons
- Must be deterministic: process ears in a stable order

Decision: start with fan triangulation. Document the limitation. Ear clipping can be added later behind a strategy enum.

Output: list of triangle index triples referring to corners/vertices.
Caching: triangulation results should be cacheable per face (invalidated via DirtySet).

## Acceptance Criteria

- Triangulation produces triangles from n-gon faces
- Output is deterministic for identical input
- Handles triangles (passthrough), quads, and n-gons
- Strategy and limitations are documented
- Unit tests for triangle, quad, pentagon, hexagon faces

