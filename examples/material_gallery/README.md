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
cargo run -p material_gallery --bin textured_finishes -- target/textured-finishes
blender --background --python examples/material_gallery/tools/render_textured_finishes.py -- target/textured-finishes
```

The example exports a 450 × 24 × 700 mm slab door with caller-authored UVs,
then a 4 mm chamfer and a 4 mm fillet of the same door. The finish happens inside
the constructive recipe, before assembly compilation and GLB export.

The Blender script renders checkerboard and woodgrain views using only the
exported UVs; it never unwraps or repairs the imported mesh. Textures are supplied
by the script because the Exedra material resolver currently supports untextured
materials. Fillet and chamfer mapping projects the owning source face's chart:
the close views show both texture continuity on that side and stretching toward
the other tangency. Source-chart changes at bands and corner patches are seams.
