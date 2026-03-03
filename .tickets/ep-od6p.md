---
id: ep-od6p
title: Cylinder primitive
status: closed
deps: [ep-cl8t, exe-jbkx, exe-jctb]
links: [cam-tecx]
created: 2026-03-03T06:53:52Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1, phase2]
---
# Cylinder primitive

Implement cylinder(): optionally capped cylinder. Exercises seam behavior and later cylinder UV projection. Good for columns/drums in the basilica demo.

## Design

CylinderParams { radius: f32, height: f32, segments: u32, capped: bool, centered: bool }
fn cylinder(params: &CylinderParams) -> Primitive

Ring vertices at increasing angles. Seam always at angle 0.
Cap faces are ngons (single polygon per cap).

Selections: faces.side, faces.cap_top, faces.cap_bottom, edges.seam, edges.rim_top, edges.rim_bottom
Regions: side, cap_top, cap_bottom

Worked example: docs/worked_example_basilica.md — drums and columns use cylinders.
See docs/exedra_primitives_handoff.md section "cylinder".

## Acceptance Criteria

- cylinder() returns valid mesh with side quads and optional cap ngons
- validate_fast() passes
- Seam edge set is deterministic (angle 0)
- Capped and uncapped modes work
- Unit tests for both modes


## Notes

**2026-03-03T06:54:22Z**

Worked example: docs/worked_example_basilica.md — drums and columns are cylinders. The drum primitive feeds into shape.add.dome.

**2026-03-03T17:12:35Z**

Implemented deterministic cylinder primitive with capped/uncapped modes, deterministic seam/rim selections from builder provenance, and side/top/bottom region tagging. Added no-std sin/cos helper in shared module for angular sampling without extra dependencies. Added tests for capped and uncapped validity and determinism. Validation: cargo fmt --all, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace --all-features.
