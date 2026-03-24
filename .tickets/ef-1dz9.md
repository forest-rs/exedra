---
id: ef-1dz9
status: closed
deps: []
links: []
created: 2026-03-24T03:49:02Z
type: bug
priority: 2
assignee: Bruce Mitchener
parent: exe-xgtv
---
# Seed Fidget gradient inputs with basis derivatives

Fix exedra_fidget gradient evaluation to seed x/y/z inputs with basis derivatives instead of zero-derivative Grad values.

## Design

The adapter should follow Fidget's own grad-slice calling convention: x inputs carry dx=1, y inputs carry dy=1, z inputs carry dz=1. This is required for meaningful Hermite normals and QEF placement. Add focused regression coverage on gradient values and on a Fidget-backed sphere that no longer collapses to cell centers.

## Acceptance Criteria

- `exedra_fidget` `eval_gradients` seeds basis derivatives correctly
- regression tests verify simple gradient output against known values
- a Fidget-backed sphere no longer reports zero offset from the cell-center lattice
- ticket notes capture the root cause and validation

## Notes

**2026-03-24T04:02:15Z**

Root cause: exedra_fidget seeded Grad inputs via Grad::from(value), which sets dx=dy=dz=0 for all three coordinate streams. That made eval_gradients return zero derivatives for Fidget-backed fields, which in turn collapsed Hermite normals and low-rank QEF placement toward cell anchors. Fix: seed x/y/z inputs with Grad::new(value, 1,0,0) / (0,1,0) / (0,0,1) to match Fidget's grad-slice convention. Validation: cargo fmt --all; typos crates/exedra_fidget/src/field.rs crates/exedra_fidget/docs/plans/ef-1dz9-fidget-gradient-seeding.md .tickets/ef-1dz9.md; cargo test -p exedra_fidget; cargo clippy -p exedra_fidget --all-targets --all-features -- -D warnings; cargo doc -p exedra_fidget --no-deps.
