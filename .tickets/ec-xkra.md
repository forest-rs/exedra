---
id: ec-xkra
status: closed
deps: [ec-dhp7]
links: [ec-dhp7]
created: 2026-09-05T02:30:32Z
type: feature
priority: 1
assignee: Bruce Mitchener
external-ref: cxc-dmq.4
---
# Make discretization tolerance truthful

Make circular and cubic chord tolerance a truthful contract: satisfy it or return a typed budget/numeric failure. Share a stable circular edge-count API between profile arcs, revolutions, and external cylinder adapters. Coordinate with request 002 inverted-bound validation and preserve provenance, cache identity, and existing failure propagation.

## Design

Fence: exedra_constructive::discretize owns generic curve accuracy and finite per-curve work budgets; it does not own catalog units, catalog cylinder construction, or universal cardinal-axis alignment. Add explicit circular constraints separating minimum topology, maximum work, and optional edge-count multiple. Use cancellation-resistant circular arithmetic; fail typed on budget/count/numeric limits. Arc and cubic discretization never clamp a count that is required for tolerance. Full revolutions explicitly request a multiple of four. Explicit PrimitiveSpec cylinder segment counts remain authored sampling; callers derive tolerance-driven counts through the shared helper. Propagate failure through TessellateError, EvalError, and CompileError. Record the decision in a constructive ADR and bump the stacked evaluation schema to 12.

## Acceptance Criteria

Representative arc, cylinder, cubic, and revolution deviation tests satisfy the requested mathematical chord bound; tightening tolerance does not reduce counts; inadequate budgets, invalid/numeric limits, and count overflow fail typed without panic or false success; endpoints and per-edge provenance survive refinement; cache policy identity and cached report parity remain correct; evaluator and assembly preserve the typed failure payload; request 002 is linked before closure; docs and required repository checks pass.


## Notes

**2026-09-05T02:59:39Z**

Implemented a shared circular_edge_count API with explicit min/max/multiple constraints, stable sagitta arithmetic, checked count/alignment overflow, and typed tolerance-budget or numeric-realization failures. Arc and cubic discretization now reserve f64 coordinate headroom and validate emitted geometry; full revolves opt into multiple-of-four cardinal sampling while explicit primitive cylinder counts remain authored sampling. Failures retain their typed payload through evaluator/cache and assembly compilation. Added ADR 0007, public catalog adapter example, cache/schema 12 updates, and focused geometric/numeric regressions. Tradeoff: when f64 coordinates cannot reliably realize the requested tolerance, callers receive NumericLimit instead of approximate output; f32 mesh narrowing remains a separate representation boundary. Validation passed: cargo fmt --all -- --check; taplo fmt --check; typos; git diff --check; CARGO_TARGET_DIR=/Users/bruce/Development/forest-rs/exedra/target cargo check -p exedra_constructive --no-default-features; CARGO_TARGET_DIR=/Users/bruce/Development/forest-rs/exedra/target cargo clippy --workspace --all-targets --all-features -- -D warnings; CARGO_TARGET_DIR=/Users/bruce/Development/forest-rs/exedra/target cargo test --workspace --all-features; cargo doc --workspace --all-features --no-deps.
