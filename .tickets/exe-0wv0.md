---
id: exe-0wv0
title: Migrate EDGE_SHARPNESS from bool to f32 for semi-sharp creasing
status: open
deps: []
links: []
created: 2026-03-04T05:53:27Z
type: feature
priority: 1
assignee: Bruce Mitchener
tags: [v0.5]
---
# Migrate EDGE_SHARPNESS from bool to f32 for semi-sharp creasing

Migrate EDGE_SHARPNESS from bool to f32. Semi-sharp Catmull-Clark creasing requires continuous sharpness values that decay by 1.0 per subdivision level. Current bool gives only smooth (0) or infinitely sharp — no semi-sharp creases. Affects: attr key type, Mesh accessors (edge_sharpness/set_edge_sharpness), Txn wrappers, split_edge propagation (decrement instead of copy), MarkEdgeSharp operator (accept f32), render extraction seam detection (threshold instead of equality).

## Design

Storage: sparse f32 on canonical edge (same as current bool). Default: 0.0 (smooth). Infinity or a large sentinel (e.g. f32::INFINITY) for infinitely sharp. Semi-sharp range: (0.0, inf). split_edge propagation: child sharpness = max(parent - 1.0, 0.0). MarkEdgeSharp operator: accept f32 value instead of bool. Backward compat: edge_sharpness() returns Option<f32>; callers checking bool can compare > 0.0. Seam detection in render extraction: edge is a seam if sharpness > 0.0 (or use a configurable threshold).

## Acceptance Criteria

1) EDGE_SHARPNESS key type is f32. 2) Mesh::edge_sharpness returns Option<f32>. 3) set_edge_sharpness accepts f32. 4) Default is 0.0 (smooth). 5) split_edge decrements sharpness by 1.0 (clamped to 0.0). 6) MarkEdgeSharp operator accepts f32. 7) Existing bool-based tests migrated. 8) cargo clippy/test pass.
