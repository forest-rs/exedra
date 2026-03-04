---
id: exe-oinz
status: open
deps: []
links: []
created: 2026-03-04T01:06:25Z
type: chore
priority: 3
assignee: Bruce Mitchener
tags: [v0.1, maintainability]
---
# Introduce sorted-merge helper for deterministic ID joins

Several topology paths use ad-hoc dual-iterator sorted merges/count joins (for example boundary continuation preflight in txn::delete_faces). Add a small internal helper for merge-join/count-join over sorted ID pairs to reduce repetition and prevent subtle divergence in future kernels.

## Design

Create an internal utility module with minimal, no_std-friendly helpers (e.g., merge_count_by_key over sorted slices). Keep API crate-private; avoid generic complexity until a second/third call site exists. Migrate current preflight merge in txn.rs once helper exists, preserving deterministic behavior and existing tests.

## Acceptance Criteria

1) helper exists with unit tests for equal/left-only/right-only paths. 2) txn preflight merge logic is migrated to helper. 3) behavior remains identical (existing tests pass). 4) clippy/test pass.

