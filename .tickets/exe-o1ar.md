---
id: exe-o1ar
status: open
deps: [exe-o4iu, exe-z9pv]
links: []
created: 2026-03-03T05:40:41Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.5]
---
# Extended render extraction (normals + full vertex splitting)

Extend render extraction to include normals in the output and split render vertices on (position, UV, normal) discontinuities. This completes the shading pipeline.

## Design

v0.5 extraction outputs:
- positions, indices, normals, UVs
- Render vertex splits on any discontinuity: UV seam OR normal discontinuity
- NormalsSource controls which normals: derived, custom-or-derived, custom-only
- Extraction stats updated to include normal-related splits

This builds on v0.1 extraction (UV-only splitting) and adds the normal dimension.

## Acceptance Criteria

- Extraction outputs normals
- Vertices split where normals differ (in addition to UV splits)
- NormalsSource policy respected
- Deterministic output
- Golden tests for smooth/flat/mixed shading cases

