---
id: ep-06tl
status: open
deps: [ep-oun3]
links: []
created: 2026-03-05T17:31:16Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [v0.1, primitives]
---
# Segmented grid primitive

## Design

Add a planar grid primitive with configurable size, centered flag, and segment counts in X/Y. Emit deterministic quad topology with region + selections (faces.all, edges.boundary).

## Acceptance Criteria

1) GridParams + grid() added. 2) Topology validates fast/deep. 3) Determinism test included. 4) Selection/region docs present.

