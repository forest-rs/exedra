# ADR-0008: Constructive evaluation hardening

## Status

Accepted (review-hardening).

## Boundary

The constructive head owns recipe identity, placement, evaluation completeness,
and source attribution; it does not own catalog material definitions or mesh
Boolean algorithms. The mesh head owns typed attribute storage and edit tracking.

## Decisions

- Imported-mesh fingerprints cover every accepted built-in attribute, including
  UVs, authored normals, regions, seams, and sharpness. Imports carrying custom
  layers are refused because their values cannot be canonically encoded by the
  constructive head. Evaluation schema 13 invalidates schema-12 identities.
- Mesh layer registration is exposed separately from edit scopes. Registration
  does not advance mesh revision or record changes; subsequent value edits use
  the existing edit-session contract.
- Imports and instances share one placement path. Positions transform in f64
  and narrow once to f32; overflow and newly collapsed face edges are typed
  errors. Authored normals use the inverse transpose and renormalization.
  Reflections reverse face loops and preserve source attribution through the
  rebuild's element correspondence.
- A CSG operand must finish without error diagnostics. A partially emitted
  subtree cannot become an exact operand: its parent withholds geometry and
  reports `eval.csg.incomplete_operand` without caching a partial result.
- Boolean vertex attribution comes from incident face operands. Vertices shared
  by multiple operands use `Feature::BooleanSeam`; other vertices retain the
  sole operand. CSG lists are bounded by `MAX_CSG_OPERANDS` (65,535).
- Fan triangulation remains the CSG path. Known fan-unsafe operand faces produce
  `eval.csg.fan_unsafe_faces` warnings and a work counter. This is a measured
  limitation, not a guarantee that the resulting geometry is verified. Switching
  to robust triangulation is deferred because it refuses existing chained cuts.
- Output body, face, vertex, and source-map byte counters are derived from the
  final emitted bodies. Work counters continue to measure evaluation activity.
- Loft correspondence is authored segment structure: matching hole counts and
  segment counts per loop, with segment positions defining correspondence.
  Each corresponding segment family takes the largest required edge count.
  Supplied counts are lower bounds within the policy work cap, never permission
  to coarsen curves or exceed the cap. Structural mismatches and degenerate
  lofts become envelope-only evaluator diagnostics; direct tessellation retains
  typed errors.

## Alternatives and consequences

Silently omitting attributes from identity or accepting partially evaluated CSG
would allow plausible but stale or incomplete output. Explicit refusal keeps
those cases observable. Generic custom-layer serialization remains outside this
slice rather than adding a registry or production dependency.

Matching lofts by total sampled point count couples author intent to tolerance.
Matching by segment structure makes correspondence explicit and supports different
radii and line-to-curve transitions, at the cost of refining smaller sections to
match the most demanding section. Arbitrary profile correspondence remains a
caller responsibility.

The separate `constructive_probe` example exercises composed recipes, geometry
validity, provenance freshness, cache identity, and GLB export. Its known refusals
are pinned alongside successful outcomes.

## Migration

- Recompute persisted recipe and assembly fingerprints and discard evaluation
  caches from schema 12. Wire-format v1 remains unchanged.
- Handle `RecipeError::UnsupportedImportLayer` and `TooManyOperands`,
  `TessellateError::CollapsedGeometry`, and `Feature::BooleanSeam` in exhaustive
  matches. Keep unsupported custom import metadata outside the imported mesh.
- Include `csg_fan_unsafe_faces` when constructing `EvalCounters` explicitly.
  Consumers requiring verified output must inspect warnings as well as fidelity.
- Expect invalid lofts to return an evaluation report with error diagnostics
  rather than aborting the whole recipe. Check the report before export.
- Author corresponding loft segments in the same order; matching total point
  counts alone no longer suffices. New count-based discretization helpers may
  return `ToleranceBudgetExceeded` for caller-requested counts over the cap.
- Reflecting instance placements are now supported; the previous
  `eval.instance.reflecting` refusal is no longer emitted.
