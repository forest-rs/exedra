# ADR-0003: Three-witness cross-validation oracle for CSG

## Status

Accepted (2026-08). Implemented by the `boolean_oracle` harness crate
(`benchmarks/boolean_oracle`), ticket ei-c48a.

## Context

Two independent CSG implementations exist in the workspace: the exedra mesh
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
