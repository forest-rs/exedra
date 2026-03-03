---
id: exe-ikdf
status: open
deps: [exe-h16i]
links: []
created: 2026-03-03T05:46:16Z
type: feature
priority: 2
assignee: Bruce Mitchener
tags: [v0.9, boolean]
---
# Intersection graph construction

Build the intersection graph from narrow-phase intersection segments. Connects segments into polylines that trace the intersection curves on both meshes.

## Design

Input: intersection segments from narrow phase
Output: connected polylines/loops on each mesh surface

Steps:
- Connect segments that share endpoints (within tolerance)
- Trace connected components into polylines or closed loops
- Each polyline vertex knows which face it lies on (both meshes)
- hashbrown may be used internally for connectivity lookup, but output must be deterministically ordered

Artifacts: intersection polylines are a key debug artifact for boolean diagnostics

## Acceptance Criteria

- Segments connected into polylines/loops
- Polylines reference source faces on both meshes
- Output deterministically ordered
- Debug artifact: intersection polylines exportable
- Unit tests for simple intersection curves


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/exedra/docs/briefs/06_staged_booleans_with_artifacts.md
