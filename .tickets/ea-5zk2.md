---
id: ea-5zk2
status: closed
deps: []
links: []
created: 2026-03-17T01:07:48Z
type: feature
priority: 2
assignee: Bruce Mitchener
---
# analytic opening lifecycle primitives

Add opening removal and stronger opening lifecycle validation for planar analytic faces.

## Design

Own opening lifecycle in exedra_analytic. Add a removal primitive keyed by face + opening loop, keep validation centralized in the analytic shell, and extend tests/docs for multi-opening behavior.

## Acceptance Criteria

AnalyticShell can remove an existing opening from a face; removing a non-member opening fails deterministically; multi-opening add/remove behavior is covered by tests; public API is documented.


## Notes

**2026-03-23T18:49:49Z**

Added AnalyticShell::remove_opening keyed by face + opening loop, introduced deterministic OpeningNotOnFace errors, and covered single-opening removal plus multi-opening add/remove behavior in exedra_analytic tests. Updated the crate README so the documented planar MVP now matches explicit opening loops and face-level opening lifecycle edits. No ADR update was needed because this extends the accepted planar-opening lifecycle within the existing exedra_analytic boundary rather than changing crate ownership or conversion policy. Validation: typos crates/exedra_analytic/src/lib.rs crates/cambium/src/analytic.rs crates/exedra_analytic/README.md .tickets/ea-5zk2.md .tickets/cam-lz5z.md; cargo fmt --all; cargo test -p exedra_analytic -p cambium --all-features; cargo clippy -p exedra_analytic -p cambium --all-targets --all-features -- -D warnings; cargo doc -p exedra_analytic -p cambium --no-deps.
