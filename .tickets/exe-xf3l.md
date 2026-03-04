---
id: exe-xf3l
status: closed
deps: []
links: []
created: 2026-03-04T00:40:37Z
type: task
priority: 1
assignee: Bruce Mitchener
tags: [v0.1, correctness]
---
# Preflight boundary-manifold checks before delete_faces mutation

delete_faces can panic during OUTSIDE restitch if deletion would create ambiguous boundary continuation at a vertex (e.g., non-manifold/bowtie boundary topology). Because Txn is eager, panic occurs after partial mutation. Add a preflight check that rejects such cases before mutating arenas.

## Design

Compute boundary-transition implications for the requested face set against surviving topology. Detect vertices where post-delete boundary continuation would not be unique. Return structured precondition error before any remove/insert operations. Keep restitch panic as internal invariant guard, but expected user-input failures should be surfaced as recoverable errors.

## Acceptance Criteria

1) New preflight runs before mutation in delete_faces. 2) Ambiguous boundary continuation returns structured error (no mutation). 3) Add regression test that previously would panic and now errors cleanly. 4) Existing delete tests still pass.


## Notes

**2026-03-04T00:59:28Z**

Implemented preflight boundary-manifold continuation checks in Txn::delete_faces before mutation. Added DeleteFacesError::BoundaryContinuationAmbiguous and regression test on octahedron deletion case to ensure no mutation on failure. Validation: cargo test -p exedra --all-features; cargo clippy -p exedra --all-targets --all-features -- -D warnings.
