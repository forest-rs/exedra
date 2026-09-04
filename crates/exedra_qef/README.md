# `exedra_qef`

Small, deterministic quadratic-error-function solving for Exedra. It is the
cell-local fitting kernel used by dual-contouring extraction, and can also be
used directly without pulling in fields, octrees, or meshes.

Current scope:

- accumulate plane constraints from position + normal pairs,
- solve a 3×3 QEF without external linear algebra dependencies,
- support explicit low-rank anchoring so callers can bias null-space dimensions
  toward a mass point instead of always using the bounds center,
- classify the local feature as smooth, edge, or corner from solver rank,
- expose residual error for extraction-time quality checks.

The solver normalizes accepted normals, solves the unconstrained least-squares
system, anchors rank-deficient directions, and finally clamps the point to the
requested axis-aligned bounds. The last step keeps output inside a cell; it is
not a general box-constrained optimization algorithm.

`QefBounds::new` rejects non-finite or reversed bounds. The solve methods also
validate plain struct values, parameters, and custom anchors, returning a
typed `QefSolveError` rather than allowing invalid numerics into the solver.

```rust
use exedra_qef::{QefBounds, QefParams, QefSolver, SharpnessClass};

let mut solver = QefSolver::new();
assert!(solver.add([0.25, 0.0, 0.0], [1.0, 0.0, 0.0]));
assert!(solver.add([0.0, -0.5, 0.0], [0.0, 1.0, 0.0]));
assert!(solver.add([0.0, 0.0, 0.75], [0.0, 0.0, 1.0]));

let bounds = QefBounds::new([-1.0; 3], [1.0; 3]).expect("ordered bounds");
let result = solver
    .solve(bounds, &QefParams::default())
    .expect("usable constraints");

assert_eq!(result.position, [0.25, -0.5, 0.75]);
assert_eq!(result.sharpness_class, SharpnessClass::Corner);
```

This crate is `no_std`. Its default `std` feature supplies square root; use
`default-features = false, features = ["libm"]` in a `no_std` application.

## Boundary

`exedra_qef` owns only plane accumulation, fitting, rank classification, and
residual reporting. Hermite sampling, spatial traversal, field evaluation,
and mesh extraction belong to their respective crates.

## License

Apache-2.0 OR MIT
