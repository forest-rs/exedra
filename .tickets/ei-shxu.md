---
id: ei-shxu
status: closed
deps: []
links: []
created: 2026-03-24T03:42:47Z
type: feature
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
---
# Use Hermite mass-point anchoring for dual contour QEF

Reduce cell-center quantization in dual contouring by biasing low-rank QEF solves toward the cell's Hermite mass point instead of the geometric cell center.

## Design

Own the smallest cross-crate fix that can materially improve smooth-surface output. exedra_qef should gain an explicit anchor-aware solve path while preserving the current solve API, and exedra_isosurface should pass the average Hermite intersection position for each active cell. Add regression coverage showing that sphere-cell solutions move off the cell-center lattice when Hermite data supports it.

## Acceptance Criteria

- exedra_qef exposes an anchor-aware bounded solve without breaking existing callers
- exedra_isosurface uses the Hermite mass point as the QEF anchor for active cells
- regression tests cover both QEF anchoring semantics and non-center sphere-cell placement
- docs/ticket notes explain the new anchoring behavior and remaining mesher limits

## Notes

**2026-03-24T03:49:31Z**

Implemented explicit anchor-aware QEF solving in exedra_qef and switched exedra_isosurface active-cell placement to use the Hermite mass point instead of always anchoring low-rank null-space dimensions to the geometric cell center. exedra_qef now exposes solve_with_anchor while preserving the existing solve API, with new unit coverage proving single-plane solves respect a custom anchor in null-space directions. exedra_isosurface now computes the average Hermite hit position per active cell and passes it into the QEF solve, plus a regression test verifies that analytic sphere cells no longer all solve exactly to their cell centers. Updated exedra_qef and exedra_isosurface README/ADR docs and captured the execution plan in crates/exedra_isosurface/docs/plans/ei-shxu-mass-point-qef-anchoring.md. Validation: cargo fmt --all; typos crates/exedra_qef/src/lib.rs crates/exedra_qef/README.md crates/exedra_qef/docs/adr-0001-qef-solver-scope.md crates/exedra_isosurface/src/dual_contour.rs crates/exedra_isosurface/README.md crates/exedra_isosurface/docs/adr-0002-phase-1-dual-contouring.md crates/exedra_isosurface/docs/plans/ei-shxu-mass-point-qef-anchoring.md .tickets/ei-shxu.md; cargo test -p exedra_qef -p exedra_isosurface; cargo clippy -p exedra_qef -p exedra_isosurface --all-targets --all-features -- -D warnings; cargo doc -p exedra_qef -p exedra_isosurface --no-deps. One important follow-on surfaced during verification: Fidget-backed spheres still quantize to cell centers because exedra_fidget is currently seeding gradient-evaluator inputs incorrectly; that bug is tracked separately in ef-1dz9.
