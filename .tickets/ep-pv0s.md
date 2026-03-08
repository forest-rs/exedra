---
id: ep-pv0s
status: closed
deps: []
links: []
created: 2026-03-08T13:30:27Z
type: task
priority: P1
assignee: Bruce Mitchener
---
# Fix box primitive face winding

Default box primitive faces are wound inward, which inverts default extracted normals and hides normal-line debug output.

## Design

Reverse each emitted box face loop so all six region faces point outward while preserving deterministic face ordering and sharp-edge assignment.

## Acceptance Criteria

1. Box face normals point outward for all six region faces. 2. Default box extracted normals behave correctly in the viewer/debug path. 3. Tests cover region-normal direction. 4. Full workspace quality gates pass.

