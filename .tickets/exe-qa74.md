---
id: exe-qa74
status: open
deps: [exe-07nj]
links: []
created: 2026-03-03T05:48:47Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.9, boolean]
---
# Boolean stitch and cleanup

Stitch selected patches from both meshes into a single output mesh. Clean up redundant edges, merge boundary loops along intersection curves, and produce a valid closed mesh.

## Design

Final boolean pipeline stage:
- Take selected patches from classification
- Merge into single mesh with correct topology
- Stitch boundary loops along intersection curves
- Fix orientation (Difference flips B patches)
- Remove degenerate elements if any
- Validate result

Output: Mesh + BooleanArtifacts for diagnostics

## Acceptance Criteria

- Stitched mesh is valid (validate_deep passes)
- Boundary loops correctly merged
- Orientation correct for each CSG op
- No dangling half-edges or orphaned vertices
- End-to-end boolean produces correct results on test corpus


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/exedra/docs/briefs/06_staged_booleans_with_artifacts.md
