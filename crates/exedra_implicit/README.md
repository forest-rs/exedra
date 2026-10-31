# exedra_implicit

Implicit fields written once as formulas, with exact gradients, interval
bounds and creases.

A field is a function of space, negative inside a solid and positive outside,
whose zero set is the solid's surface. This crate defines fields and what
consumers may ask of them. It does not extract surfaces; `exedra_isosurface`
does. It is `no_std` + `alloc` and depends only on `exedra_math`.

```rust
use exedra_implicit::{Aabb, Cuboid, Cylinder, Difference, ScalarField};

// A box drilled through.
let drilled = Difference::new(
    Cuboid::new([0.0; 3], [0.6; 3]).unwrap(),
    Cylinder::new([0.0; 3], [0.0, 0.0, 1.0], 0.3, 1.0).unwrap(),
);

let mut value = [0.0];
drilled.eval_points(&[[0.0, 0.0, 0.0]], &mut value);
assert!(value[0] > 0.0); // inside the bore

let region = Aabb::new([0.5; 3], [0.7; 3]).unwrap();
assert!(drilled.eval_interval(&region).contains(0.0));

// On an edge, a crease is reported with its tie at zero distance.
let edge = drilled.eval_crease([0.6, 0.6, 0.1]);
assert!(edge.ties[0].unwrap().distance < 1e-12);
```

## Formulas

A `Formula` is written once, generically over `Number`, and evaluated in
whichever number answers the question at hand:

- `f64` for values,
- `Dual` for values with exact gradients from one forward-mode pass,
- `Interval`, rounded outward, for bounds over a box,
- `Dual<Interval>` for bounds on the gradient over a box,
- an internal crease-tracking number for the creases the formula marks.

`Number` has no comparisons, so a formula cannot branch on a value; it
chooses with `min`, `max`, `abs` and `select`, which stay meaningful for
numbers that stand for many values at once. `min`, `max` and `abs` mark
creases: where their two sides meet the surface may turn sharply, and the
tie between them, a function zero where they meet, is reported with its
distance. `select` joins pieces that meet smoothly and marks nothing, as
where a capsule's cylinder meets its caps. `hypot` keeps gradients bounded
where a length is zero.

## The evaluation boundary

`ScalarField` is what consumers ask of a field: values and gradients at
points, in batches; bounds on values and on gradients over a box; the
creases nearest a point; and the field's claims. Every formula is a
`ScalarField`, answering each query from its single definition. A formula's
value bounds are the tightest of three sound forms: the formula evaluated on
intervals, the mean-value form `F(c) + ∇F(box)·(box - c)`, which tightens
with the square of the box, and its Lipschitz claim.

Fields that cannot be formulas, such as compiled expressions, sampled data
or caller code, implement `ScalarField` directly and answer what they can;
the defaults answer the rest soundly from their claims. `Opaque` lets such a
field join formulas as a leaf, answering each number from the field's own
queries, through the chain rule where the formula moves it.

## Claims

`Claims` state how values relate to distance, as a `Metric` (exact distance,
a Lipschitz factor, or nothing), and where the interior can lie, as a
`Support` (empty, a box established or asserted, unbounded, or unknown).
Built-in fields state them from their parameters, and combinators derive
theirs from their parts' through the arithmetic they apply, so claims are
never stated by hand for a composite.

## Fields as data

`Expr` is a field as a value: a tree of the built-in shapes, `ProfileExpr`
profiles, combinations, placements and tags, each variant holding the type it
names. It evaluates exactly as those types do, so it can be built at run
time, stored, compared, and serialized by a consumer, as constructive recipes
do for their implicit leaves.

## Built-in fields

- Shapes, exact distances with checked parameters: `Sphere`, `Cuboid`,
  `Cylinder`, `Torus`, `HalfSpace`, `Capsule`, `RoundCone`.
- Profiles in the plane, `Circle`, `Rectangle` and `HalfPlane`, lifted by
  `Extrude` and `Revolve`; any `Profile` can be.
- Combinations: `Union`, `Intersection` and `Difference`, which mark the
  creases where their parts meet; `SmoothUnion`, Quilez's polynomial blend;
  and `SmoothUnionN`, a symmetric blend of any number of parts that skips
  parts whose claims show them out of reach.
- Regions: `Tagged` labels a part, and `eval_crease` reports the region of
  the part that shapes the surface at a point, so extraction can label
  faces with the part they came from.
- Placement: `Translate`, `Scale` and `Placed`, a rigid `Placement3`.

## License

Apache-2.0 OR MIT
