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
