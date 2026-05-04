---
id: exe-1wdr
title: Semver-stable API audit
status: closed
deps: []
links: []
created: 2026-03-03T05:51:38Z
type: task
priority: P2
assignee: Bruce Mitchener
tags: [v1.0]
---
# Semver-stable API audit

Audit the entire Exedra public API surface for semver stability. Ensure types, traits, and methods are intentionally public and documented. Lock the API shape for 1.0.

## Acceptance Criteria

- All public types and methods are intentionally public
- All public items are documented
- No accidental pub(crate) leakage
- API surface documented in a summary


## Notes

**2026-05-04T16:35:38Z**

Summary: added crates/exedra/docs/api-surface.md as the Exedra public API audit summary. The audit records the intended root entry points, low-level public diagnostic/advanced surfaces, and stability notes for IDs, OUTSIDE, corner IDs, attributes, mutation, and extraction. No public API was removed; no ADR update was needed because this documents existing API intent rather than changing ownership or semantics. Validation: typos crates/exedra/src/lib.rs crates/exedra/docs/api-surface.md; cargo fmt --all --check; cargo doc -p exedra --no-deps; cargo test -p exedra --all-features; cargo clippy -p exedra --all-targets --all-features -- -D warnings.
