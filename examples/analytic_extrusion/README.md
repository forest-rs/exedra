# Analytic extrusion

One plate with a round hole, described by hand as an `exedra::curves::profile::Profile`
and through constructive builders with `Profile2::to_line_arc`. Both routes give
the same profile, so the same exact `Extrusion` and a bit-identical realized
mesh. The demo:

- reads exact native measures without a mesh;
- realizes the solid, with the deviation measured on the emitted coordinates;
- compiles the mesh as baked geometry while keeping the realization report;
- refuses a tolerance the per-arc budget cannot meet, then raises the budget to
  the count the refusal names. The samples then meet the tolerance, but `f32`
  narrowing at this scale does not, and the report says so;
- names the segments a hole touches when it is moved through the outline.

```sh
cargo run -p analytic_extrusion
```

`instrument_panel` builds a front panel from several analytic extrusions. A
bezel's opening is an inset faceplate offset by a 1.5 mm reveal. The faceplate
has a display window recessed into its front as a pocket, a knob bore, vent
slots placed from one tagged slot profile, indicator holes, and four blind bores in its
back. The vent segments carry a tag, and `tessellate_with_regions` groups
every wall with that tag into one region, and the window's recess into
another. The parts compile as baked geometry, and the GLB binds one material
per region: the vents and the recess get their own finish without listing
faces.

The example maps feature tags directly to named `Finish` values and gives every
part a default finish. That same finish identifies the mesh region and resolves
to a material at export, without relying on a material array's position. Native
feature counts use `faces_tagged`; mesh finish counts use `faces_in_region`.

```sh
cargo run -p analytic_extrusion --example instrument_panel -- panel.glb
```

`boundary_geometry` exports three closed models through the construction-independent
body realizer: a periodic cylinder authored directly with two vertices and three
edges, a pocketed plate with the extrusion record discarded, and a box assembled
from six planar faces. All share boundary samples, retain native correspondence,
and check the emitted result against a 0.02 mm tolerance. Each native face maps
once to a material region.

```sh
cargo run -p analytic_extrusion --example boundary_geometry -- boundary.glb
blender --background --python examples/analytic_extrusion/render_boundary.py -- boundary.glb boundary.png
```

The render uses Blender only for lighting and presentation of the exported mesh.

## Turned fitting and matched mesh comparison

```sh
cargo run -p analytic_extrusion --example turned_fitting -- /tmp/turned-fitting
blender --background --python examples/analytic_extrusion/render_boundary.py -- /tmp/turned-fitting.glb /tmp/turned-fitting.png
blender --background --python examples/analytic_extrusion/render_revolve_comparison.py -- /tmp/turned-fitting-comparison.glb /tmp/turned-fitting-comparison-topology.json /tmp/turned-fitting-wire.png --wire
blender --background --python examples/analytic_extrusion/render_revolve_comparison.py -- /tmp/turned-fitting-comparison.glb /tmp/turned-fitting-comparison-topology.json /tmp/turned-fitting-solid.png
```

`turned_fitting` revolves one tagged radius/height profile into a coarse full
turn, a finely sampled cutaway and a fine full turn. The stepped outside and
tapered bore retain planes, cylinders and four conical faces. The cutaway exposes
the bore and planar caps. A shared embedded grid texture uses construction-aware
charts with a 12 mm repeat; the bore and cut caps have their own finish. Every
mesh must be closed and meet its reported tolerance. The SVG shows the generating
profile; console output reports native measures and realization evidence.

The comparison exports that same cutaway through native and existing mesh-based
revolution at 64 angular intervals. Both have 1,105 vertices and 2,206 final render
triangles. The source-edge JSON preserves the native realizer's triangle faces
and the mesh path's 1,088 wall quads plus 30 cap triangles. Wire rendering draws
those source edges rather than exposing GLB triangulation on both sides. Vertex
positions are compared independently of ordering after removing occurrence
translations. Both paths retain a construction recipe; only the native path
also retains independent analytic surfaces and boundaries.

## Filled-region and corner studies

```sh
cargo run -p analytic_extrusion --example region_studies -- /tmp/region-studies
```

