# `exedra_analytic`

Narrow analytic geometry head for Exedra. It retains exact boundary geometry
until an explicit conversion into `exedra_mesh::Mesh`.

`geometry` provides checked f64 lines, circles, ellipses, planes, cylinders, regular
cones, spheres and ring tori, with first and second derivatives in their native parameter units.
Rotational angles are radians; axial height and plane coordinates are model
lengths. A cone uses `radius + slope * height` and excludes its apex and the
nonpositive-radius nappe.
Both torus parameters are radians; its major radius must exceed its positive
minor radius. Horns and spindles are excluded. Sphere coordinate poles and cone
apices use explicit collapsed boundaries; they have no fictitious 3D edge curve.
`CurveSpan` supplies normalized traversal of a bounded line, circle or ellipse.
`Surface::ruled` interpolates two spans exactly over a unit square and certifies
local regularity using conservative interval bounds. It supplies all first and
second partials; a closest-point inverse is not supplied. Other supporting
geometry is untrimmed within its regular region. Surface parameters, boundary trims and mesh
texture coordinates are separate concepts.

`body::Body<F, E>` owns f64 vertices, bounded edges, oriented coedges, boundary
loops and faces. Generic metadata holds caller identity or provenance without
being required for evaluation. Local topology IDs are indexes within one body.
A closed circle can have one endpoint vertex; a periodic face can use its seam
edge twice. `Body::try_from_parts` checks references, loop closure and conservative
curve/vertex/support agreement against an explicit gap tolerance. It does not
heal geometry or certify trim nesting, global self-intersection, shell containment
or regular-solid validity. `is_closed` checks oriented edge incidence.

`BodyParts::trims` stores optional `LoopTrims` records, with a parameter curve
for each coedge occurrence in a uniquely owned loop. A seam's two occurrences
can have different chart coordinates while retaining the same shared edge.
Each `TrimCurve` maps a normalized canonical edge fraction to face parameters;
coedge reversal affects traversal only. Affine and affine-plus-harmonic curves
represent straight chart boundaries, planar ellipses and oblique cylinder-section
trims. Complete analytic coefficient bounds establish supported 3D/UV agreement,
including arithmetic at the original operand scale. Unsupported compositions are
refused rather than certified from a few samples. Validation reports count
explicitly checked trim occurrences separately from legacy support-only loops.
Domain bounds and chart closure are checked independently of model-space gaps.
`Body::map_roles` consumes and remaps metadata while moving all retained geometry,
topology and trim tables unchanged.

The extrusion, revolution, sweep and loft constructors and planar shell builder
own this shared model and expose it through `body()`. A cloned body can outlive
its construction record. Legacy constructors may omit explicit trim records;
that means support-only validation, never an inferred UV correspondence.
`Body::realize` keeps bounded specialized paths for planar trims, cylindrical/conical
strips, toroidal rectangles, complete ruled patches and supported pole patches.
Monotone collinear line-edge chains on strip boundaries retain every native
split vertex without refining the whole strip; only adjacent panels change.
Other elementary curved faces with complete explicit trims use chart polygon
triangulation and conforming interior-edge refinement. Uniform second-partial
bounds certify the surface interpolation error. Shared boundary chains remain
fixed, including periodic seam copies and hole boundaries. Multi-period charts,
unsupported pole patches and exhausted sampling/storage budgets are typed
refusals. Recognized patches keep their optimized path even when explicit trim
records are present.

Native edges are sampled once into flat shared chains. Paired strip edges and
opposite grid boundaries synchronize their counts. Planar arrangements and local
triangle orientation are checked after f32 emission; global solid validity
remains the caller's responsibility. The report combines curve/surface error,
emitted precision, native geometric gaps and paired-ruling residuals.
`realize_with_regions` maps metadata to mesh regions once per native face while
retaining individual native correspondence.

Toroidal grids synchronize both pairs of opposite boundary chains. Refinement
uses both principal curvatures and a mixed term, rather than accepting the
boundary chord tolerance as a surface guarantee. Interior samples retain native
face parameters; authoritative boundary vertices and seam edges remain shared.
Whole-curve coefficient comparisons check boundary alignment independently of
caller gap slack. Surface error, alignment, parameter arithmetic and emitted
precision contribute to the realization report. Sampling and storage refusals
precede allocation of the output grid; two reusable rows drive triangle emission.

