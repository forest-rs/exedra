---
id: cam-f0wg
title: Artifacts (bounded, deterministic)
status: open
deps: [exe-dc9l]
links: []
created: 2026-03-03T05:56:28Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, foundation]
---
# Artifacts (bounded, deterministic)

Implement the bounded artifact system for operator debug output. Artifacts are optional payloads (meshes, sets, polylines, fields) that operators emit for debugging and inspection.

## Design

Artifacts { items: Vec<Artifact> }

Artifact enum:
- Mesh { name, mesh }
- FaceSet { name, faces }
- EdgeSet { name, half_edges }
- CornerSet { name, corners }
- Polyline3 { name, points: Vec<[f32; 3]> }
- Polyline2 { name, points: Vec<[f32; 2]> }
- FieldF32 { name, domain, values: Vec<f32> }

Bounded by LimitsPolicy:
- max_artifact_items
- max_artifact_bytes
- Overflow: keep earliest by insertion order (deterministic)

Byte accounting (v0.1):
- Vec<T> artifacts: len * size_of::<T>()
- Mesh artifacts: item-count only (byte estimate = 0) until Exedra has estimate_bytes()

Module layout: artifact.rs

## Acceptance Criteria

- Artifact enum with all documented variants
- Artifacts container enforces item and byte limits
- Overflow is deterministic (earliest kept)
- Unit tests for limit enforcement and byte accounting


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/cambium/docs/briefs/03_reports_and_bounded_artifacts.md
