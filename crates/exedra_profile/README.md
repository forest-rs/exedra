# `exedra_profile`

Line and circular-arc 2D profile geometry shared by Exedra's geometry heads.

A `Profile` is a counter-clockwise outer `Loop` with clockwise holes, in `f64`
model units. Segments are lines or bulge arcs between exact endpoints, with
optional caller tags. `Profile::new` validates the arrangement against an
explicit `ProfilePolicy`: winding, segments that cross, touch, overlap or come
closer than `min_separation`, and holes outside the outer loop or nested in
one another. The first violation names its loops and segments. Nothing is
moved, merged or healed. Areas and lengths are exact for lines and arcs.

`SegTag` is the shared feature label used by constructive profiles and analytic
face roles. A tag passed to a constructive cutout stays the same typed value
after `to_line_arc` and extrusion; queries can match `tag: Some(VENT)` directly.
Tags do not replace the owning geometry's scoped face and edge ids.

```rust
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};

let outer = Loop::new(vec![
    Segment::line([4.0, 0.0]),
    Segment::line([4.0, 2.0]),
    Segment::line([0.0, 2.0]),
    Segment::line([0.0, 0.0]),
])?;
let hole = Loop::new(vec![
    Segment::arc([1.5, 1.0], -1.0),
    Segment::arc([2.5, 1.0], -1.0),
])?;
let profile = Profile::new(outer, vec![hole], &ProfilePolicy::new(1e-9))?;
assert!((profile.area() - (8.0 - core::f64::consts::PI / 4.0)).abs() < 1e-12);
# Ok::<(), Box<dyn core::error::Error>>(())
```

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
