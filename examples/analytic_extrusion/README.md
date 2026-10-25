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
slots drawn with `LoopCursor`, indicator holes, and four blind bores in its
back. The vent segments carry a tag, and `tessellate_with_regions` groups
every wall with that tag into one region, and the window's recess into
another. The parts compile as baked geometry, and the GLB binds one material
per region: the vents and the recess get their own finish without listing
faces.

```sh
cargo run -p analytic_extrusion --example instrument_panel -- panel.glb
```
