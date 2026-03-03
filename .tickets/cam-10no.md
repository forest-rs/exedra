---
id: cam-10no
title: Mark seam edges operator
status: open
deps: [cam-ibof, exe-5nj1]
links: []
created: 2026-03-03T06:00:47Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1]
---
# Mark seam edges operator

Implement seam edge marking operator. Tags edges as UV seams. Related to but distinct from sharpness — seams affect UV continuity, sharpness affects normal computation.

## Design

Uses the explicit EDGE_SEAM edge-domain bool attribute (defined in exe-5nj1). The operator sets EDGE_SEAM = true on selected edges. This is the authoritative seam marker — distinct from implicit UV discontinuity detection (which checks whether corner UVs differ across an edge). UV projection operators read EDGE_SEAM to know where to cut.

## Acceptance Criteria

- Sets EDGE_SEAM attribute on selected edges
- Distinct from sharpness (EDGE_SHARPNESS)
- Can mark and unmark edges
- Unit tests verify attribute is set correctly
- Works through EditOperator / OperatorRunner

