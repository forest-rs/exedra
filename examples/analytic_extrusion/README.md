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
