---
id: exe-usoc
status: closed
deps: []
links: []
created: 2026-03-04T00:40:37Z
type: task
priority: 2
assignee: Bruce Mitchener
tags: [v0.1, perf]
---
# Optimize delete_faces vertex-out repair for localized edits

delete_faces currently builds a global outgoing-half-edge index (O(total_half_edges)) even when only a small set of vertices is affected. Introduce a locality-aware strategy: for small affected sets, repair out pointers via scoped scans over affected vertex stars; retain global index path for large batches. This should preserve deterministic behavior while reducing small-edit overhead.

## Design

Add a thresholded strategy in Txn::delete_faces. Inputs: affected vertex set size, total half-edge count. Strategy A (small): for each affected vertex, scan incident candidates only (or bounded scan with deterministic tie-break). Strategy B (large): current global index. Keep identical tie-break semantics for chosen outgoing half-edge. Add micro-benchmark hooks later if needed.

## Acceptance Criteria

1) Out-pointer fixup no longer always builds a global index for small deletions. 2) Deterministic outcomes remain unchanged for existing tests. 3) Add tests covering both strategy paths. 4) cargo clippy/test pass.


## Notes

**2026-03-04T01:00:04Z**

Implemented thresholded out-pointer repair strategy in Txn::delete_faces: localized per-vertex scan for small affected sets, global outgoing index for large affected sets. Added strategy tests for both paths and kept deterministic tie-breaking behavior. Validation: cargo test -p exedra --all-features; cargo clippy -p exedra --all-targets --all-features -- -D warnings.
