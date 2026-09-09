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

These synthetic dimensions use metres in the recipe. The example preserves
fillet normal overrides through assembly compilation and supplies its own
preview material. UV generation for finished faces is deferred; no textures
are used. Recessed-panel rim fillets are outside this example's supported scope.
