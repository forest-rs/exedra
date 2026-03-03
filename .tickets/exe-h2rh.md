---
id: exe-h2rh
title: collapse_edge and flip_edge
status: open
deps: [exe-tezb, exe-0a9w]
links: []
created: 2026-03-03T05:39:55Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.5]
---
# collapse_edge and flip_edge

Implement collapse_edge and flip_edge edit primitives. These are higher-risk operations — correctness over speed. Only implement if correctness can be proven.

## Design

collapse_edge: remove edge, merge endpoints.
- Deterministic: smaller id wins (v_keep)
- Degenerate faces removed
- UV conflict: prefer surviving corner UV or clear on conflict
- Custom normals: clear on affected corners
- Edge attributes: merge (sharp if either sharp, crease = max)

flip_edge: change diagonal in a quad region (two triangles).
- Corner UVs: derive by interpolation or clear
- Custom normals: clear on affected corners
- Edge sharpness: new diagonal defaults smooth

Both are v0.5+ and only if correctness is proven. May be deferred.

## Acceptance Criteria

- collapse_edge removes edge and merges vertices correctly
- flip_edge rotates diagonal in triangle pair
- All attribute domains handled
- No invariant violations after operation
- validate_deep() passes after each operation
- Unit tests with complex attribute configurations


## Notes

**2026-03-03T06:27:28Z**

Design brief: crates/exedra/docs/briefs/13_edit_propagation_model.md
