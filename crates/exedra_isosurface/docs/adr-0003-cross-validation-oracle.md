# ADR-0003: Three-witness cross-validation oracle for CSG

## Status

Accepted (2026-08). Implemented by the `boolean_oracle` harness crate
(`benchmarks/boolean_oracle`).

## Context

Two independent CSG implementations exist in the workspace: the `exedra_mesh`
boolean pipeline (exact predicates, split/classify/stitch) and this crate's
pointwise field combinators (`analytic::Union` / `Intersection` /
`Difference` over `ScalarField`s). Each had only self-referential tests
(closed-form volumes, conservative-interval properties). Neither had ever
been checked against the other, and a two-witness disagreement cannot say
which side is wrong.

## Decision

A standing harness evaluates seeded random CSG expression trees three ways
and cross-checks point membership:

1. **Referee (ground truth).** Operands are convex, planar-faced polyhedra
   (boxes, regular n-gon prisms) whose half-space planes are derived from
   the *transformed operand mesh itself*, so the referee describes exactly
   the solid the mesh pipeline consumes. Composition is min/max/negate over
   per-operand pseudo-SDFs in f64. Sign is exact (min/max sign logic
   depends only on operand signs). Magnitude is a sound *lower bound* on
   distance to the composed boundary: each convex pseudo-SDF under-reports
   distance sign-consistently, and min/max are monotone, so growing every
   leaf magnitude never crosses zero.
2. **Mesh witness.** The boolean pipeline folded over the operand meshes,
   then exact ray-parity membership (five `orient3d` signs per triangle,
   deterministic direction ladder, degeneracy retried, never guessed).
3. **Field witness.** This crate's combinators folded over the placed
   analytic fields (`Transform3` wrappers), sign per sample point.

**Attribution.** A point is compared only when the referee margin exceeds a
band. The mesh band (1e-3 at unit scale) covers f32 narrowing and stitch
vertex rounding; a mesh disagreement beyond it is a *mesh pipeline
finding*. The field band additionally includes the per-operand Hausdorff
gap between the mesh polyhedron and the analytic solid (box: slop only;
prism vs round cylinder: chord sagitta) — min/max composition is
1-Lipschitz per operand, so the leaf-wise bound survives composition. A
field sign flip beyond that band is an *isosurface finding*. Points inside
a band are counted, never silently dropped.

**Typed deferrals are skips, not failures.** Mesh-pipeline typed refusals
(coplanar ambiguity, deferred splits, suspect classifications, Build
errors) skip the mesh comparison under a counted category; only geometric
disagreement on an `Ok` boolean is a finding.

**Determinism first.** The harness re-runs a probe case and asserts
identical outcomes before any counting; all randomness is explicit-seed
SplitMix64.

## Consequences

- Findings carry exact attribution and a one-command reproducer
  (`ORACLE_SEED=<seed> cargo test -p boolean_oracle --release isolate_case
  -- --ignored --nocapture`), which prints per-stage referee/parity/volume
  and per-operand attribution for disagreeing points.
- First sweep (seed 1, 400 cases x 2000 points, 2026-08): the field
  witness was clean; the mesh pipeline produced 85 disagreements across 6
  cases plus 5 Build failures — filed as exe-04ex, exe-dnny, exe-3ir1.
  The expectation recorded on ei-c48a (first harvest = isosurface bugs)
  inverted; the harness is the arbiter either way.
- Curved operands (spheres, torus) are out of scope until a referee with
  the same exactness properties exists for them; adding them must not
  weaken attribution to "statistical".
- `ScalarField` gained `&F` / `Box<F>` forwarding impls (ei-8w1z) so
  runtime-shaped field trees compose through the combinators.

## Amendment: scenario taxonomy (2026-08, bo-6o04)

The single convex-operand family could not generate the failure class that
produced the first real findings (cut-bound sliver cascades on collinear
cut runs — exe-dnny) except by luck, and never exercised stitched meshes
as *inputs*. The harness now runs seven seeded scenario classes, each with
per-class reporting (`class.<key>.*` lines), counted typed-deferral skips,
and a `--class` CLI filter:

