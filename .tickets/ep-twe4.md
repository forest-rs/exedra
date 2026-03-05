---
id: ep-twe4
status: open
deps: [ep-oun3]
links: []
created: 2026-03-05T17:31:16Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [v0.1, primitives]
---
# Torus primitive

## Design

Add torus primitive with major/minor radii and ring/segment counts. Emit deterministic quad mesh with seams in both param directions represented consistently in selections.

## Acceptance Criteria

1) TorusParams + torus() added. 2) Topology validates fast/deep. 3) Selection + region metadata provided. 4) Determinism test included.

