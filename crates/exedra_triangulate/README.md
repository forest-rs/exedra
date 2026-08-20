# exedra_triangulate

Deterministic 2D polygon triangulation for concave loops with holes.

This crate owns triangulation of simple polygons with holes (deterministic ear
clipping with deterministic hole bridging) and the exact-sign orientation
predicates those algorithms rely on. Identical input bits produce identical
output triangles on every platform and build mode: only f64 comparisons and
exact-sign predicates, no transcendentals, no ambient epsilons. Every output
vertex is an input vertex, so callers carry per-vertex provenance through
triangulation unchanged.

Inputs are f64 coordinate slices; outputs are `u32` indices into their
concatenation. The crate is `no_std` + `alloc` with zero dependencies and
knows nothing about meshes, curves, or tolerances — adapters live with their
consumers (`exedra` render extraction, `exedra_analytic`, the constructive
geometry head).

See `docs/adr-0001-deterministic-triangulation-scope.md` for scope and
contract details.

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

## Quality wind tunnel

The top-level `exedra_triangulate_bench` executable records a deterministic
EarClip quality and timing baseline over fixtures typed as choice-quality,
tie-control, or input-constraint probes. Those roles describe diagnostic
intent rather than a proven cause of each result. The executable intentionally
lives outside this zero-dependency core crate:

```sh
cargo run --release -p exedra_triangulate_bench -- --quick
```

The benchmark is measurement, not a hidden quality policy. Follow-up work is
tracked as a dependency ladder: exact incircle, opt-in input-index constrained
Delaunay legalization, and only then
generated-vertex cap resampling after kernel face-replacement/edit-lineage
support exists. Ear clipping remains the default while those slices are
measured and reviewed.

## License

Apache-2.0 OR MIT
