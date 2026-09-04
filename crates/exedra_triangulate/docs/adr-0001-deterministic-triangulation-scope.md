# ADR-0001: Deterministic Triangulation Scope

- Status: Accepted
- Date: 2026-08-19
- Owners: Exedra maintainers

## Context

Three parts of the workspace need to triangulate concave polygons with holes,
and today none of them share an implementation:

- `exedra_mesh`'s render extraction fan-triangulates every face
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
*below* `exedra_mesh` in the dependency graph so the kernel itself can consume it.

## Decision

`exedra_triangulate` owns:

- triangulation of simple polygons with holes ([`PolygonInput`] →
  [`Triangulation`]), via deterministic ear clipping with deterministic hole
  bridging ([`TriStrategy::EarClip`]) and optional exact edge legalization
  ([`TriStrategy::ConstrainedDelaunay`]);
- exact-sign planar predicates (`orient2d` and `incircle`) used by current and
  future strategies;
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
  strategies arrive behind the same API and must each be independently
  deterministic. `EarClip` remains the default. `triangulate_with_stats`
  provides local strategy work diagnostics without global counters; the
  sign-only `triangulate` entry point remains unchanged.

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

### Exact `incircle` exponent domain

Constrained-Delaunay edge legalization needs the sign of a degree-four
incircle determinant. The additive `incircle` API follows the same calm
wrapper-and-local-diagnostic shape as `orient2d`:

1. the ordinary Shewchuk error-bound filter proves clear queries;
2. an inconclusive finite query expands the homogeneous determinant directly
   into 48 signed four-factor monomials;
3. decoded binary64 significands are multiplied into four-word magnitudes and
   accumulated in fixed positive and negative 132-limb arrays.

Direct homogeneous expansion avoids rounded coordinate differences on the
exact path. The accumulator covers the complete finite binary64 exponent
domain plus the carry from all 48 monomials: it stores through relative bit
8447 while the largest possible sum reaches at most bit 8397. The predicate
therefore remains allocation-free, dependency-free, safe, and available under
`no_std`. `incircle_evaluated` reports a typed per-call path and uses an
explicit `NonFiniteInput` sentinel contract. Existing APIs and ear-clipping
behavior are unchanged; no caller migration is required.

### Deterministic edge legalization

`ConstrainedDelaunay` begins with the same validated ear-clipped cover, then
legalizes only edges with exactly two incident triangles. True outer and hole
boundary edges have one incident triangle and cannot enter the worklist;
temporary bridge edges have two and correctly remain eligible. A convex
quadrilateral flips when the exact `incircle` determinant says the opposite
vertex is inside the circumcircle. Exact cocircular cases choose the
lexicographically smaller normalized diagonal.

Adjacency is maintained incrementally in an ordered map, and affected edges
are revisited through an ordered set. This makes both the result and the
`edge_flips` work count independent of allocation addresses or hash order.
After legalization, every triangle is rotated to its lowest vertex and the
ordered result is sorted, giving one stable representation. The operation
does not add vertices and cannot alter a constrained boundary. The API is
additive: `EarClip` remains the default and existing callers require no
migration.

The cocircular rule is the exact answer for a symbolic perturbation of the
paraboloid lift in which lower input indices are lowered slightly more than
higher ones, so the lowest index of any cocircular set lies inside every
circle through the others. Strict decisions are unchanged by an infinitesimal
perturbation, so every flip is a legal Lawson flip on a point set without
cocircular quadruples. That gives termination without an iteration guard, and
because a constrained Delaunay triangulation is unique in general position,
the legalized triangle set does not depend on the ear-clipped cover it started
from. Both properties are pinned: fans of the same convex ring from every apex
legalize to one canonical set, and the torture corpus checks every generated
polygon for unchanged boundary edges, zero remaining illegal edges, and
idempotence.

## Consequences

- `exedra_analytic` will retire its private clipper and epsilon (`ea-ds4b`).
- `exedra_mesh` render extraction and boolean split retriangulation gain a robust
  strategy without the kernel owning triangulation math (`exe-hi4e`,
  `exe-o1su`).
- Exedra brief 11 is implemented by this crate; the brief remains as design
  rationale and is annotated to point here.
- One more workspace crate — accepted because it has three consumers and zero
  dependencies, matching the `exedra_qef`/`exedra_spatial` leaf pattern.
- Uniformly tiny or wide-exponent noncollinear inputs may now change from the
  incorrect `Collinear` result to their mathematical orientation. Existing
  callers require no source migration.
