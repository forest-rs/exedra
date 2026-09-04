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
  tolerances.

Every operation is correctly rounded, so the `std` and `libm` backends produce
bit-identical results. Transcendental functions are deliberately absent; the
crates that need them keep their own backend plumbing.

This crate owns only scalar vector arithmetic. It does not own vector types,
placements, bounding boxes, or matrices.

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

The default `std` feature supplies square root. For a `no_std` build, disable
defaults and enable `libm`.

## License

Apache-2.0 OR MIT
