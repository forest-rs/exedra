# Analytic extrusion

One plate with a round hole, described by hand as an `exedra::profile::Profile`
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

The sample builds three native solids and one constructive sweep. A cursor draws
an arched profile; one tagged circle supplies four placed cutouts; an open
half-outline closes against its mirror line; and a spatial path carries its
endpoint/tangent through two radius/sweep arcs and a relative line. The native
solids use `Workplane::extrude_profile` directly. Bore tags become mesh regions
and bind the dark wall material with the same feature identity.

Every output mesh must be deeply valid and closed; native realization must meet
its reported tolerance. A cubic input also exercises the typed native refusal,
which retains the authored segment index. Sweeps still use the ordinary recipe
and evaluation policies; radius/tangent authoring does not conceal sharp-corner
foldovers or promise new geometric capabilities.
