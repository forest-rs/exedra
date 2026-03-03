---
id: ep-wbxp
title: UV sphere primitive
status: closed
deps: [ep-cl8t, exe-jbkx, exe-jctb]
links: [ep-od6p, ep-jql5, ep-ahz4]
created: 2026-03-03T06:53:52Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1, phase2]
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


## Notes

**2026-03-03T17:13:54Z**

Implemented deterministic uv_sphere primitive with pole triangle fans, quad mid-bands, deterministic seam selection, and canonical pole/all face selections. Added region tagging for body/top-pole/bottom-pole and tests for topology counts + determinism. Reused shared no-std trig helper in common module. Validation: cargo fmt --all, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace --all-features.
