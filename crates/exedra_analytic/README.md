# `exedra_analytic`

Narrow analytic geometry head for Exedra. It retains exact boundary geometry
until an explicit conversion into `exedra_mesh::Mesh`.

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

The planar shell keeps the earlier scope: `AnalyticShellBuilder` for planar
faces with line-segment coedges and explicit opening loops, `AnalyticShell`
for face-level region and opening edits, and `TessellatedShell` for the mesh
plus its analytic-face provenance.

This crate is deliberately not a full CAD kernel: no surface intersections,
booleans, fillets, healing or reverse conversion from meshes.

The default `std` feature selects native floating-point support. For `no_std`,
disable defaults and enable `libm`. Realization trig always uses `libm`.

## License

Licensed under either of Apache License 2.0 or MIT license at your option. See
the workspace root for license files.
