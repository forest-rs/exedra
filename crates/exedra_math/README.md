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
- `path`: owned `Path3`, framing/connectivity intent and line/arc/cubic spans,
  plus an allocation-free borrowed curve view shared by geometry heads;
  cursor authoring, sampling, recipe encoding and mesh evaluation stay in
  their consuming crates. `Path3::xy_circle` validates a radius independently
  of sampling policy. Owned paths use `alloc` and remain `no_std`,
- `predicates`: exact-sign `orient2d`, `orient3d`, `incircle`, `insphere`
  and `plane_side`, with floating-point filters and exact fallbacks,
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

## Exact predicates

`predicates::orient2d` keeps the ordinary floating-point filter for clear
turns. Borderline narrow-span inputs are scaled by one lossless common power
of two before exact expansion arithmetic; wider exponent spans use a
fixed-size exact dyadic accumulator. This preserves the exact-sign contract
for every finite coordinate accepted by `MAX_COORDINATE`, including uniformly
tiny and subnormal geometry. `predicates::orient2d_evaluated` returns the same
sign plus a typed `Orient2dPath` diagnostic (`Filter`, `NormalizedExpansion`,
or `Dyadic`). A non-finite query instead reports `NonFiniteInput` alongside a
deterministic `Collinear` sentinel, which is not a geometric classification.
The diagnostic is per call; there are no global counters.

`predicates::orient3d` uses the same filter shape. An inconclusive query whose
coordinate differences are exact, the common case for grid-aligned and
coplanar input, expands six exact triple products of those differences;
otherwise it sums 24 exact triple products of the original coordinates in the
dyadic accumulator, so the sign stays exact through overflow and underflow.

`predicates::incircle` classifies a point against an oriented circumcircle.
Its ordinary path uses a floating-point error-bound filter; inconclusive
queries expand the homogeneous degree-four determinant into 48 monomials and
sum their exact binary64 values in fixed positive and negative limb arrays.
`incircle_evaluated` reports `IncirclePath::Filter` or `IncirclePath::Dyadic`.

`predicates::insphere` classifies a point against the circumsphere of a
positively oriented tetrahedron, in Shewchuk's orientation convention. Its
filter is Shewchuk's; inconclusive queries expand the lifted 5×5 determinant
into 360 five-factor monomials over the original coordinates and sum them in
the dyadic accumulator, so the sign is exact for every finite input.
`insphere_evaluated` reports `InspherePath::Filter` or `InspherePath::Dyadic`.

`predicates::plane_side` evaluates the exact sign of an authored plane equation
without normalizing its coefficients. A conservative floating-point filter uses
the shared dyadic accumulator when uncertain, including overflow and underflow.
It accepts all finite f64 exponents and returns `None` for non-finite input.

The filter error bounds and expansion arithmetic follow Jonathan Richard
Shewchuk's *Adaptive Precision Floating-Point Arithmetic and Fast Robust
Geometric Predicates* and its public-domain `predicates.c`.

## License

Apache-2.0 OR MIT
