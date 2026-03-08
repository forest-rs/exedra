---
id: cam-ku22
status: closed
deps: []
links: []
created: 2026-03-08T17:38:05Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Complete semantic sharp-edge defaults for composed face edits

Inset-driven step geometry and opening workflows still leave some intended hard feature boundaries smooth. Complete the default sharpness policy across inset, cut/opening, and solidify/extrude compositions with regressions taken from pedestal and wall-opening style shapes.

## Design

Add composed-workflow regressions for stepped inset/extrude and wall opening meshes, inspect generated edge sharpness around the remaining soft boundaries, then adjust face-edit defaults so the semantic feature edges are marked sharp consistently. Keep smooth-only behavior only where the geometry should read as one continuous surface.

## Acceptance Criteria

1. Stepped inset/extrude geometry no longer smooths across intended architectural step boundaries. 2. Wall opening/frame workflows no longer leave intended hard feature edges smooth. 3. Solidify/extrude defaults remain consistent with the same semantic model. 4. Tests cover the composed workflows. 5. Full workspace quality gates pass.

