---
id: exe-gujq
title: Mesh compaction (compact + Remap)
status: closed
deps: []
links: []
created: 2026-03-03T05:51:38Z
type: feature
priority: P2
assignee: Bruce Mitchener
tags: [v1.0]
---
# Mesh compaction (compact + Remap)

Implement explicit mesh compaction. Removes tombstoned slots from arenas and produces a Remap for translating old IDs to new. Never implicit — caller-controlled.

## Design

Mesh::compact(&self) -> (Mesh, Remap)

Remap provides old->new mappings per domain. Deleted elements have no mapping.
New arenas laid out deterministically (stable traversal order of old mesh, excluding tombstones).
Compaction produces reproducible results (determinism contract).

## Acceptance Criteria

- compact() produces a new mesh with no tombstones
- Remap correctly translates all live IDs
- Deterministic: same input produces same compacted mesh
- Attributes preserved correctly
- validate_deep() passes on compacted mesh


## Notes

**2026-03-03T06:21:10Z**

Design brief: crates/exedra/docs/briefs/07_stable_ids_and_compaction.md

**2026-05-04T16:44:36Z**

Implemented explicit Mesh::compact and Remap for source-to-compacted ID translation. Compaction builds fresh tombstone-free arenas, rewrites topology through generation-checked remap entries, preserves MeshRevision, and rebuilds dense/sparse attributes against compacted domain capacities. Updated crate root/docs to expose Remap and document OUTSIDE/deleted/stale mapping behavior. No new ADR: this implements the existing stable IDs and explicit compaction brief without changing the underlying architecture decision. Validation: cargo fmt --all; cargo test -p exedra --all-features; cargo clippy -p exedra --all-targets --all-features -- -D warnings; cargo doc -p exedra --no-deps; typos crates/exedra/src crates/exedra/docs; cargo fmt --all --check.
