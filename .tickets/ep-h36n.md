---
id: ep-h36n
status: open
deps: [ep-cl8t, exe-jbkx]
links: []
created: 2026-03-03T06:53:52Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Quad / plane primitive

Implement quad(): a single quad face primitive. The smallest mesh for validation, extraction, and UV testing. Exercises ngon triangulation.

## Design

QuadParams { size: [f32; 2], centered: bool }
fn quad(params: &QuadParams) -> Primitive

Single ngon face (not two triangles). Deterministic vertex ordering.
Selections: faces.all, edges.boundary
Region: REGION_FACE = 1

See docs/exedra_primitives_handoff.md section "quad / plane".

## Acceptance Criteria

- quad() returns a valid Primitive with single quad face
- validate_fast() passes
- Selections are canonical
- Deterministic output across runs
- Unit test with fixed params

