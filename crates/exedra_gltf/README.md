# `exedra_gltf`

glTF 2.0 export for Exedra assembly render lists: instances become nodes,
per-region index ranges become primitives with real material bindings, and
instance paths ride in `extras`. Single-file output uses either an embedded
base64 buffer or a standard binary GLB container; both are deterministic
byte-for-byte.

```rust
use exedra_assembly::{Assembly, PartCompiler, flatten};
use exedra_constructive::{ir::Placement3, tessellate::EvalPolicy};
use exedra_gltf::{GlbDocument, export_glb};
use exedra_mesh::{BuildParams, Mesh};

let mesh = Mesh::from_indexed_triangles(
    &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    &[[0, 1, 2]],
    &BuildParams::default(),
)
.expect("valid triangle");
let mut assembly = Assembly::new();
let part = assembly
    .add_baked_part("triangle", mesh, &[])
    .expect("unique part key");
assembly
    .add_instance(None, "placed", part, Placement3::IDENTITY)
    .expect("unique root key");

let compiled = PartCompiler::new()
    .compile_parts(&assembly, &EvalPolicy::default())
    .expect("part compiles");
let list = flatten(&assembly, &compiled);
let export = export_glb(&assembly, &compiled, &list).expect("GLB export");
let document = GlbDocument::parse(&export.bytes).expect("exporter wrote valid GLB");

assert_eq!(document.node_names(), ["placed"]);
assert_eq!(document.triangle_count(), 1);
```

`export_gltf` preserves authored coordinates. Call
`export_gltf_with_options` with `GltfExportOptions::z_up_to_y_up()` when a
Z-up Exedra scene should be presented in glTF's conventional Y-up frame. The
conversion is one right-handed scene-root rotation, so geometry, normals,
winding, and instanced item transforms stay coherent.

Use `export_glb` or `export_glb_with_options` for deployable binary glTF. The
GLB JSON and BIN chunks are padded and length-checked according to glTF 2.0;
the JSON document references the BIN chunk directly rather than embedding a
data URI.

Tests can parse that output with `GlbDocument::parse` and ask semantic
questions through `node_names`, `node_extras`, `material_names`,
`position_bounds`, and `triangle_count`. This avoids copying GLB byte-offset
parsers or asserting against JSON whitespace and key order. Instance metadata
is emitted directly in node `extras`; the exporter-reserved `instancePath`,
`partKey`, and `body` keys remain authoritative on collisions.

## Main APIs

- `export_gltf` and `export_glb` preserve authored coordinates.
- `export_gltf_with_options` and `export_glb_with_options` accept explicit
  coordinate conversion through `GltfExportOptions`.
- `GltfExport` and `GlbExport` return bytes plus `GltfStats` work counters.
- `GlbDocument` is a focused inspection helper for tests, not a general glTF
  loader or validator.

## License

Apache-2.0 OR MIT
