---
id: cam-kiqi
title: Selection canonicalization
status: closed
deps: [exe-dc9l]
links: [ep-cl8t]
created: 2026-03-03T06:00:09Z
type: feature
priority: P1
assignee: Bruce Mitchener
tags: [v0.1]
---
# Selection canonicalization

Implement canonical selection representation for v0.1. Face selections are Vec<FaceId> sorted in increasing stable-id order and deduplicated.

## Design

Canonical FaceSet: Vec<FaceId> sorted by stable id, deduplicated.

Operators must either:
- Require callers to pass canonical selections, or
- Canonicalize internally (sort + dedup) before use

Provide a canonicalize helper function.
Future: may introduce compressed sets or bitsets, but deterministic ordering rule remains.

SmallCounters includes selections_canonicalized for tracking.

## Acceptance Criteria

- Canonical selection type or helper exists
- Sort + dedup produces deterministic ordering
- Used by uv_planar FaceSet scope
- Unit tests for canonicalization of unsorted/duplicated input


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/cambium/docs/briefs/04_canonical_selection_representation.md

**2026-03-03T06:37:44Z**

Worked example: docs/worked_example_basilica.md — canonical FaceSets flow between operators (e.g. ruin.damage.mask produces a selection consumed by ruin.damage.delete_faces).
