---
id: exe-0sqg
status: closed
deps: []
links: []
created: 2026-03-05T02:27:26Z
type: task
priority: P1
assignee: Bruce Mitchener
tags: [v0.1, test, architecture]
---
# Adjacency index consistency cross-check mode

Add a debug/test consistency mode that rebuilds adjacency from topology and asserts equality with maintained index state after representative kernel edits.

## Design

Implement internal checker utilities to derive adjacency from authoritative topology and compare against maintained index structures. Expose checker for tests and optional debug assertions in edit-heavy paths. Keep deterministic comparison ordering and diagnostics for first mismatch.

## Acceptance Criteria

- Adjacency checker utility exists and compares maintained index vs rebuilt view; - representative edit-kernel tests invoke checker after mutations; - mismatch diagnostics include vertex/edge context; - no impact on release-mode behavior

## Notes

**2026-03-05T04:11:36Z**

Implemented test/debug adjacency consistency check in EditSession (assert_outgoing_index_consistent) that compares maintained outgoing index vs rebuilt topology view with first-mismatch diagnostics (from/to/half-edge/face). Added representative kernel tests invoking checker after add_face, split_edge, delete_faces, and delete_vertices mutations. Validation: cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo test --workspace --all-features.
