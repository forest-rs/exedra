# exedra_triangulate quality wind tunnel

This executable compares deterministic `EarClip` and `ConstrainedDelaunay`
results for `exedra_triangulate`. It keeps quality reporting separate from
wall-clock sampling so formatting, angle calculation, and signature
construction are not part of the timed region.

Run the quick profile:

```sh
cargo run --release -p exedra_triangulate_bench -- --quick
```

Run the longer timing profile over the identical fixed corpus:

```sh
cargo run --release -p exedra_triangulate_bench -- --stress
```

Each fixture has one typed diagnostic role. The role describes what the
fixture is intended to probe; it does not claim that the role is the sole
cause of its output quality:

- **`choice_quality`:** an asymmetric convex quad where legal ear choices can
  produce different element quality, plus its exact power-of-two copies;
- **`tie_control`:** near-circular and exactly cocircular convex rings, plus a
  multi-hole fixture that exercises deterministic bridge ordering;
- **`input_constraint`:** sparse rectangle boundaries around dense holes, a
  drill-like loop with exactly collinear chord midpoints, and a constrained
  small-angle wedge.

Every fixture is triangulated twice with each strategy before measurement.
The quality phase checks byte-identical triangle output, positive triangle
orientation, valid input indices, polygon-area preservation, and
non-regression of the minimum angle. It also reports and pins the exact
constrained-Delaunay `edge_flips` count. The pinned signature is FNV-1a
over, in order: the scenario-name bytes, outer-loop length, ordered outer
coordinate bits, hole count, each hole length and ordered coordinate bits,
then every emitted triangle index. Lengths, coordinate bits, and indices are
encoded as little-endian `u64`; the FNV offset and prime are defined in the
benchmark source. The signature covers both the fixture and ordered result,
but is not a general interchange format or a geometric-equivalence hash.

Quality metrics are element-oriented:

- `min_angle_deg` is the worst triangle minimum angle;
- `p01_angle_deg` is the nearest-rank first percentile of per-triangle minimum
  angles;
- `worst_quality` is the minimum `twice_area / longest_edge_squared`;
- `below_{1,5,10}deg` count triangles whose minimum angle is below that
  threshold.

These metrics are diagnostics, not a general quality threshold. Edge
legalization can change choices within the same surviving input vertex set and
fixed boundary constraints. Low residual quality in a sparse-boundary,
collinear, or small-angle fixture is consistent with an input constraint, but
this corpus alone does not establish the cause or the best remedy.

The timing phase constructs each fixture's hole-reference slice and
`PolygonInput` once, outside the clock. It reports best and average end-to-end
nanoseconds per triangulation, plus both values per input vertex, including the
algorithm's ordinary scratch allocation. Small fixtures are measured in
explicit batches (`batch_size`) to reduce timer quantization; `samples` counts
timed batches and `triangulations` counts total calls. An untimed output
checksum and `black_box` prevent dead-code elimination. Cross-machine
wall-clock values are not goldens; compare profiles on the same machine and
build configuration.

The final `predicate_timing` phase separately exercises and verifies one query
on every finite typed path used by `orient2d_evaluated` and
`incircle_evaluated`: the orientation filter, normalized expansion, and
dyadic paths, plus the incircle filter and dyadic paths. Inputs are passed
through `black_box` and the returned evaluation is consumed inside the timed
loop. The reported best and average nanoseconds are per predicate call; they
measure path cost rather than polygon triangulation and, like the
triangulation timings, should only be compared on the same machine and build
configuration.
