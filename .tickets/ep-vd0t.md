---
id: ep-vd0t
status: open
deps: []
links: [ep-y90k]
created: 2026-03-03T17:19:45Z
type: feature
priority: 2
assignee: Bruce Mitchener
---
# Box primitive segmented topology support

Extend box_primitive beyond [1,1,1] segments to support deterministic subdivisions along X/Y/Z while preserving stable face ordering, side-region tags, and canonical selections.

## Design

Current implementation intentionally panics unless segments=[1,1,1]. Add deterministic grid subdivision per side with explicit vertex/face emission order. Preserve existing side selection names and region IDs, and define whether side selections include all subdivided faces per side. Keep no_std and avoid new dependencies.

## Acceptance Criteria

box_primitive accepts segments >= 1 for each axis; validate_fast passes; side region mapping and canonical selections are correct for subdivided boxes; deterministic output holds across runs; tests cover [1,1,1], asymmetric segmentation (e.g., [2,1,3]), and invalid zero segments.

