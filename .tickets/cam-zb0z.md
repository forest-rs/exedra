---
id: cam-zb0z
status: open
deps: [cam-ibof]
links: []
created: 2026-03-03T06:00:47Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.1]
---
# UV box projection operator

Implement box projection UV generation — 6 planar projections with deterministic per-face plane selection based on face normal dominant axis.

## Acceptance Criteria

- Box projection selects from 6 planes deterministically
- Correct UV output for all face orientations
- Golden tests for cube and sphere-like meshes


## Notes

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — step 7 uses uv.box for texturing basilica walls/dome.
