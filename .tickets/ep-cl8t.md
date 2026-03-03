---
id: ep-cl8t
title: Primitive return type and selection helpers
status: closed
deps: [exe-dc9l]
links: [cam-kiqi]
created: 2026-03-03T06:53:52Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v0.1, foundation, phase2]
---
# Primitive return type and selection helpers

Define the Primitive return type, FaceRegionLayer, Selections, FaceSet, EdgeSet, SelectionName, and canonicalization helpers. This is the public API shape that all primitive generators return.

## Design

Primitive { mesh, face_region, selections }
FaceRegionLayer { default: RegionId, values: Vec<RegionId> }
Selections { face_sets: Vec<(SelectionName, FaceSet)>, edge_sets: Vec<(SelectionName, EdgeSet)> }
FaceSet(Vec<FaceId>) — sorted by stable id, deduplicated
EdgeSet(Vec<HalfEdgeId>) — sorted by stable id, deduplicated
SelectionName(&static str) — stable dot-separated identifiers

Canonical invariants enforced: all sets sorted and deduped.
Provide sort_and_dedup helper functions.

See docs/exedra_primitives_handoff.md for full API shape.

## Acceptance Criteria

- Primitive, FaceRegionLayer, Selections types exist
- FaceSet and EdgeSet enforce canonical invariants
- Sort+dedup helpers exist
- All types are documented
- Unit tests for canonicalization


## Notes

**2026-03-03T17:03:05Z**

Implemented Primitive metadata API in exedra_primitives: Primitive, RegionId, FaceRegionLayer, SelectionName, Selections, canonical FaceSet/EdgeSet wrappers, and sort+dedup helpers. Added rustdoc for all public types and unit tests covering canonicalization and FaceRegionLayer default fallback. Validation: cargo fmt --all, taplo fmt, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace --all-features.
