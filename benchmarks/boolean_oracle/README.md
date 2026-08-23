# boolean_oracle

Dual-witness cross-validation oracle: seeded random CSG expression trees are
evaluated by the exedra mesh boolean pipeline and by the `exedra_isosurface`
field combinators, and both are checked point-by-point against a closed-form
convex half-space referee that attributes every disagreement to the
responsible witness. A separate fixed, typed suite exercises opt-in
semi-analytic box/cylinder contouring without erasing its capability behind
`dyn ScalarField`.

Quick profile:

```sh
cargo run --release -p boolean_oracle
```

Deep sweep:

```sh
cargo run --release -p boolean_oracle -- --seed 1 --cases 400 --points 2000
```

Reproduce one reported case without reapplying the batch-seed expansion:

```sh
cargo run --release -p boolean_oracle -- --class chained --case-seed 16616483859594386079 --points 2000
```

The fixed suite runs on every invocation. It covers `Union`, `Intersection`,
and `Difference` at scales `1e-3`, `1`, and `1e4`, plus a rotated pair that
must take counted QEF fallbacks. Translated and `UniformScale`-wrapped
through-cuts verify that those capability-preserving adapters remain inside
the exact envelope. Reports include deterministic triangle-mesh signatures,
deep-topology findings, primitive face counts, feature counts, and maximum
joint box/cylinder implicit residuals. Feature measurement first selects the
topological seam set (vertices incident to both primitive regions), then
measures the best `feature_snaps` members of that fixed set; the residual
tolerance never selects its own samples. All typed fallback counters are
reported and checked against the fixed scenario expectations, and every seam
candidate must be explained by either one snap or one expected ambiguous cell.

Write its unit-scale through-cut mesh for visual inspection:

```sh
cargo run --release -p boolean_oracle -- --cases 1 --points 40 --feature-obj
```

The flag writes only
`target/boolean_oracle/semi_analytic_box_cylinder.obj`. The OBJ is stable and
groups faces as `primitive_10` (box) and `primitive_20` (cylinder). It is a
diagnostic artifact, not a checked-in golden file.

Output follows the wind-tunnel `key=value` convention; the determinism
oracle runs before any counting. Typed mesh-pipeline deferrals (coplanar
contact, deferred split configurations) are counted skip categories. A mesh
or field disagreement exits non-zero so a deep sweep can serve as a CI gate.
