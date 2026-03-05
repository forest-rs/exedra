---
id: cam-y041
status: closed
deps: []
links: []
created: 2026-03-05T10:08:32Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, operator, edit, modeling]
---
# Rectangular face cut operator for openings

Add an operator to cut a rectangular opening on a selected face so users can place windows/doors at explicit positions instead of relying on centroid inset behavior.

## Design

Introduce edit.face.cut.rect with params describing target face, local frame (origin/u_axis/v_axis), and rectangle extents. v0.1 scope: planar quad support, deterministic topology split, and opening face IDs output for downstream delete/extrude flows. Reject unsupported/non-planar/degenerate inputs with structured diagnostics.

## Acceptance Criteria

- edit.face.cut.rect operator exists with stable name()
- deterministic cut topology for supported faces
- output includes created inner face(s)/boundary edges for follow-up operations
- diagnostics for unsupported geometry and invalid rectangle params
- tests cover door/window-style cuts on a wall face
