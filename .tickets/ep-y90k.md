---
id: ep-y90k
title: Box primitive
status: open
deps: [ep-cl8t, exe-jbkx, exe-jctb]
links: []
created: 2026-03-03T06:53:52Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1, phase2]
---
# Box primitive

Implement box_primitive(): a 6-sided box. Exercises sharp edges, region tags, and UV box projection. Basic hard-surface primitive.

## Design

BoxParams { size: [f32; 3], centered: bool, segments: [u32; 3] }
fn box_primitive(params: &BoxParams) -> Primitive

v0.1 may require segments=[1,1,1]. 6 quad faces with per-side regions.
Fixed vertex numbering and face emission order (+X, -X, +Y, -Y, +Z, -Z).

Selections: faces.all, faces.side_x_pos, ..., faces.side_z_neg
Regions: one per side (6 regions)

See docs/exedra_primitives_handoff.md section "box".

## Acceptance Criteria

- box_primitive() returns valid 6-face box
- validate_fast() passes
- Per-side selections and regions correct
- Deterministic vertex/face ordering
- Unit test with fixed params


## Notes

**2026-03-03T06:54:22Z**

Worked example: docs/worked_example_basilica.md — the box primitive is useful for testing UV box projection (step 7).
