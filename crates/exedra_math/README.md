# `exedra_math`

Small, deterministic 3-vector helpers for the Exedra workspace.

Current scope:

- componentwise `add`, `sub`, `scale`, and `dot`/`cross` products over plain
  `[f32; 3]` and `[f64; 3]` arrays,
- `norm` and a single `normalize` contract that reports degenerate input as
  `None` instead of producing NaN,
- `distance_squared`, `lerp`, and a 3×3 `det3`,
- the `f32` ↔ `f64` `promote`/`narrow` pair kernels use at their single
  narrowing point,
- finiteness, unit-length, and orthogonal-frame predicates with explicit
  tolerances,
- affine `Placement3` matrices and `Plane3` with checked normalization,
- `Quat` rotation quaternions (Hamilton, `x, y, z, w`), with composition,
  vector rotation, rotation-matrix and rigid-`Placement3` conversions, and
  shortest-arc slerp,
- `keyed`: stable, counter-based deterministic randomness (`hash`, `mix`,
  `tag`, `Key`), a frozen cross-repository contract
  ([ADR-0001](docs/adr-0001-keyed-hash-contract.md)).

`Placement3::try_from_orthonormal_axes(x, y, z, origin, tolerance)` checks a
finite right-handed frame without changing authored values. `FrameError`
identifies non-unit axes, non-orthogonal pairs and nonpositive determinants.
Tolerance measures absolute error in squared axis length and pairwise dot
products. Use `from_axes` when scale, shear or reflection is intentional.
This API is additive; the cross-crate
[authoring contract](../exedra_constructive/docs/adr-0018-authored-geometry-diagnostics.md)
records its scope.

Vector arithmetic is correctly rounded, so its `std` and `libm` backends produce
bit-identical results. Shared `Placement3` and `Plane3` types also live here,
independent of recipes and meshes. Rotation constructors use libm when enabled,
otherwise the standard backend. Constructive explicitly selects libm to preserve
recipe evaluation arithmetic.

```rust
use exedra_math::{cross, dot, normalize};

let x = [1.0_f64, 0.0, 0.0];
let y = [0.0_f64, 1.0, 0.0];
assert_eq!(cross(x, y), [0.0, 0.0, 1.0]);
assert_eq!(dot(x, y), 0.0);
let direction = normalize([3.0_f64, 0.0, 4.0]).expect("non-degenerate vector");
assert!((direction[0] - 0.6).abs() < 1.0e-12);
assert!((direction[2] - 0.8).abs() < 1.0e-12);
```

Quaternion axis–angle construction and slerp use trigonometry, which, like the
rotation constructors, prefers libm when enabled. The keyed hash uses only
integer operations, exact conversions and single IEEE operations in a fixed
order, so its bits never depend on the backend.

The default `std` feature supplies square root. For a `no_std` build, disable
defaults and enable `libm`.

## License

Apache-2.0 OR MIT
