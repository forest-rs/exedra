---
id: exe-2g4u
title: Corner UV attribute layer
status: open
deps: [exe-17rj, exe-cbv1]
links: [cam-v5ko]
created: 2026-03-03T05:28:50Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Corner UV attribute layer

Implement the optional corner-domain UV attribute layer. UVs are per-corner (per face-vertex), enabling UV seams without topological splits. This is the key attribute that demonstrates the corner-domain model.

## Design

Corner UVs:
- Domain: HalfEdge (Corner == HalfEdge)
- Type: [f32; 2]
- Storage: optional layer (dense or sparse — decide based on attribute system design)
- Built-in key: exedra::attr::CORNER_UV
- UV seams are represented as discontinuities in corner UV values across twin half-edges, not as topological splits
- Missing UVs on some corners is valid (partial UV coverage)

This layer is consumed by render extraction (to_trimesh) which splits render vertices where UVs differ across corners sharing a vertex.

## Acceptance Criteria

- Corner UV layer exists as an optional corner-domain attribute
- Type is [f32; 2]
- Can be present on some corners and absent on others
- Accessible via built-in key CORNER_UV
- Unit tests for UV get/set, partial coverage


## Notes

**2026-03-03T06:17:41Z**

Design brief: crates/exedra/docs/briefs/01_corner_attributes_and_extraction.md, crates/exedra/docs/briefs/10_attribute_storage_hybrid_dense_sparse.md
