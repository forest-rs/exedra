---
id: cam-475p
status: closed
deps: []
links: []
created: 2026-03-08T14:11:47Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Mark semantic feature edges sharp in face-edit operators

Extrude, solidify, and cut-rect currently preserve/clear edge attributes but do not mark obvious feature boundaries sharp by default. This leaves caps, shell rims, and opening perimeters unexpectedly smooth in derived normals.

## Design

Define operator-level defaults for generated topology: extrude/solidify cap-to-wall and shell rim boundaries sharp, wall strips smooth, cut-rect opening perimeter sharp. Add targeted tests that inspect generated edge sharpness and normal behavior.

## Acceptance Criteria

1. Extrude marks cap-to-wall boundaries sharp by default while wall-parallel edges remain smooth. 2. Solidify preserves the same semantic hard boundaries. 3. Cut-rect marks opening perimeter edges sharp by default. 4. Tests cover the contracts. 5. Full workspace quality gates pass.

