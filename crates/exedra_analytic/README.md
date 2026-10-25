# `exedra_analytic`

Narrow analytic geometry head for Exedra. It retains exact boundary geometry
until an explicit conversion into `exedra_mesh::Mesh`.

`geometry` provides checked f64 lines, circles, planes and cylinders, with
first and second derivatives in their native parameter units. Circle and cylinder
angles are radians; cylinder height and plane coordinates are model lengths.
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

Both the extrusion constructor and the planar shell builder own this shared
model and expose it through `body()`. A cloned body can outlive its construction
record. `Body::realize` handles planar trims and four-edge cylindrical strips,
including full periodic walls. Native edges are sampled once into flat shared
chains; paired circular edges have synchronized counts. Other cylindrical trims
are typed refusals. Realization checks planar arrangements and local triangle
orientation after f32 emission; global solid validity remains the caller's
responsibility. The report separates chord error, vertex displacement and native
support gaps, and includes all three in its deviation bound. Storage and sampling
budgets are explicit. `realize_with_regions` maps metadata to mesh regions once
per native face while retaining individual native correspondence.

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
