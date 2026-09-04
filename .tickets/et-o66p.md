---
id: et-o66p
status: closed
deps: [et-c9eu]
links: [et-c9eu, exe-ccxf, et-jmpb]
created: 2026-08-21T13:39:16Z
type: feature
priority: 2
assignee: Bruce Mitchener
---
# Add measured Steiner refinement for polygon caps

Permit deterministic generated interior vertices when boundary-only constrained-Delaunay triangulation cannot meet the measured cap-quality target.

## Design

exedra_triangulate owns deterministic planar point generation and refined triangle output; mesh integration stays in the consuming adapter. Preserve input-index provenance separately from generated vertices, enforce explicit vertex/work budgets, and measure the real sparse-hole, drill-like, and dense-boundary cases before setting any quality claim. Consume the kernel face-replacement operation only for existing-face retriangulation; constructive profile caps may build the refined patch directly.

## Acceptance Criteria

An opt-in refinement path emits deterministic generated points and CCW triangles under explicit budgets; cover, boundary preservation, termination, scale stability, and provenance are pinned; the wind tunnel demonstrates material improvement on quality-limited fixtures with reported time and work; at least one constructive cap consumer uses the result without ad hoc face surgery; no_std, safe Rust, no new production dependency.


## Notes

**2026-08-21T13:58:08Z**

Implementation boundary: exact `incircle`, boundary-preserving
constrained-Delaunay legalization, budgeted refinement, then constructive
consumers. Atomic replacement of an existing mesh face is related work, but
not a prerequisite: constructive profile caps can build their refined patch
directly without mesh surgery.

**2026-09-05T01:01:54+07:00**

Implemented `exedra_triangulate::refine`, a deterministic, budgeted,
quality-directed Ruppert variant over the constrained-Delaunay cover.
`RefineParams` controls the circumradius-to-shortest-edge bound, generated
point budget, and boundary-split policy; `RefinedTriangulation` returns every
generated point with `SteinerOrigin`, while `RefineStats` distinguishes
completed quality from budget, declined, and input-limited stops.

Boundary encroachment alone is not work: an already compliant cover is a
no-op, and angles fixed between two input boundary segments are reported
rather than chased. A candidate circumcenter checks every live boundary
segment because those deliberate stops mean the stronger global
no-encroachment invariant does not hold. All mutation remains gated by exact
`orient2d`/`incircle`, stable ordered worklists, checked `u32` ID capacity, and
an explicit hard budget.

`EvalPolicy` exposes separate planar-face and extrusion-cap refinement. Cap
refinement forces boundary splits off and restores distinct collinear input
samples before legalization, so every rim edge remains shared with side
walls without consuming the generated-point budget.
Generated face vertices retain profile-segment provenance even when the
triangulator simplified a collinear source chain across the ring-index seam. `TessellatedBody` and
`GeometryReport` retain the refinement outcome, including across cache hits;
incomplete outcomes emit typed `eval.refinement.*` diagnostics. Policy fields
participate in cache identity, with the ignored cap boundary policy normalized
out.

The wind tunnel records exact output/work signatures, quality distributions,
input-limited exceptions, predicate paths, timings, and SVG galleries. Tests
pin deterministic repetition, cover and boundary preservation, Delaunay
legality, power-of-two scale stability, provenance, no-op stopping, hostile
acute corners, bounded work, and constructive cache replay. The independent
audit additionally compares exact predicates against integer arithmetic and
checks randomized polygons, holes, refinement policies, extreme exponents,
and direct constructive provenance. Rotated outer/hole rings pin complete
boundary incidence and Delaunay legality; closed-cap and rotated source-chain
tests pin the constructive boundary contract.

Validation passed:

- `cargo test --workspace --all-features`: 1330 passed, 5 ignored.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- `cargo check -p exedra_triangulate -p exedra_constructive --no-default-features`.
- `cargo fmt --all --check`, `taplo fmt --check`, and `typos`.
- `cargo run --release -p exedra_triangulate_bench -- --quick`.
- `RUSTDOCFLAGS='--cfg docsrs -D warnings' cargo +nightly doc --workspace
  --locked --all-features --no-deps --document-private-items`: warning-free.
- `cargo doc --no-deps`: succeeds with four existing disabled-feature facade
  link warnings.
