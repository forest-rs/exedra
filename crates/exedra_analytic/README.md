# `exedra_analytic`

Narrow analytic geometry head for Exedra. It retains exact boundary geometry
until an explicit conversion into `exedra_mesh::Mesh`.

`geometry` provides checked f64 lines, circles, planes, cylinders and regular
cones, with first and second derivatives in their native parameter units.
Rotational angles are radians; axial height and plane coordinates are model
lengths. A cone uses `radius + slope * height` and excludes its apex and the
nonpositive-radius nappe.
Supporting geometry is untrimmed. Surface parameters, boundary trims and mesh
texture coordinates are separate concepts.

`body::Body<F, E>` owns f64 vertices, bounded edges, oriented coedges, boundary
loops and faces. Generic metadata holds caller identity or provenance without
being required for evaluation. Local topology IDs are indexes within one body.
A closed circle can have one endpoint vertex; a periodic face can use its seam
edge twice. `Body::try_from_parts` checks references, loop closure and conservative
curve/vertex/support agreement against an explicit gap tolerance. It does not
heal geometry or certify trim nesting, global self-intersection, shell containment
or regular-solid validity. `is_closed` checks oriented edge incidence.

The extrusion and revolution constructors and planar shell builder own this shared
model and expose it through `body()`. A cloned body can outlive its construction
record. `Body::realize` handles planar trims and four-edge cylindrical/conical
strips, including full periodic walls. Coaxial planar annuli and sectors use
the same paired strips to preserve their angular coordinate net. Native edges
are sampled once into flat shared chains; paired circular edges have synchronized
counts. Other curved trims
are typed refusals. Realization checks planar arrangements and local triangle
orientation after f32 emission; global solid validity remains the caller's
responsibility. The report bounds chord error using the actual rounded parameter intervals,
measures emitted paired samples against one fixed axial/radial ruling, and includes that
residual alongside vertex displacement and native support gaps in its deviation
bound. Rectangular-strip eligibility is checked independently of caller gap
slack. Storage and sampling
budgets are explicit. `realize_with_regions` maps metadata to mesh regions once
per native face while retaining individual native correspondence.

`revolve::Revolution` revolves a checked line profile whose XY coordinates are
radius and height. Every radius must be strictly positive. Rotation is about
local +Y: a positive angle moves +X toward -Z, matching constructive revolution.
Sweeps in `(0, TAU]` produce closed native boundaries: full turns share a seam;
partial turns include planar end caps. Profile holes, segment tags, rigid
placement, exact volume and surface area are retained. Axis contact and curved
profile segments are named refusals; spherical/toroidal walls and poles are not
implemented.

`Revolution::realize_with_uvs` supplies each triangle's construction coordinates
to a caller mapper. Walls use `(angle, distance along the profile loop)`; caps
use `(radius, height)`. A full turn shares geometry vertices while retaining
distinct texture coordinates on either side of the seam. The existing constructive
`SurfaceChart::Revolve` maps these coordinates through
`map_revolve_wall_triangle` and `map_cap_triangle`; no geometry crate owns texture
scale or material policy. Mapping errors name the native face, and non-finite
UVs are refused before any UV mutation.

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
booleans, fillets, healing or reverse conversion from meshes.

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
