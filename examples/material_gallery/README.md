# Material gallery

One compiled cylinder appears with four finishes: red paint, polished
gold, rough metal, and translucent blue plastic. The GLB contains real PBR
factors and one shared geometry payload. Colors are authored in linear space.
The plastic uses alpha coverage; this example does not model refraction.

```sh
cargo run -p material_gallery -- target/material-gallery
```

- `materials.glb`: four finishes on four instances of one part.
- `edited.glb`: the red paint becomes glossy green; the other finishes retain
  their original appearance.
- `rebound.glb`: the second instance also uses the green paint.

Every export reports geometry bytes and checks that material edits and instance
rebinding caused zero new part compilations or emitted triangles. Geometry
buffers remain shared even when the glTF material bindings need separate meshes.

The resolver closure adapts a caller-owned table. Replace that table with the
application's material lookup and conversion to glTF JSON. Missing bound keys
fail export; unassigned regions retain glTF's unassigned material behavior.

This example needs no textures or UV generation. It demonstrates the material
resource/export slice; Boolean cut-slot attribution and texture mapping have
separate contracts in the design-notes handoff.

## Geometry edge finishes

```sh
cargo run -p material_gallery --bin edge_finishes -- target/edge-finishes
```

- `rail-fillet.glb`: 90 × 200 × 2000 mm box with a true 6 mm convex 3D fillet.
- `rail-profile.glb`: the same section dimensions with 6 mm rounded profile
  corners, extruded with sharp end perimeters.
- `foot-chamfer.glb`: 90 × 200 × 150 mm foot with a 6 mm bevel on the boundary
  between its +X and +Y face regions.
- `door-recess.glb`: 450 × 24 × 700 mm door with a Boolean-cut 330 × 580 mm
  recess, 12 mm deep.
- `door-recess-chamfer.glb` and `door-recess-fillet.glb`: the same door with
  a 3 mm finish around the recess, joined with miters at its four corners.
- `two-pockets.glb`, `two-pockets-chamfer.glb`, and `two-pockets-fillet.glb`:
  a 400 × 200 × 100 mm block with two 50 mm diameter, 20 mm deep pockets.
  Both independently selected rims receive a 1 mm finish.

These synthetic dimensions use metres in the recipe. The example preserves
fillet normal overrides through assembly compilation and supplies its own
preview material. These inputs have no UVs and use no textures. The recess
example uses ordinary box primitives for both panel and cutter. It selects the
panel-front/cutter-wall boundaries through `OperandBoundaries`, qualifying each
region by its CSG operand instead of renumbering imported geometry. Square turns
explicitly opt into
`max_tangent_turn = FRAC_PI_2`; adjoining fillets retain a miter crease.

Render the exported recess geometry, materials and normals in Blender:

```sh
blender --background --python examples/material_gallery/tools/render_recessed_door.py -- target/edge-finishes
```

The script writes `recess-comparison.png` (sharp, chamfer, fillet from left to
right), `recess-chamfer-close.png`, and `recess-fillet-close.png`.

## Textured edge finishes

```sh
cargo run -p material_gallery --bin textured_finishes -- target/textured_finishes
blender --background --python examples/material_gallery/tools/render_textured_finishes.py -- target/textured_finishes
```

The example imports a 450 × 24 × 700 mm slab door, authors box UVs through
`NodeKind::UvMap` at one repeat per 80 mm, then applies a 4 mm chamfer or fillet.
Mapping and finishing happen inside the constructive recipe, before assembly
compilation and GLB export. A loader-provided mesh follows the same path.

The Blender script renders checkerboard and woodgrain views using only the
exported UVs; it never unwraps or repairs the imported mesh. Textures in this older example are supplied
by the script. The construction-chart example below embeds its texture through
the material resolver. Fillet and chamfer mapping projects the owning source face's chart:
the close views show both texture continuity on that side and stretching toward
the other tangency. Source-chart changes at bands and corner patches are seams.


## UV projection gallery

```sh
python3 examples/material_gallery/tools/make_projection_grid.py
cargo run -p material_gallery --bin uv_projections -- target/uv_projections
blender --background --python examples/material_gallery/tools/render_uv_projections.py -- target/uv_projections
```

Eight imported meshes demonstrate planar fitting, oriented box faces, per-face
planes, cylindrical, spherical, toroidal and perspective projection, plus a
pivoted affine transform copied into the secondary UV channel. All mappings
are construction nodes. The GLB embeds its labeled diagnostic texture; the
Blender script displays the exported coordinates without repairing or unwrapping
them. glTF axis conversion keeps each study on its own labeled pedestal.

Open `target/uv_projections/gallery.png` for the complete comparison,
`sphere-pole.png` and `torus-seams.png` for close views, or
`uv-projections.glb` in a glTF viewer. `gallery.json` records per-study mapping
counts and placement. Tests check pole/seam evidence, complete finite coverage,
secondary coordinates in the exported GLB, and geometry reuse after material edits.

## Construction surface charts

```sh
cargo run -p material_gallery --bin surface_charts -- target/surface-charts.glb
```

A half-round vault and two turned vessels use an embedded procedural grid.
Dark grid lines are one metre apart in the authored rest coordinates; pale
intermediate lines divide a repeat in half. Extrusion measures sampled perimeter
distance and depth. The vessels use a one-metre reference radius for angular
distance, so the grid deliberately stretches with the actual radius. One vessel
is a reflected occurrence of the same compiled part.

A smooth draped strip uses its first section as the transverse reference and
an authored six-metre longitudinal rest length, distributed as two metres per
section band. A mirrored strip shares its compiled geometry. The spatial cubic
rail uses sampled centerline distance, deliberately stretching on inner and
outer curves. The grid remains continuous through their sampled stations.
No box projection is used.

The example checks complete UV coverage and that a material-color edit reuses
compiled geometry. The integration test checks texture/normal accessors and the
embedded image in the exported GLB. See the constructive crate's
[chart contract](../../crates/exedra_constructive/docs/adr-0023-construction-surface-charts.md).
