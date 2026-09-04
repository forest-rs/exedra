# exedra_triangulate

Deterministic 2D polygon triangulation for concave loops with holes.

This crate owns triangulation of simple polygons with holes: deterministic ear
clipping with deterministic hole bridging, plus opt-in constrained-Delaunay
edge legalization. It also owns the exact-sign planar predicates those
strategies rely on. Identical input bits produce identical output triangles
on every platform and build mode: only f64 comparisons and exact-sign
predicates, no transcendentals, no ambient epsilons. Every `triangulate`
output vertex is an input vertex, so callers carry per-vertex provenance
through that operation unchanged. The separate `refine` entry point may add
vertices and reports the origin of each one.

Inputs are f64 coordinate slices; outputs are `u32` indices into their
concatenation. The crate is `no_std` + `alloc` with zero dependencies and
knows nothing about meshes, curves, or tolerances — adapters live with their
consumers (`exedra_mesh` render extraction, `exedra_analytic`, the constructive
geometry head).

```rust
use exedra_triangulate::{PolygonInput, TriParams, triangulate};

let square = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]];
let result = triangulate(
    &PolygonInput {
        outer: &square,
        holes: &[],
    },
    &TriParams::default(),
)
.expect("simple CCW polygon");

assert_eq!(result.len(), 2);
```

See the [triangulation scope](https://github.com/forest-rs/exedra/blob/main/crates/exedra_triangulate/docs/adr-0001-deterministic-triangulation-scope.md)
for contract details.

## Exact predicate exponent domain

`predicates::orient2d` keeps the ordinary floating-point filter for clear
turns. Borderline narrow-span inputs are scaled by one lossless common power
of two before exact expansion arithmetic; wider exponent spans use a
fixed-size exact dyadic accumulator. This preserves the exact-sign contract
for every finite coordinate accepted by `MAX_COORDINATE`, including uniformly
tiny and subnormal geometry.

`predicates::orient2d_evaluated` returns the same sign plus a typed
`Orient2dPath` diagnostic (`Filter`, `NormalizedExpansion`, or `Dyadic`). A
non-finite query instead reports `NonFiniteInput` alongside `orient2d`'s
deterministic `Collinear` sentinel; that sentinel is not an exact geometric
classification. The diagnostic is per-call and has no global counters. The
existing `orient2d` function keeps its source-compatible signature and all
finite inputs in the documented domain retain their contract. Out-of-domain
NaN and infinity behavior is now standardized and may differ from prior
incidental results. Inputs whose nonzero determinant previously underflowed to
zero now receive the mathematically correct orientation.

`predicates::incircle` classifies a point against an oriented circumcircle.
Its ordinary path uses a floating-point error-bound filter; inconclusive
queries expand the homogeneous degree-four determinant into 48 monomials and
sum their exact binary64 values in fixed positive and negative limb arrays.
`incircle_evaluated` reports `IncirclePath::Filter` or
`IncirclePath::Dyadic` without global counters. Ear clipping does not pay for
the new predicate.

## Constrained-Delaunay legalization

Set `TriParams::strategy` to `TriStrategy::ConstrainedDelaunay` to legalize
the unconstrained interior edges of the ear-clipped cover. Polygon and hole
boundaries remain fixed. Edges are visited in canonical index order; exact
cocircular choices retain the lexicographically smaller diagonal. The result
therefore stays deterministic without an epsilon or hash-order dependency.

```rust
use exedra_triangulate::{PolygonInput, TriParams, triangulate};

let outer = [[0.0, 0.0], [2.0, 0.0], [1.8, 1.0], [0.0, 1.0]];
let result = triangulate(
    &PolygonInput {
        outer: &outer,
        holes: &[],
    },
    &TriParams::constrained_delaunay(),
)
.expect("simple polygon");
assert_eq!(result.triangles.len(), 2);
```

`triangulate_with_stats` returns the same `Triangulation` as `triangulate`
plus a per-call `edge_flips` count. `EarClip` remains the default.

## Budgeted refinement

`refine` starts from the constrained-Delaunay cover and inserts generated
vertices until every triangle's circumradius-to-shortest-edge ratio is at
most `RefineParams::max_radius_edge_ratio`, or a hard vertex budget stops
the work. The default bound `sqrt(2)` guarantees a 20.7° minimum angle for
inputs whose own angles allow it; `1.0` asks for 30°. Boundary segments that
block required quality work split at their rounded midpoint before the worst
remaining triangle receives its circumcenter; a circumcenter that would
encroach a segment is withheld and the segment splits instead. Ordering is
worst-first with index tie-breaks, so the output is deterministic and, under
power-of-two scaling, exactly scaled.

An already compliant legalized cover is returned unchanged. Refinement is a
quality operation, not an implicit conforming-Delaunay pass: an encroached
boundary segment is split only when it blocks work needed for the requested
quality bound.

Unlike `triangulate`, the result may contain vertices that are not input
vertices. `RefinedTriangulation::points` holds the input concatenation
followed by generated points, and `steiner` names each generated point's
origin: the input boundary edge it splits, or the interior. Callers that
cannot accept new boundary vertices choose `BoundarySplits::Forbidden` and
accept weaker quality near the boundary. `RefineStats` reports generated,
declined, remaining, and input-limited counts so a budget or an input angle
below the bound is visible rather than silent.

```rust
use exedra_triangulate::{PolygonInput, RefineParams, refine};

let outer = [[0.0, 0.0], [8.0, 0.0], [8.0, 1.0], [0.0, 1.0]];
let result = refine(
    &PolygonInput {
        outer: &outer,
        holes: &[],
    },
    &RefineParams::new(1.0).with_max_steiner_points(128),
)
.expect("simple polygon");
assert!(result.points.len() >= outer.len());
```

## Quality wind tunnel

The top-level `exedra_triangulate_bench` executable compares deterministic
EarClip and ConstrainedDelaunay quality, work, and timing over fixtures typed
as choice-quality, tie-control, or input-constraint probes. Those roles
describe diagnostic intent rather than a proven cause of each result. The
executable intentionally lives outside this zero-dependency core crate:

```sh
cargo run --release -p exedra_triangulate_bench -- --quick
```

The benchmark is measurement, not a hidden quality policy. Input-index edge
legalization improves diagonal-choice failures but cannot remove poor angles
forced by sparse or uneven boundary samples; `refine` is the opt-in remedy
for those, at the cost of generated vertices that consumers must carry.

## License

Apache-2.0 OR MIT
