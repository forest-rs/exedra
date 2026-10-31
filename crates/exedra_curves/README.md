# `exedra_curves`

Deterministic curve geometry for the Exedra workspace and its consumers.

Current scope:

- `CubicBasis`: Bezier, uniform B-spline, Catmull-Rom and Hermite as
  explicit basis matrices (the `RenderMan` table that OpenUSD's
  `UsdGeomBasisCurves` documents), evaluated, with first and second
  derivatives, for controls of any dimension, so positions, widths and
  colors follow a span alike;
- spans as four control indices chosen by the caller, `stations` at an
  explicit count per span, and `sample`, `sample_derivative` and
  `sample_linear` at those stations;
- `Nurbs`: rational B-splines of any order in any dimension (space curves by
  default, plane curves for profiles and trims), validated once into a
  `ValidNurbs`, evaluated with derivatives by de Boor's algorithm over
  homogeneous controls, sampled per nonempty knot span of a parameter range
  with the jumps at full-multiplicity knots sampled on both sides and marked
  as breaks, and prepared as per-span Bezier controls (`PreparedNurbs`) for
  repeated evaluation; `Nurbs` borrows a caller's buffers, and `NurbsCurve`
  owns its own, for curves kept as values;
- `rotation_minimizing_frames` and `rotation_minimizing_frames_along`: the
  double-reflection transport `path::double_reflection` that
  the constructive sweeps use too, along estimated or supplied tangents,
  with repeated points, cusps and a closed loop's signed seam twist
  reported;
- `tube` and `ribbon`: triangles at a stated side count with a finite,
  nonnegative radius or width per station, outward-wound, a tube's normals
  following its taper, as plain `f64` buffers and a `SweepReport`;
- arc length by stated Gauss-Legendre quadrature, with the parameter at a
  length, and sampling refined to a chord or angle tolerance under a hard
  cap, each interval's chord and tangent bounds proven with rounding
  allowed for and reported (`PreparedNurbs::adaptive_intervals`);
- `ChordBound`: the fewest uniform steps keeping every chord of a NURBS
  range or a cubic span within a tolerance;
- `intersect`: certified crossings of planar Bezier pieces, polynomial or
  rational, and of a piece with a circle, each proven unique within a
  stated bound or refused, under a caller's step budget;
- `path`: the owned `Path3` and borrowed `CurvePathView` sweep paths of
  lines, arcs and cubics that Exedra's geometry heads share, with their
  closure and join intent;
- `profile`: exact 2D profiles of lines and circular arcs, below;
- `realize`: a `RealizationPlan` built once from an owned
  `CurveDescription` (a cubic chain or a `NurbsCurve`, widths and ribbon
  normals as constant, varying or per-control attributes, closure), keeping
  three layers that change for different reasons: topology (Bezier
  extraction matrices, pieces between breaks, stations), controls (Bezier
  controls, bounds, creases and gaps, station points, analytic tangents,
  rotation-minimizing frames) and attributes. Moving controls redoes only
  the spans they shape and reframes from the first changed station; new
  widths touch nothing else; each update returns a `PlanWork` count of
  what it redid, bit for bit what a fresh plan would hold. A plan realizes
  CPU tubes and ribbons through the sweeps above;
- an `_into` form of every allocating function, filling a caller's buffers
  with a reusable `Workspace` for scratch, so a retained caller allocates
  nothing in steady state.

The crate is format-free: which controls form which span (stepping,
wrapping, end conditions), how a format lays out per-control data, and how
finely to sample are the caller's. The crate documentation states its
contracts: explicit counts, reported (not repaired) degeneracy, finite
nonnegative sweep sizes, discontinuous NURBS sampled on both sides of each
break, and which results are bitwise deterministic across platforms.

```rust
use exedra_curves::{Nurbs, CubicBasis, sample, stations};

// One Bezier span sampled at its ends and middle.
let controls = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
let points = sample(CubicBasis::Bezier, &controls, &[[0, 1, 2, 3]], &stations(1, 2, false)).unwrap();
assert_eq!(points[1], [0.5, 0.75, 0.0]);

// The exact quarter circle.
let quarter = Nurbs {
    order: 3,
    knots: &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
    points: &[[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
    weights: Some(&[1.0, core::f64::consts::FRAC_1_SQRT_2, 1.0]),
};
assert_eq!(quarter.validate().unwrap().sample(None, 8).unwrap().points.len(), 9);
```

`no_std` + `alloc`; depends on `exedra_math`, and on `libm`, through which
profile trigonometry always routes. Features `std` (default) and `libm`
select `exedra_math`'s backend.

## Profiles

A `Profile` is a counter-clockwise outer `Loop` with clockwise holes, in `f64`
model units. Segments are lines or circular arcs between exact endpoints,
with optional caller tags. Arcs carry either a bulge or an authoritative
center and direction. `Profile::new` validates the arrangement against an
explicit `ProfilePolicy`: winding, segments that cross, touch, overlap or come
closer than `min_separation`, and holes outside the outer loop or nested in
one another. The first violation names its loops and segments. Nothing is
moved, merged or healed. Areas and lengths are exact for lines and arcs.

`SegTag` is the shared feature label used by constructive profiles and analytic
face roles. A tag passed to a constructive cutout stays the same typed value
after `to_line_arc` and extrusion; queries can match `tag: Some(VENT)` directly.
Tags do not replace the owning geometry's scoped face and edge ids.

```rust
use exedra_curves::profile::{Loop, Profile, ProfilePolicy, Segment};

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
use exedra_curves::profile::{Loop, Profile, ProfilePolicy, Segment};

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

The module owns deterministic circular sampling: `circular_edge_count`
returns the smallest edge count that keeps every chord within a tolerance of
its arc, under explicit `CircularEdgeConstraints` for minimum topology,
maximum work and an optional count multiple. A work budget never coarsens the result; it is a
typed error instead. Trig always routes through `libm`.

```rust
use exedra_curves::profile::{CircularEdgeConstraints, circular_edge_count};

let constraints = CircularEdgeConstraints::new(8, 4096).with_edge_multiple(4);
let edges = circular_edge_count(25.0, core::f64::consts::TAU, 0.01, constraints)?;
assert_eq!(edges % 4, 0);
# Ok::<(), exedra_curves::profile::CircularSamplingError>(())
```

### Filled-region composition

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

### Corners

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

Licensed under either of Apache License, Version 2.0 or MIT license at your
option.
