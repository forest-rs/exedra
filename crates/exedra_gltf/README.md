# exedra_gltf

glTF 2.0 export for Exedra assembly render lists: instances become nodes,
per-region index ranges become primitives with real material bindings, and
instance paths ride in `extras`. Single-file output uses either an embedded
base64 buffer or a standard binary GLB container; both are deterministic
byte-for-byte.

Instance metadata is preserved under each node's `extras.metadata` object.
Exporter-owned keys such as `instancePath`, `partKey`, and `body` remain at the
top level of `extras`, so application metadata cannot collide with them.

`export_gltf` preserves authored coordinates. Call
`export_gltf_with_options` with `GltfExportOptions::z_up_to_y_up()` when a
Z-up Exedra scene should be presented in glTF's conventional Y-up frame. The
conversion is one right-handed scene-root rotation, so geometry, normals,
winding, and instanced item transforms stay coherent.

Use `export_glb` or `export_glb_with_options` for deployable binary glTF. The
GLB JSON and BIN chunks are padded and length-checked according to glTF 2.0;
the JSON document references the BIN chunk directly rather than embedding a
data URI.
