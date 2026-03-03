---
id: exe-07nj
status: open
deps: [exe-imti]
links: []
created: 2026-03-03T05:47:29Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.9, boolean]
---
# Patch classification (inside/outside)

Classify face patches as inside or outside relative to the other mesh. This determines which patches to keep for union/intersect/difference.

## Design

After mesh splitting, faces on each mesh are separated into patches by the intersection curves.

For each patch:
- Determine if it is inside or outside the other mesh
- Classification method: ray casting or winding number from a sample point
- CsgOp determines which patches to keep:
  - Union: outside-A + outside-B
  - Intersect: inside-A + inside-B
  - Difference: outside-A + inside-B (with flipped normals on B)

Coplanar patches require special handling and may produce CoplanarAmbiguity errors.
Classification must be deterministic.

## Acceptance Criteria

- Patches correctly classified as inside/outside
- Union, Intersect, Difference produce correct patch selection
- Coplanar cases detected and handled or reported
- Deterministic classification
- Unit tests for each CSG operation on simple shapes


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/exedra/docs/briefs/06_staged_booleans_with_artifacts.md
