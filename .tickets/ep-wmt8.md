---
id: ep-wmt8
status: closed
deps: []
links: []
created: 2026-03-08T13:06:04Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Fix torus winding and default extracted normals

The torus primitive currently emits inward-facing normals. Default extracted normals should point outward consistently.

## Design

Reverse torus quad winding to match the outward-facing normal convention used by other primitives. Add regression tests for extracted normal direction at representative torus vertices/faces.

## Acceptance Criteria

Torus default extracted normals point outward; torus primitive tests cover the winding contract; workspace render-facing tests remain green.

