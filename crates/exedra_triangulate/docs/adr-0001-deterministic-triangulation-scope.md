# ADR-0001: Deterministic Triangulation Scope

- Status: Accepted
- Date: 2026-08-19
- Owners: Exedra maintainers
- Ticket: `et-at1d`

## Context

Three parts of the workspace need to triangulate concave polygons with holes,
and today none of them share an implementation:

- `exedra`'s render extraction fan-triangulates every face
  (`Mesh::triangulate_face_fan`), which silently produces overlapping or
  inverted triangles for non-convex ngons.
- `exedra_analytic` carries a private ear clipper with its own hardcoded
  `PLANAR_EPSILON`, and it takes the first valid ear in ring order — not the
  lowest-stable-index tie-break that Exedra brief 11 (deterministic
  triangulation strategy) prescribes.
- The upcoming constructive geometry head must triangulate flattened profile
  caps (concave, holed) while preserving per-vertex provenance, and the
  boolean pipeline will need face retriangulation after splitting.

Triangulation sits under caches, goldens, and cross-platform fingerprints, so
determinism is an invariant, not a preference. A shared facility must sit
*below* `exedra` in the dependency graph so the kernel itself can consume it.

## Decision

`exedra_triangulate` owns:

- triangulation of simple polygons with holes ([`PolygonInput`] →
  [`Triangulation`]), via deterministic ear clipping with deterministic hole
  bridging ([`TriStrategy::EarClip`]);
- the exact-sign orientation predicates (adaptive `orient2d`) those
  algorithms rely on;
- a typed failure taxonomy ([`TriError`]) distinguishing invalid-input
  classes from internal invariant violations — the triangulator never panics
  and never returns garbage triangles.

Contract points:

- **Determinism.** Identical input bits produce identical output on every
  platform and build mode. Only f64 comparisons and exact-sign predicates —
  no transcendentals, no ambient epsilons, no hash-order iteration.
  Ambiguous ear choices resolve by lowest stable input index, implementing
  brief 11's tie-break rule.
- **No invented points.** Every output vertex is an input vertex (no Steiner
  insertion), so callers carry per-vertex provenance through unchanged.
- **Index-based boundary.** Inputs are f64 coordinate slices, outputs are
  `u32` indices into their concatenation. The crate knows nothing about
  meshes, curves, tolerances, or Exedra ID types, and has zero dependencies
  (`no_std` + `alloc`).
- **Strategy seam.** `TriParams::strategy` is `#[non_exhaustive]`; later
  strategies (monotone decomposition, constrained Delaunay quality passes)
  arrive behind the same API and must each be independently deterministic.
  Ear clipping is O(n²), which is acceptable at profile-cap sizes; a quality
  pass is deferred until a consumer demonstrably needs it.

### Exact `orient2d` exponent domain

The original `orient2d` fallback accumulated exact products in binary64
expansions over the unscaled coordinates. Bounding magnitudes by
`MAX_COORDINATE = 1e100` prevented overflow, but did not prevent underflow:
the determinant of `[0, 0]`, `[1e-300, 0]`, `[0, 1e-300]` is positive even
though both the filter and original fallback rounded its `1e-600` value to
zero. The magnitude bound alone therefore did not prove the documented exact
sign contract.

`orient2d` now has three deterministic paths:

1. the ordinary Shewchuk error-bound filter remains the hot path;
2. after an inconclusive filter, a common power-of-two normalization is used
   only when every nonzero coordinate remains normal and every possible
   error-free-transform product bit remains above the subnormal boundary;
3. wider exponent spans are decoded as signed binary64 significands and
   exponents, then the six determinant products are accumulated into fixed
   positive and negative limb arrays before an exact magnitude comparison.

The dyadic arrays are statically sized for the complete finite binary64
product exponent span plus carry, although triangulation continues to validate
the smaller `MAX_COORDINATE` envelope. This keeps the core zero-dependency,
`no_std`, allocation-free per query, and free of `unsafe`. A local
`orient2d_evaluated` diagnostic returns `Orient2dPath` with the exact sign;
`orient2d` remains its source-compatible sign-only wrapper. No global path
counters are introduced. Non-finite direct predicate queries explicitly
report `Orient2dPath::NonFiniteInput` with a deterministic `Collinear` sentinel
rather than falsely attributing that sentinel to an exact arithmetic path.
This standardizes previously unspecified out-of-domain NaN and infinity
behavior, which could return a different orientation; finite-domain semantics
and the existing function signature remain compatible.

The crate intentionally does not own: 3D triangulation, plane projection
(callers project), polygon repair (self-intersecting input is rejected with a
typed error, never auto-fixed), or mesh integration (adapters live with their
consumers).

## Consequences

- `exedra_analytic` will retire its private clipper and epsilon (`ea-ds4b`).
- `exedra` render extraction and boolean split retriangulation gain a robust
  strategy without the kernel owning triangulation math (`exe-hi4e`,
  `exe-o1su`).
- Exedra brief 11 is implemented by this crate; the brief remains as design
  rationale and is annotated to point here.
- One more workspace crate — accepted because it has three consumers and zero
  dependencies, matching the `exedra_qef`/`exedra_spatial` leaf pattern.
- Uniformly tiny or wide-exponent noncollinear inputs may now change from the
  incorrect `Collinear` result to their mathematical orientation. Existing
  callers require no source migration.
