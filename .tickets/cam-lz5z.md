---
id: cam-lz5z
status: closed
deps: [ea-5zk2]
links: []
created: 2026-03-17T01:07:48Z
type: feature
priority: 2
assignee: Bruce Mitchener
---
# analytic opening lifecycle helpers

Expose the analytic opening lifecycle surface through Cambium helpers.

## Design

Add a typed helper for opening removal and keep Cambium as a thin workflow layer over exedra_analytic mutation primitives.

## Acceptance Criteria

Cambium exposes a stable helper for removing analytic openings; helper names/tests/docs are updated; end-to-end workflow tests cover add/remove/convert behavior.


## Notes

**2026-03-23T18:49:49Z**

Exposed the analytic opening removal lifecycle through cambium::analytic with a stable helper name, typed params/output, OpError mapping for non-member openings, and end-to-end add/remove/convert workflow coverage. Validation reused the paired analytic lifecycle run: typos crates/exedra_analytic/src/lib.rs crates/cambium/src/analytic.rs crates/exedra_analytic/README.md .tickets/ea-5zk2.md .tickets/cam-lz5z.md; cargo fmt --all; cargo test -p exedra_analytic -p cambium --all-features; cargo clippy -p exedra_analytic -p cambium --all-targets --all-features -- -D warnings; cargo doc -p exedra_analytic -p cambium --no-deps.