`revolve::Revolution` revolves a checked line/arc profile whose XY coordinates are
radius and height. Every radius must be strictly positive. Rotation is about
local +Y: a positive angle moves +X toward -Z, matching constructive revolution.
Sweeps in `(0, TAU]` produce closed native boundaries: full turns share a seam;
partial turns include planar end caps. Profile holes, segment tags, rigid
placement, exact volume and surface area are retained. Arcs generate regular
toroidal walls; their whole swept radius is checked, including interior extrema.
Axis contact and arcs requiring a nonregular torus are named refusals.
`Revolution::from_profile` accepts authored profiles with separate conversion and
construction errors. Spherical walls and poles are not implemented.

`Revolution::realize_with_uvs` supplies each triangle's construction coordinates
to a caller mapper. Walls use `(angle, distance along the profile loop)`; caps
use `(radius, height)`. A full turn shares geometry vertices while retaining
distinct texture coordinates on either side of the seam. The existing constructive
`SurfaceChart::Revolve` maps these coordinates through
`map_revolve_wall_triangle` and `map_cap_triangle`; no geometry crate owns texture
scale or material policy. Mapping errors name the native face, and non-finite
UVs are refused before any UV mutation.

`sweep::Sweep` transports a checked line/arc section along a smooth open path
of lines and circular arcs. It accepts the same borrowed curve path authored by
constructive `PathCursor3`; shared mathematical descriptors live in
`exedra_math::path`, without coupling either geometry head to the other.
Section boundaries and rails are shared native edges. Planar, cylindrical and
conical/toroidal walls retain path-span and profile-segment identity, feature tags,
exact enclosed volume and surface area, independently of sampling density.
The section datum and initial direction are explicit. Sharp corners, closed
paths, cubic spans, nonregular toroidal walls and local bend-axis contact are
typed refusals. Distant self-intersection remains the caller's responsibility.

`Sweep::realize_with_uvs` supplies `(profile_distance, centerline_distance)` on
walls and source profile coordinates on caps. Distances are measured on the
authored curves. The constructive `SurfaceChart::Sweep` applies texture scale
through `map_sweep_wall_triangle` and `map_cap_triangle`. The geometry seam is
shared while corner UVs remain distinct. Uncharted realization has no UV scratch.

`loft::Loft::from_profiles` accepts corresponding placed line/arc sections via
`IntoProfile`. Each segment can meet a line or an arc; source tags at both ends
remain inspectable. Walls retain exact ruled surfaces, section curves and vertex
rails are shared native edges, and caps retain their source sketch planes.
Profile validation and native table budgets are explicit in `loft::LoftPolicy`.
Different profile structures can be prepared by the authoring head before this
operation. The geometry head does no implicit correspondence or cubic smoothing.

Sections must advance through both neighboring local +Z planes, and every wall
must pass a sufficient regularity check. Failures retain section, band and source
segment context. These checks do not certify global intersections or intermediate
hole containment. Native ruled lofts expose surface evaluation and provenance;
exact loft volume and surface-area queries are not currently provided.

The independent body can be remeshed repeatedly under new tolerances. Ruled grids
bound source-curve chord and mixed triangulation error, share opposite edge
counts, and refine long rulings after the source-curve error is controlled.
The final count is resolved inside a doubling bracket to avoid excess rows.
Per-face source-curve samples are cached across rows; emission uses two reusable
row buffers rather than allocating per vertex or cell.

`Loft::realize_with_uvs` accepts an explicit reference section and an increasing
rest-distance list. It delivers `(exact_reference_profile_distance, station_distance)`
on walls and source `(x,y)` on caps. `SurfaceChart::Loft::map_loft_wall_triangle`
applies the authoring head's texture transform. Caps, profile seams, mapping
refusals and occurrence placement follow the same contracts as other native heads.

`extrusion::Extrusion` extrudes an `exedra_profile::Profile` of lines and
circular arcs along local +Z under a checked rigid frame. The result is a closed
solid with planar caps, planar and cylindrical walls, and shared edges used
once in each direction by oriented coedges. Faces, edges and vertices record
the profile segment, end and tag they come from.

