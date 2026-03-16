---
id: cam-lggo
status: closed
deps: [ea-m7gy]
links: []
created: 2026-03-16T18:18:26Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Cambium analytic workflow helpers

Expose the first analytic-facing operation family in Cambium on top of exedra_analytic mutation primitives and explicit conversion.

## Design

Add a Cambium analytic module with typed helper params/output for the first analytic edits, using OpError-compatible diagnostics but not the mesh-only EditOperator runtime. Re-export the surface from crate root and prove one analytic edit -> convert to mesh flow in tests/docs.

## Acceptance Criteria

1. Cambium exposes typed analytic helper APIs for setting face region and adding a rectangular opening. 2. Helpers delegate to exedra_analytic instead of mutating mesh state. 3. Tests cover an edit-then-convert workflow. 4. fmt/clippy/tests/doc pass.

## Notes

**2026-03-16T19:08:00Z**

Added a new `cambium::analytic` module for the first analytic-facing helper family. The module exposes typed params/output for `set_face_region` and `add_rect_opening_xy`, maps analytic edit failures into `OpError`, and keeps the ownership boundary honest by mutating `AnalyticShell` directly rather than trying to force analytic state through the mesh-only `EditOperator` runtime. The crate root now re-exports this module and its helper surface.

Validation:
- `typos crates/cambium/src/analytic.rs crates/cambium/src/lib.rs .tickets/cam-lggo.md`
- `cargo fmt --all`
- `cargo test -p exedra_analytic -p cambium --all-features`
- `cargo clippy -p cambium --all-targets --all-features -- -D warnings`
- `cargo doc -p cambium --no-deps`
