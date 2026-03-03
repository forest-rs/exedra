---
id: cam-kiqi
status: open
deps: [exe-dc9l]
links: []
created: 2026-03-03T06:00:09Z
type: feature
priority: 1
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