- **convex_mixed** — the original family: boxes and 8–24-gon prisms under
  random rigid placements, left-fold trees.
- **curved_wall** — cylindrical prisms up to 96 segments: collinear cut
  runs and sliver cascades along curved walls by construction.
- **nonconvex** — L/U-shaped prisms carried as single watertight meshes.
  The referee generalizes to a *union of convex pieces* (`min` over pieces
  of `max` over planes): each piece is one more min/max leaf, so the
  value-space soundness argument is unchanged. Piece planes come from an
  analytic box decomposition mapped through the rigid transform in f64
  (no Newell slop); the witness mesh deviates from them by at most the f32
  narrowing the mesh band already covers. Pieces deliberately overlap so
  interior referee margins stay healthy at decomposition seams.
- **chained** — balanced trees `(a op b) op (c op d)`: intermediate
  pipeline outputs, with their stitched cut-curve vertex structure, feed
  back in as operands.
- **adversarial** — axis-aligned boxes on an exact 1/64 dyadic lattice
  (f32-exact contact arithmetic) in deliberate sub-modes with per-sub-mode
  run/skip counters: face_flush, shared_edge, shared_vertex, tiny_overlap
  (2^-20), near_touch (2^-20), containment_near (63/64 scaled copy).
- **scale** — the convex family at coordinate scales 1e-3 and 1e4.
  Comparison bands and near-surface sampling offsets scale linearly; the
  operand convexity sanity check is extent-relative.
- **empty_total** — result-shape contract edges: disjoint difference /
  intersection / union, contained intersection / difference (internal
  cavity) / union, and identical-placement difference / intersection.
  Zero-face results are counted (`empty_results`), never inferred.

A class whose cases mostly typed-defer is itself a result: the skip map
documents the pipeline's real envelope per configuration family.

## Amendment: typed semi-analytic extraction suite (2026-08, `bo-5zl0`)

The seeded membership witness intentionally erases runtime-shaped field trees
behind `Box<dyn ScalarField>`. That is the correct boundary for evaluating
field sign, but it cannot preserve the optional `SemiAnalyticField`
capability. Semi-analytic extraction therefore has a separate fixed suite in
the same harness. Its expression trees remain statically typed through
`Union`, `Intersection`, and `Difference`.

The suite extracts aligned box/cylinder pairs at scales `1e-3`, `1`, and
`1e4`. Every scenario must validate deeply, attribute faces to both primitive
identities, snap at least one verified seam candidate, stay within a
scale-relative equation-residual bound, and reproduce the same triangle-mesh
signature and counters. A rotated pair separately proves that the declared
closed-form envelope takes a counted unsupported QEF fallback without
corrupting topology. Additional translated and `UniformScale`-wrapped
through-cuts validate that the optional capability survives the public field
adapters used by modeled scenes.

Feature residual measurement does not filter vertices by the residual it is
trying to prove. It first derives a topology-only candidate set: vertices
incident to faces from both primitive identities. It then sorts that fixed set
by the maximum unsigned implicit residual for the finite box and cylinder
surface equations and measures the best `feature_snaps` entries claimed by the
extractor. The worst of those entries must satisfy the scale-relative bound,
and the total candidate count must equal
`feature_snaps + ambiguous_fallbacks`. The displaced Union fixture records four
expected multi-component `Ambiguous` cells, which explains why its
topology-derived seam set is larger than its snapped subset; every other
aligned typed fallback count is zero. The rotated fixture requires every active
cell to report `Unsupported` and compares its positions and indices directly
with ordinary QEF extraction.

`--feature-obj` writes the unit-scale through-cut to
`target/boolean_oracle/semi_analytic_box_cylinder.obj`. The local writer emits
vertices in stable mesh order and faces grouped by `FACE_REGION`; it adds no
serialization dependency and cannot write outside the target subtree. The
artifact is deliberately generated rather than checked in.

The non-published harness directly depends on the existing workspace
`exedra_spatial` and `exedra_qef` crates only because constructing the public
`DualContourParams` struct requires naming `Aabb` and `QefParams`. Re-exporting
those types or adding a core convenience API solely for this harness would
widen the core surface for the wrong reason.
