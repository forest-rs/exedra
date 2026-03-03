---
id: ct-geap
status: closed
deps: []
links: []
created: 2026-03-03T17:20:31Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Primitive to UV operator smoke test

Add an end-to-end smoke test that builds an exedra_primitives mesh and runs a Cambium UV operator to validate cross-crate integration.

## Design

Place test in cambium_testkit to avoid adding new production dependencies to core crates. Use box_primitive + uv.box via OperatorRunner::run_commit and assert successful execution, expected counters, and non-trivial UV output after extraction.

## Acceptance Criteria

Workspace test suite includes a passing smoke test that covers primitive generation + operator execution + extraction check.


## Notes

**2026-03-03T17:20:39Z**

Implemented smoke test in cambium_testkit using exedra_primitives::box_primitive through cambium::UvBox via OperatorRunner::run_commit, with validation and extracted UV assertions. Added workspace dev-deps for cambium/exedra/exedra_primitives in cambium_testkit only. Validation: cargo fmt --all, taplo fmt, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace --all-features.
