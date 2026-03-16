---
id: ea-m7gy
status: closed
deps: []
links: []
created: 2026-03-16T18:18:26Z
type: task
priority: 2
assignee: Bruce Mitchener
---
# Planar analytic face mutation primitives

Add the first mutable analytic face edits to exedra_analytic so opening-bearing shells can be edited after construction.

## Design

Add narrow analytic mutation methods on AnalyticShell for the current planar MVP: set face region and add a rectangular opening on an XY-aligned planar face. Keep ownership in exedra_analytic; do not route through cambium mesh operators. Document scope and add deterministic tests.

## Acceptance Criteria

1. exedra_analytic can mutate face region on an existing face. 2. exedra_analytic can add one rectangular opening loop to an existing XY planar face with validation. 3. Tests cover success and invalid opening cases. 4. fmt/clippy/tests/doc pass.

## Notes

**2026-03-16T19:01:00Z**

Added the first mutable analytic face operations directly on `AnalyticShell`: `set_face_region`, `face_region`, and `add_rect_opening_xy`. The opening edit validates XY-aligned faces, rejects degenerate/out-of-bounds rectangles, and prevents overlap with existing openings before appending a new opening loop. Updated the owning ADR to capture that the planar MVP now includes narrow face-level mutation, not just static authoring plus tessellation.

Validation:
- `typos crates/exedra_analytic/src/lib.rs crates/exedra_analytic/docs/adr-0001-planar-mvp-scope.md .tickets/ea-m7gy.md`
- `cargo fmt --all`
- `cargo test -p exedra_analytic --all-features`
- `cargo clippy -p exedra_analytic --all-targets --all-features -- -D warnings`
- `cargo doc -p exedra_analytic --no-deps`
