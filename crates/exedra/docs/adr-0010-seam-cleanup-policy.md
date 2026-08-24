# ADR-0010: Geometry-conservative boolean seam cleanup

## Status

Accepted (exe-zk3b).

## Context

Boolean outputs concentrate degenerate triangles along their seam rings:
cut-loop vertices are f32-narrowed f64 constructions, wall-triangle
diagonals cross cut planes at exact chord midpoints, and drilled-face
re-facing reinserts exactly-collinear rim vertices. The result is
near-zero-area cap slivers hugging every ring. With `collapse_edge` and
`flip_edge` in the kernel (ADR-0009), a cleanup pass can remove them —
the question is under what contract.

Boolean stitching first canonicalizes graph constructions that land on
the same stored `f32` position within one connected seam component. If
that identity merge pinches a selected source face, stitching decomposes
its boundary walk into simple cycles and repeats the source provenance for
each emitted face. This is intentionally topology-scoped: equal positions
in disconnected seams or shells remain distinct. Cleanup therefore receives
identity-canonical seams and is not a positional welding fallback.

## Decision 1: geometry-conservative, refusal over degradation

`boolean::cleanup_seams(&mut Mesh, &SeamCleanupPolicy)` never moves a
vertex: collapses keep the surviving vertex's authored position and
flips only rewire connectivity. Volume change is therefore local and
exactly computable per candidate (per the fan-volume metric), and the
pass budgets cumulative absolute drift against an explicit fraction of
the mesh's initial volume. Every candidate the pass cannot improve
safely is skipped and counted by reason — kernel precondition, seam
guard, region guard, budget, or quality guard — never approximated.

This deliberately positions the pass below any future quality remesher:
it fixes the boolean pipeline's own numerical debris without acquiring
opinions about mesh aesthetics.

## Decision 2: guard semantics

- **Seam rings stay closed and in place.** A seam edge may collapse (its
  ring shortens by one near-collinear rim vertex but stays a closed
  ring of authored positions). A collapse whose removed vertex lies on
  a seam while the edge itself is not a seam edge is refused — it would
  drag ring geometry to the survivor. Seam edges never flip — the flip
  would erase them from their ring.
- **`FACE_REGION` fidelity.** A triangle dropped by a collapse must
  leave its region represented on an adjacent surviving face; flips
  require both triangles to share one region value. Region coverage can
  therefore shift only by the (budgeted, near-zero) sliver areas.
- **Strict quality improvement for flips.** The flipped pair's worst
  normalized quality (area over squared longest edge, compared in
  squared form — the pass is square-root-free) must strictly beat the
  current pair's, and both new triangles must agree with the local
  orientation. Strict improvement makes flip sequences finite; the
  explicit op budget bounds the pass regardless.

## Decision 3: determinism and thresholds

Candidates are visited in ascending face-id order within bounded
rounds; all geometry is plain f64 arithmetic over exactly promoted f32
positions; every threshold (needle ratio, cap sliver quality, volume
budget, op and round caps, seam scope) lives in `SeamCleanupPolicy`
with documented defaults — no hidden constants. A zero volume budget
still admits ops whose drift is exactly zero: free ops are free.

Scope: triangle faces only. Sliver quads do not arise from the current
pipeline (walls stay rectangular; caps re-face to triangles); extending
candidate selection to polygons is future work if a producer appears.

## Consequences

- The drill fixture's worst squared triangle quality improves from
  ~9.3e-16 to ~3.5e-5 with two flips and ~7e-18 volume drift; healthy
  outputs (touching-box unions) pass through untouched.
- cambium's `BooleanRunPolicy` gains an opt-in `seam_cleanup` field
  (default `None`); enabling it runs the pass after tiny-component
  removal, reports `SeamCleanupStats` on the commit, and prunes
  provenance rows for faces the collapses removed.
- The pass composes with the cross-validation oracle unchanged: it
  moves no vertices and only removes/rewires degenerate triangles, so
  point-membership classification is unaffected within the drift
  budget.
