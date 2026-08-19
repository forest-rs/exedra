# boolean_oracle

Dual-witness cross-validation oracle: seeded random CSG expression trees are
evaluated by the exedra mesh boolean pipeline and by the `exedra_isosurface`
field combinators, and both are checked point-by-point against a closed-form
convex half-space referee that attributes every disagreement to the
responsible witness.

Quick profile:

```sh
cargo run --release -p boolean_oracle
```

Deep sweep:

```sh
cargo run --release -p boolean_oracle -- --seed 1 --cases 400 --points 2000
```

Output follows the wind-tunnel `key=value` convention; the determinism
oracle runs before any counting. Typed mesh-pipeline deferrals (coplanar
contact, deferred split configurations) are counted skip categories, never
failures.
