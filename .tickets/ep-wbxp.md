---
id: ep-wbxp
status: open
deps: [ep-cl8t, exe-jbkx, exe-jctb]
links: []
created: 2026-03-03T06:53:52Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.1]
---
# UV sphere primitive

Implement uv_sphere(): latitude-longitude sphere with poles and a seam. Great for UV and normal edge cases. Surfaces triangulation and derived normal corner cases early.

## Design

UvSphereParams { radius: f32, lat_segments: u32, lon_segments: u32, centered: bool }
fn uv_sphere(params: &UvSphereParams) -> Primitive

Pole caps are triangle fans. Mid-band faces are quads. Seam at longitude 0.

Selections: faces.all, edges.seam, faces.pole_top, faces.pole_bottom
Regions: all faces; optionally separate pole cap regions

See docs/exedra_primitives_handoff.md section "uv_sphere".

## Acceptance Criteria

- uv_sphere() returns valid mesh with correct pole/band topology
- validate_fast() passes
- Seam and pole selections correct
- Deterministic vertex/face ordering
- Unit test with fixed params

