# `exedra_curves`

Deterministic curve geometry for the Exedra workspace and its consumers.

Current scope:

- `CubicBasis`: Bezier, uniform B-spline, Catmull-Rom and Hermite as
  explicit basis matrices (the `RenderMan` table that OpenUSD's
  `UsdGeomBasisCurves` documents), evaluated, with first and second
  derivatives, for controls of any dimension, so positions, widths and
  colors follow a span alike;
- spans as four control indices chosen by the caller, `stations` at an
  explicit count per span, and `sample`, `sample_derivative` and
  `sample_linear` at those stations;
- `Nurbs`: rational B-splines of any order, validated once into a
  `ValidNurbs`, evaluated with derivatives by de Boor's algorithm over
  homogeneous controls, sampled per nonempty knot span of a parameter range
  with the jumps at full-multiplicity knots sampled on both sides and marked
  as breaks, and prepared as per-span Bezier controls (`PreparedNurbs`) for
  repeated evaluation;
- `rotation_minimizing_frames` and `rotation_minimizing_frames_along`: the
  double-reflection transport `exedra_math::path::double_reflection` that
  the constructive sweeps use too, along estimated or supplied tangents,
  with repeated points, cusps and a closed loop's signed seam twist
  reported;
- an `_into` form of every allocating function, filling a caller's buffers
  with a reusable `Workspace` for scratch, so a retained caller allocates
  nothing in steady state.

The crate is format-free: which controls form which span (stepping,
wrapping, end conditions), how a format lays out per-control data, and how
finely to sample are the caller's. The crate documentation states its
contracts: explicit counts, reported (not repaired) degeneracy,
discontinuous NURBS sampled on both sides of each break, and which results
are bitwise deterministic across platforms.

```rust
use exedra_curves::{Nurbs, CubicBasis, sample, stations};

// One Bezier span sampled at its ends and middle.
let controls = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
let points = sample(CubicBasis::Bezier, &controls, &[[0, 1, 2, 3]], &stations(1, 2, false)).unwrap();
assert_eq!(points[1], [0.5, 0.75, 0.0]);

// The exact quarter circle.
let quarter = Nurbs {
    order: 3,
    knots: &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
    points: &[[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
    weights: Some(&[1.0, core::f64::consts::FRAC_1_SQRT_2, 1.0]),
};
assert_eq!(quarter.validate().unwrap().sample(None, 8).unwrap().points.len(), 9);
```

`no_std` + `alloc`; depends only on `exedra_math`. Features `std` (default)
and `libm` select `exedra_math`'s backend.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your
option.
