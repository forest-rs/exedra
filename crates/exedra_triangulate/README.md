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
tracked as a dependency ladder: exact predicate exponent-domain repair, exact
incircle, opt-in input-index constrained Delaunay legalization, and only then
generated-vertex cap resampling after kernel face-replacement/edit-lineage
support exists. Ear clipping remains the default while those slices are
measured and reviewed.

## License

Apache-2.0 OR MIT