`Extrusion::with_pocket` cuts a `Pocket`, a clockwise outline that stops at a
depth, from either end: a recess, or a blind bore when the outline is round.
`Pocket::from_region` takes the region to remove as a counter-clockwise
outline, like a filled shape. Each pocket adds walls and a planar floor and
becomes a hole of its cap.
Pockets on one end must be valid holes of the profile; pockets on opposite
ends may overlap only where material remains between them.
Depth refusals carry the requested depth, host length, remaining material and
required separation. Opposite-pocket refusals name the other pocket and the
actual web thickness, so callers can diagnose and revise a dimension directly.

`Pocket::tagged` labels a whole pocket. `faces_tagged` selects its floor and
walls, and `edges_tagged` selects its rims and seams, independently of pocket
insertion order. Repeated labels select all matching features. `face_tag` and
`edge_tag` resolve the same labels from the native IDs in mesh correspondence;
material callbacks can group whole features without guessing table indexes.
An unlabelled pocket keeps its individual wall-segment tags. These caller
labels are separate from each solid's local topology IDs.

`profile_axial_edges` selects full-length profile seams, and
`profile_axial_edges_tagged(label)` selects seams between pieces sharing a feature
label. `with_axial_corners` applies shared `CornerTreatment` geometry directly
to those native edge IDs. Chamfers retain planar transition walls; tangent
fillets retain cylinders. The result includes every new profile segment's source
interval or generating corner, plus each transition wall's original edge ID.
Generated walls can receive one explicit feature tag for material selection.

This bounded operation handles line-line corners on outer and hole loops.
Cap rims, pocket seams, smooth seams and arc junctions are typed refusals.
It returns an independent extrusion and preserves occurrence placement, length
and pocket identities. Pockets are revalidated against each complete end once,
with original pocket indices attached to arrangement diagnostics; depth and
opposite-floor web checks share the ordinary pocket-construction logic. Native
tables are built once after validation. No mesh rounding or Boolean is involved.

`Extrusion::tessellate` realizes it under an explicit `TessellationPolicy`:
- Each segment's sample count is chosen once and shared by both caps and its
  wall, so the mesh is closed by construction rather than welding.
- Caps and floors are triangulated in the emitted `f32` coordinates, so boundaries that
  cross, touch or collapse after narrowing are refused or, for arcs, refined
  within budget.
- The result carries mesh-to-native face and vertex correspondence, each
  native edge's vertex chain, and a report of the deviation measured on the
  emitted coordinates against the requested tolerance.

```rust
use exedra_analytic::extrusion::{Extrusion, TessellationPolicy};
use exedra_math::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};

let disk = Loop::new(vec![Segment::arc([-1.0, 0.0], 1.0), Segment::arc([1.0, 0.0], 1.0)])?;
let profile = Profile::new(disk, vec![], &ProfilePolicy::new(1e-9))?;
let solid = Extrusion::new(profile, Placement3::IDENTITY, 2.0)?;
let realized = solid.tessellate(&TessellationPolicy::default())?;
assert!(realized.mesh.boundary_loops()?.is_empty());
assert!(realized.report.within_tolerance());
# Ok::<(), Box<dyn core::error::Error>>(())
```

Native queries do not depend on any realization. `evaluate` gives points and
outward normals on caps and walls. Exact measures are `edge_length`,
`face_area`, `surface_area`, `volume` and `local_bounds`. `attachment_frame`
gives a placed right-handed frame on a planar face, with +Z outward;
cylindrical walls have none.

`Workplane` is a checked rigid sketch plane. Select `cap(End::End)` or a unique
`floor_tagged(label)`, ask the solid for its `workplane`, then translate or
offset the plane and `extrude` a local profile. This creates an independent
solid with explicit placement; it does not perform a Boolean on the host.
Missing or ambiguous floor labels and curved or unknown faces are distinct
typed refusals. `Pocket::from_profile` accepts a whole filled profile and
explicitly refuses holes that would require islands.

