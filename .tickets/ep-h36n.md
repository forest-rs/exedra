---
id: ep-h36n
title: Quad / plane primitive
status: closed
deps: [ep-cl8t, exe-jbkx, exe-jctb]
links: []
created: 2026-03-03T06:53:52Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1, phase2]
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


## Notes

**2026-03-03T17:08:56Z**

Implemented deterministic quad primitive as a single ngon face with canonical selections and region tagging. Added shared exedra_primitives common helpers for primitive assembly/face-region layers. Added unit tests for validity, canonical selection contents, and deterministic output. Validation: cargo fmt --all, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace --all-features.
