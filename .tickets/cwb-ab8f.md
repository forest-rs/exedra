---
id: cwb-ab8f
status: closed
deps: []
links: []
created: 2026-03-08T06:23:23Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Add cylinder normals demo scenario

Add a web demo scenario that shows the difference between flat and smooth authored normals on a cylinder so the new normal-editing operators are visible.

## Design

Use a cylinder primitive and step through default derived normals, baked flat side normals, and smoothed side normals. The scenario should use current Cambium operators and keep the geometry otherwise unchanged so the shading change is legible.

## Acceptance Criteria

- Web demo has a cylinder-focused normals scenario
- Scenario uses the normal editing operators, not viewer-side hacks
- Step labels make the flat vs smooth transition clear
- Bridge tests cover scenario determinism