The planar shell also uses native f64 geometry: `AnalyticShellBuilder` for planar
faces with line-segment coedges and explicit opening loops, `AnalyticShell`
for face-level region and opening edits, and `TessellatedShell` for the mesh
plus its analytic-face provenance and emitted vertex displacement.
Its builder shares edges by vertex identity and validates on `build()`.

This crate is deliberately not a full CAD kernel: no surface intersections,
booleans, general edge fillets, healing or reverse conversion from meshes.

The default `std` feature selects native floating-point support. For `no_std`,
disable defaults and enable `libm`. Realization trig always uses `libm`.

## License

Licensed under either of Apache License 2.0 or MIT license at your option. See
the workspace root for license files.

## Authored profile inputs

`Extrusion::from_profile(input, frame, length, &policy)` and
`Workplane::extrude_profile(input, length, &policy)` accept any checked
`exedra_profile::IntoProfile` input, including constructive `Profile2` values or
references. Callers no longer need an explicit conversion helper. Profile
conversion errors remain typed inside `ProfileExtrusionError::Profile`; native
construction errors use its `Extrusion` variant. Cubics are refused rather than
flattened. `Extrusion::new` still accepts an already checked shared profile
without repeating validation. The analytic crate does not depend on a profile
authoring frontend.

`Curve::conservative_bounds(range)`, `Surface::conservative_bounds(domain)` and
`Body::conservative_bounds()` enclose native supporting geometry without
meshing. Circular/torus supports use full angular enclosures, and ruled patches
use the convex hull of source-curve enclosures; trims can make these bounds
loose. `Body::conservative_bounds_in(placement)` applies stored affine rows with
outward-rounded products and sums, including large-coordinate cancellation.
The query returns a typed numeric limit instead of non-finite bounds. These
support enclosures are independent of display quality and are not tight extrema
or a solid-interior test.
`geometry::Curve::Ellipse` retains a rigid frame, ordered positive finite
semiaxes and a radian parameter. `geometry::intersection::plane_cylinder`
constructs a complete nonparallel section with explicit ellipse-angle/cylinder-
azimuth phase and orientation, an axial harmonic trim function and whole-curve
support-gap evidence. Near-parallel, non-finite and unmet-gap cases are typed
refusals. These are geometric surface coordinates; texture charts remain separate.
Ellipse edge length is currently unavailable; it is never replaced by a sampled
perimeter. Realization samples exact edges with an affine-circle chord bound.
The section calculation retains accepted near-rigid input bases: it solves the
plane equation in native cylinder parameters and diagonalizes the resulting
world-space harmonics. Parameter-map error uses complete forward coefficients.
Angular maps are bounded to one turn around zero so large floating-point phase
addition cannot silently erase correspondence. Axial harmonic enclosures round
outward and include arithmetic headroom.

`Extrusion::with_through_bore` returns an independent `bore::BoredPlate` for
a plain local rectangular plate and one fully contained nonparallel cylinder.
Inputs are plate-local; placement and the original extrusion stay available.
Native caps acquire ellipse loops, the cylindrical wall retains a repeated
shared seam, and inherited/generated roles identify provenance. Complete rim
clearance proves containment in this convex slab. Existing openings, sidewall
contact, near-parallel axes and unmet numeric agreement are explicit refusals.

`Body::cylindrical_band` derives native angle/height trim functions from complete
curve coefficients and paired coedges, including local rim/seam endpoint
agreement. This view is consumed by shared-edge realization; no extra pcurve
state or texture UVs become authoritative. Native area/volume are independent
of display policy; all curve, pairing, native-gap and f32-displacement evidence
contributes to realization accuracy. Retaining `BoredPlate::source` also retains
the original extrusion and its boundary storage, separately from the result.


A completed display result that narrowly misses its target because of curve
sampling can make one bounded retry. Other error must be below one quarter of
the target; the retry halves only the curve budget and retains the original
caller target and grid/geometry policies. Native face region mappings run once.
Refusal or non-improvement preserves the original honest result. Reports expose
the attempted budget, typed outcome, attempt count and completed-output work;
partial planning work in a refused attempt is excluded from those counters.
Ordinary target-achieving results take their original path. Each attempt obeys
the policy limits; a retry can retain two bounded outputs concurrently.