Writes `/tmp/region-studies.svg` and `/tmp/region-studies.glb`. The labelled SVG
compares a union cross, concave fillets and chamfers, circular union/intersection/
difference, disconnected material and an island. An enlarged corner detail marks
the tangent joins against the original boundary. The GLB extrudes those same
analytic profiles into ten closed meshes; generated corner transitions share a
feature tag and appear in orange. Console reports give native volume, face counts
and mesh-deviation evidence. Every emitted solid must be closed and meet its
requested chord tolerance.

The example keeps the region operation, validation policy, disconnected-result
handling, selected corners, transition labels and workplane placements explicit.
It introduces no mesh approximation into the line/arc composition itself.

## Profile and path authoring

```sh
cargo run --release -p analytic_extrusion --example authoring -- /tmp/authoring.glb
blender --background --python examples/analytic_extrusion/render_boundary.py -- /tmp/authoring.glb /tmp/authoring.png
```

The sample builds four native solids, including a swept rail. A cursor draws
an arched profile; one tagged circle supplies four placed cutouts; an open
half-outline closes against its mirror line; and a spatial path carries its
endpoint/tangent through two radius/sweep arcs and a relative line. The native
solids use `Workplane::extrude_profile` directly. Bore tags become mesh regions
and bind the dark wall material with the same feature identity.

Every output mesh must be deeply valid and closed; native realization must meet
its reported tolerance. A cubic input also exercises the typed native refusal,
which retains the authored segment index. The rail uses `Sweep::from_profile`
with that same authored path. Its native walls retain planes and cylinders,
and its exact measures precede meshing. An embedded grid uses construction-aware
sweep charts with a 20 mm repeat, making texture stretch through the bends visible.
This sample uses a polygonal section on a smooth open line/arc path; sharp
corners, cubic spans and local bend-axis contact are refused. Curved sections
are shown below.

## Rounded native surfaces

```sh
cargo run --release -p analytic_extrusion --example rounded_geometry -- /tmp/rounded.glb
blender --background --python examples/analytic_extrusion/render_boundary.py -- /tmp/rounded.glb /tmp/rounded.png
```

Four native solids show a rounded turned profile, a torus cutaway, a round
section carried through two bends and a straight run, and a hollow version of
that rail. Rounded shoulders retain toroidal faces; straight round walls retain
cylinders. Shared native boundaries survive the change in supporting surface.
The cut ends expose solid and hollow sections in orange.

An embedded grid uses exact source-curve construction distances with a 20 mm
repeat. Geometry stays shared across texture seams. Every output must be deeply
valid, closed and within its reported 0.04 mm tolerance. Console output includes
exact native measures, face/triangle counts and geometric deviation. Native
sweeping now accepts line/arc profiles; arcs requiring nonregular toroidal walls
and pieces touching a bend axis are source-indexed refusals. Spheres and poles
remain outside this example's supported surface families.

## Circle-to-rectangle lofts

```sh
cargo run --release -p analytic_extrusion --example loft_correspondence -- /tmp/lofts.glb
blender --background --python examples/analytic_extrusion/render_boundary.py -- /tmp/lofts.glb /tmp/lofts.png
```

Three mesh lofts show a ruled circle-to-rectangle transition, a smooth transition
through a displaced rounded rectangle, and a hollow transition with matched
inner boundaries. Explicit landmarks align the circle quarters with corners.
Correspondence preparation retains exact curves and tags; tessellation happens
afterward. The orange caps expose the sections and an embedded 20 mm grid shows
the reference-section construction chart stretching across each transition.

Twisted walls refine longitudinally under a 0.05 mm side-surface policy rather
than emitting one tall quad. Added rows stay smooth; authored ruled section
joins remain sharp. The sample requires deep validity, closure and an emitted
side-surface bound within policy, including f32 displacement. Profile chord
error is controlled separately at 0.05 mm. Console output reports preparation
work, mesh counts, sampling intervals and the side-surface bound.

## Retained ruled lofts and variable sections

```sh
cargo run --release -p analytic_extrusion --example native_loft -- /tmp/native-lofts.glb
blender --background --python examples/analytic_extrusion/render_boundary.py -- /tmp/native-lofts.glb /tmp/native-lofts.png
```

