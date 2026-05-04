---
id: ep-jql5
status: closed
deps: []
links: [ep-od6p, ep-wbxp]
created: 2026-03-03T17:19:52Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# no_std trig policy for primitive generation

Define and implement a clear numeric policy for trigonometric evaluation in no_std primitive generation (currently custom polynomial approximation).

## Design

Decide whether to keep the current approximation, switch to a feature-gated backend (e.g., libm under optional feature), or use a hybrid strategy. Document error tolerance expectations and determinism requirements. Add tests that quantify approximation error bounds relevant to cylinder/uv_sphere generation and verify that topology/selection determinism remains unchanged.

## Acceptance Criteria

A documented trig policy exists in crate docs; implementation matches policy; tests enforce chosen error bounds (or conformance assertions); no_std default remains dependency-light; deterministic outputs remain stable for fixed params.


## Notes

**2026-05-04T17:12:33Z**

Implementation summary: documented the exedra_primitives trig policy in crate rustdoc, README, and ADR 0002; clarified that std/libm backends own coordinate math while integer segment/ring indices own topology, regions, and selections; added sampled-angle error and unit-circle tests around common::sin_cos. Key decisions/tradeoffs: keep the existing dependency-light default std backend and optional libm no_std backend; remove ambiguity about custom polynomial approximation; do not promise bit-identical coordinates across backends. Validation: cargo fmt --all; cargo test -p exedra_primitives --all-features; cargo test -p exedra_primitives --no-default-features --features libm; cargo clippy -p exedra_primitives --all-targets --all-features -- -D warnings; cargo doc -p exedra_primitives --no-deps; typos crates/exedra_primitives/src crates/exedra_primitives/README.md crates/exedra_primitives/docs .tickets/ep-jql5.md; taplo fmt --check crates/exedra_primitives/Cargo.toml; cargo fmt --all --check; git diff --check.
