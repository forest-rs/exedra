---
id: exe-3otz
title: Exedra documentation pass
status: closed
deps: []
links: []
created: 2026-03-03T05:51:38Z
type: task
priority: P3
assignee: Bruce Mitchener
tags: [v1.0]
---
# Exedra documentation pass

Comprehensive documentation covering: what Exedra guarantees, what it does not attempt, how attributes and seams work, how render extraction splits vertices, determinism contract, boundary model, numeric policy.

## Acceptance Criteria

- Crate-level documentation is comprehensive
- Key concepts documented: half-edge model, boundary, attributes, extraction, determinism
- Examples in documentation
- README updated


## Notes

**2026-05-04T17:08:20Z**

Implementation summary: refreshed the crate-level rustdoc and crate README with guarantees, non-goals, core concepts, a runnable example, and key API map; corrected the render manual to document the current UV+normal render-vertex key. Key decisions/tradeoffs: docs-only pass, so no ADR is needed; crate README/rustdoc/manual are the durable artifact. Validation: cargo fmt --all; cargo test -p exedra --doc --all-features; typos crates/exedra/src/lib.rs crates/exedra/src/manual/render.rs crates/exedra/README.md .tickets/exe-3otz.md; cargo clippy -p exedra --all-targets --all-features -- -D warnings; cargo doc -p exedra --no-deps; cargo fmt --all --check; git diff --check.
