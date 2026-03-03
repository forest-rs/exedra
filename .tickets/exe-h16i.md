---
id: exe-h16i
status: open
deps: [exe-qs69]
links: []
created: 2026-03-03T05:43:08Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.9, boolean]
---
# Boolean narrow phase (tri-tri intersection)

Implement triangle-triangle intersection for the boolean pipeline narrow phase. Takes candidate pairs from broad phase and computes actual intersection segments.

## Design

For each candidate triangle pair from broad phase:
- Compute intersection segment (if any)
- Handle edge cases: coplanar triangles, vertex-on-edge, vertex-on-vertex
- Tolerance-aware using NumericPolicy
- Output: intersection segments as (point, point) pairs with source triangle references
- Deterministic: same candidates produce same intersections in same order

## Acceptance Criteria

- Tri-tri intersection computes segments correctly
- Edge cases handled or explicitly rejected with diagnostics
- Tolerance policy respected
- Deterministic output
- Unit tests for: crossing, coplanar, edge-touching, no intersection


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/exedra/docs/briefs/06_staged_booleans_with_artifacts.md