The solid and hollow circle-to-rectangle transitions now retain exact ruled
surfaces. The middle solid changes rounded section size, position and rotation
through four authored stations. Its section joins are intentionally ruled;
the smooth cubic mesh loft remains in the preceding sample. Native faces,
source curves, tags and shared edge identities survive remeshing.

Construction charts use exact reference-section perimeter and explicit datum
distances. The source profiles and correspondence are prepared once, before
native construction. Every output must be deeply valid, closed and within a
combined 0.05 mm geometric deviation bound. Console output includes native face
and edge counts, emitted mesh counts and total deviation. Orange end caps expose
the original sections.

## Selected native axial finishes

```sh
cargo run --release -p analytic_extrusion --example native_edge_finish -- /tmp/native-finishes.glb
blender --background --python examples/analytic_extrusion/render_boundary.py -- /tmp/native-finishes.glb /tmp/native-finishes.png
```

Three blocks show selected planar chamfers, tangent cylindrical fillets and
inner chamfers combined with outer fillets. Orange marks the generated transition
walls. The first two retain their blind slot recesses; the third retains a through
opening. Whole-feature labels select native axial edges and bind materials.
Recesses receive their own label so incidental builder segment labels do not
select them as finishing surfaces.

Each edit returns an independent exact solid and maps transition walls to the
original selected edges. The sample reselects after its second edit and requires
deep validity, closure and a 0.03 mm mesh deviation contract. Full-length profile
seams are the supported family; cap rims and pocket seams require other operations.


## Exact oblique bores

```sh
cargo run --release -p analytic_extrusion --example oblique_bores -- /tmp/oblique-bores.glb
blender --background --python examples/analytic_extrusion/render_oblique_bores.py -- /tmp/oblique-bores.glb /tmp/oblique-bores.png
blender --background --python examples/analytic_extrusion/render_oblique_bores.py -- /tmp/oblique-bores.glb /tmp/oblique-bores-wire.png wire
```

Normal, 30° and 60° cylindrical bores pass completely through rectangular plates.
Orange walls retain cylinders and exact ellipse rims; plate faces retain their
source roles and tags. One authoring operation produces an immutable native
result, rather than a sequence of faceted hole punches. All three display meshes
must be closed, deeply valid and within 0.025 mm. The wire view overlays actual
display triangles. The STEP exchange example independently checks corresponding
native solids, measures, topology, units and material witnesses with OCCT.

The first contract requires a plain unperforated rectangular plate and a bore
whose entire section clears every sidewall by the requested separation. Parallel,
near-parallel, sidewall-contact and unsupported source cases return diagnostics.

### Profile shoulder treatments

```sh
cargo run -p analytic_extrusion --example profile_shoulders -- /tmp/profile-shoulders
```

`profile_shoulders` selects the flange's concave profile corner by position,
then reuses the shared profile operation for a tangent fillet and an equal-distance
chamfer. The three native revolutions compare a sharp shoulder, exact torus and
exact cone. GLB and JSON report the native surfaces and integrals; no 3D edge blend
or mesh Boolean is involved. `Profile::corner_at` refuses missing/ambiguous matches,
and the treatment refuses overlapping trims under the explicit profile policy.

### Native weighted spline outline

```sh
cargo run -p analytic_extrusion --example spline_profile -- /tmp/spline-profile.step /tmp/spline-profile.glb
```

This example uses `PlanarExtrusion::new` instead of assembling boundary tables.
A tagged weighted cubic over domain `[2,5]` and a chord form the outer boundary;
a clockwise rectangle forms a hole. Display realization requests boundary spacing
0.2 and shape ratio 2.5 independently of its 0.01 geometric tolerance. It prints
the observed shape ratio and remaining bad triangles across the whole emitted
mesh, including any patch family that misses the target. Native compilation below
uses its separate tolerance-only policy. It checks native closure, realizes parameter
UVs, exports exact rational STEP geometry, then compiles the same installed native
source at two display tolerances. Construction time and admitted outline work are
printed separately from display and exchange evidence. The optional second path
writes the charted display mesh as GLB with a grid texture, suitable for shaded
or wireframe inspection. The current constructor
accepts full-domain clamped single-span NURBS and unit-range lines; it refuses
uncertain arrangements and unsupported source layouts.
