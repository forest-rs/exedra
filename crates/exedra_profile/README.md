# `exedra_profile`

Line and circular-arc 2D profile geometry shared by Exedra's geometry heads.

It owns deterministic circular sampling: `circular_edge_count` returns the
smallest edge count that keeps every chord within a tolerance of its arc,
under explicit `CircularEdgeConstraints` for minimum topology, maximum work and
an optional count multiple. A work budget never coarsens the result; it is a
typed error instead. Trig always routes through `libm`.

```rust
use exedra_profile::{CircularEdgeConstraints, circular_edge_count};

let constraints = CircularEdgeConstraints::new(8, 4096).with_edge_multiple(4);
let edges = circular_edge_count(25.0, core::f64::consts::TAU, 0.01, constraints)?;
assert_eq!(edges % 4, 0);
# Ok::<(), exedra_profile::CircularSamplingError>(())
```

The crate is `no_std` and has no features.

## License

Licensed under either of Apache License 2.0 or MIT license at your option. See
the workspace root for license files.
