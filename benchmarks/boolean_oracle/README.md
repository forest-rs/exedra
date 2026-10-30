# boolean_oracle

Dual-witness cross-validation oracle: seeded random CSG expression trees are
evaluated by the `exedra_mesh` boolean pipeline and by the `exedra_implicit`
field combinators, and both are checked point-by-point against a closed-form
convex half-space referee that attributes every disagreement to the
responsible witness. A separate fixed suite meshes box/cylinder combinations
with `exedra_isosurface`'s restricted Delaunay extractor and checks that
their sharp seams and face regions come out exact.

Quick profile:

```sh
cargo run --release -p boolean_oracle
```

Deep sweep:

```sh
cargo run --release -p boolean_oracle -- --seed 1 --cases 400 --points 2000
```

Target the curved-on-curved intersection family, which varies sphere
tessellation density, overlap depth, operand order, and independent mesh
orientation while comparing the result with both exact faceted half-spaces
and analytic sphere fields:

```sh
cargo run --release -p boolean_oracle -- --class curved_surface --seed 1 --cases 200 --points 2000
```

Reproduce one reported case without reapplying the batch-seed expansion:

```sh
cargo run --release -p boolean_oracle -- --class chained --case-seed 16616483859594386079 --points 2000
```

The fixed suite runs on every invocation. It covers `Union`, `Intersection`,
and `Difference` of a `Tagged` box and cylinder at scales `1e-3`, `1`, and
`1e4`, plus a bore along a diagonal and `Translate`- and `Scale`-wrapped
through-cuts. Each mesh must be complete, deterministic and free of
deep-topology findings, with every face in one of the two primitive regions.
The seam set is every vertex incident to both regions, chosen by topology
alone, and every member must lie on both primitive surfaces: the maximum
joint box/cylinder implicit residual over the whole set is checked against a
tolerance relative to the scale. Reports include deterministic signatures,
crease and corner counts, primitive face counts, and that residual.

Write its unit-scale through-cut mesh for visual inspection:

```sh
cargo run --release -p boolean_oracle -- --cases 1 --points 40 --feature-obj
```

The flag writes only
`target/boolean_oracle/box_cylinder.obj`. The OBJ is stable and
groups faces as `primitive_10` (box) and `primitive_20` (cylinder). It is a
diagnostic artifact, not a checked-in golden file.

Output follows the wind-tunnel `key=value` convention; the determinism
oracle runs before any counting. Typed mesh-pipeline deferrals (coplanar
contact, deferred split configurations) are counted skip categories. Every
successful Boolean stage is deep-validated before it becomes a chained
operand; its ordered face provenance and `kept_faces` count must cover the
emitted mesh exactly; and marked seam components are checked for duplicate
exact `f32` positions under distinct vertex identities. A mesh membership
disagreement, topology/bookkeeping finding, or field disagreement exits
non-zero so a deep sweep can serve as a CI gate.
