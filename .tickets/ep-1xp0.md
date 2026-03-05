---
id: ep-1xp0
status: open
deps: [ep-oun3, ep-we4l]
links: []
created: 2026-03-05T17:31:16Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [v0.1, primitives]
---
# Cone primitive with cap fill modes

## Design

Add cone primitive with radial segments, height/radius, centered option, and CapFill control for base. Emit side faces plus optional base cap according to fill mode. Provide seam/rim selections mirroring cylinder semantics where applicable.

## Acceptance Criteria

1) ConeParams + cone() added. 2) Base cap honors None/Ngon/TriangleFan. 3) Selection + region sets documented and tested. 4) Determinism test included.

