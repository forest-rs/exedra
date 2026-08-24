---
id: exe-qnt2
status: closed
deps: []
links: []
created: 2026-08-24T15:04:00Z
type: task
priority: 1
assignee: Bruce Mitchener
---
# Harden topology validation and continue Boolean performance work

The Boolean performance series exposed a public add_face boundary-continuation panic and made Mesh::from_vertex O(1), requiring an independent face-loop continuity check. CT3 sampling also shows repeated coplanar face proofs and half-edge lookup/index maintenance as the next structural hotspots.

## Design

Return typed, atomic errors for non-manifold vertex additions; validate consecutive face-loop endpoints independently of traversal accessors; then benchmark isolated structural optimizations against the gallery-shaped CT3 wind tunnel. Keep exact predicates and triangulation policy unchanged, add no production dependency, and retain only measured wins.

## Acceptance Criteria

Public add_face returns a typed error without mutation for vertex-only manifold violations. validate_deep detects face-loop endpoint discontinuity. The public error migration is documented and future-compatible. Each retained optimization improves CT3 without changing semantic oracle results. Seeded mesh/isosurface/closed-form validation and the workspace definition of done pass.


## Notes

**2026-08-24T16:13:03Z**

Implementation summary: add_face now rejects vertex-only non-manifold insertion atomically with a non-exhaustive typed error; validate_deep independently checks consecutive face-loop endpoints; deferred face partitions keep graph vertices symbolic until the plan is accepted; and stored-position seam aliasing is returned as typed numerical uncertainty rather than an invalid build. Boolean phases reuse immutable coplanar proofs and phase-local edge indexes. Edit sessions maintain one transient per-vertex incoming/outgoing boundary index, refresh it explicitly after face insertion, and repair deletion frontiers locally. Small hot public topology accessors and the orient3d wrapper are inline; the exact adaptive predicate body remains out of line. Tradeoffs: exact predicate semantics and ear-clipping policy are unchanged, no dependency or SIMD was added, and the sampled branch-heavy expansion arithmetic is not a useful SIMD target. The scale sliver seed 6750632535653330089 remains a safe typed refusal; repairing that sub-f32 seam locally is future work. Public migration: AddFaceError and ValidationError are non-exhaustive, so downstream matches need a wildcard arm. No ADR was added because this repairs established topology contracts and private implementation costs rather than establishing a new ownership or public semantic decision; durable rationale lives beside the invariants, public rustdoc, and regressions. Validation: typos; taplo fmt --check; cargo fmt --all -- --check; cargo clippy --workspace --all-targets --all-features -- -D warnings; cargo test --workspace --all-features; cargo doc --workspace --all-features --no-deps. Deep Boolean oracle seeds 1, 2, and 3 covered 8,400 generated cases with zero mesh/field disagreements or topology, bookkeeping, and seam findings; final seed-2 rerun was clean with the scale sliver counted as other_suspect. On the pinned CT-3 gallery fixture, best-of-eight release timings moved from about 99.8 ms to 10.0 ms constructive, 2.30 ms to 0.65 ms direct Boolean, and 5.45 ms to 0.52 ms rounding, with all three output signatures unchanged.
