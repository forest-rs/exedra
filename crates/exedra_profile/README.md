# `exedra_profile`

Line and circular-arc 2D profile geometry shared by Exedra's geometry heads.

A `Profile` is a counter-clockwise outer `Loop` with clockwise holes, in `f64`
model units. Segments are lines or circular arcs between exact endpoints, with optional
caller tags. Arcs carry either a bulge or an authoritative center and direction. `Profile::new` validates the arrangement against an
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

An arc can retain a construction center instead of deriving one from a bulge:

```rust
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};

let quarter_disk = Profile::new(
    Loop::new(vec![
        Segment::line([0.0, 0.0]),
        Segment::line([10.0, 0.0]),
        Segment::arc_centered([0.0, 10.0], [0.0, 0.0], false),
    ])?,
    vec![],
    &ProfilePolicy::new(1e-9),
)?;
assert!((quarter_disk.area() - 25.0 * core::f64::consts::PI).abs() < 1e-12);
# Ok::<(), Box<dyn core::error::Error>>(())
```

The start of each segment is the preceding endpoint, cyclically. `clockwise`
selects travel direction, including major arcs; a full circle needs two arcs.
`Arc::resolve` derives radius and sweep and returns typed radius-disagreement
or numeric evidence. Its allowance covers local subtraction and norm roundoff,
independent of the profile separation and distance from the origin. Centers and
endpoints are never fitted. Reversal changes travel direction; translation moves
both endpoints and centers. Region splits retain center intent and correspondence.

`IntoProfile` is the checked input seam for geometry heads. Authoring crates
implement it without the shared profile or native head depending on them.
Conversion preserves endpoints and tags and refuses unsupported curves. Owned
shared profiles retain their loop storage; every input honors the supplied
validation policy.

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

## Filled-region composition

`Profile::combine` computes union, intersection or left-minus-right difference
under an explicit `RegionPolicy`. It retains line and circular-arc boundaries;
empty and disconnected results are normal. `into_single` explicitly requires
one connected profile. Each output is validated against the requested profile
policy, including holes and separation. `Profile::validate` can check an existing
profile under a stronger policy without cloning its loops.

Intersections split segments. `RegionResult::segments` records both input
segment references and oriented parameter intervals, including both sources
of a retained coincident run. Segment tags survive; if a coincident run carries
two distinct tags, the left tag takes precedence and both sources remain in
correspondence. Branching point contacts and numerically ambiguous boundaries
are typed refusals. The report exposes pair tests, classification tests,
intersection events, fragment counts and f64 intersection residuals. Those
residuals are distinct from later mesh approximation error.

The operation uses flat cut-event storage and fixed-capacity contact candidates.
Conservative cached bounds reject separated pieces before their analytic
predicates; output rings move into validated profiles without duplicate segment
arrays. Returned geometry and correspondence still own their allocations.

## Corners

`Profile::corners` classifies each boundary vertex as convex, concave or smooth
relative to the filled material, including on holes. Select references from
that immutable profile and pass them to `with_corners` with
`CornerTreatment::fillet(radius)` or `CornerTreatment::chamfer(distance)`.
Treatments accept a feature tag for generated transitions; retained segments
keep their existing tags. `CornerResult::segments` distinguishes trimmed source
intervals from newly generated corner transitions.

Treatments currently require two adjacent straight segments. Other arcs stay
analytic and unchanged; selected line/arc or arc/arc corners are explicitly
refused. Dimensions, references, neighbouring trim overlap and the final profile
arrangement are checked under the caller's policy. An error identifies the
offending corner or limiting edge and its requested/available lengths.

## License

Licensed under either of Apache License 2.0 or MIT license at your option. See
the workspace root for license files.
