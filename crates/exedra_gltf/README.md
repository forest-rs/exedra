# `exedra_gltf`

glTF 2.0 export for Exedra assembly render lists: instances become nodes,
region/slot index ranges become primitives with real material bindings, and
instance paths ride in `extras`. Single-file output uses either an embedded
base64 buffer or a standard binary GLB container; both are deterministic
byte-for-byte.

```rust
use exedra_assembly::{Assembly, CompilePolicy, PartCompiler, flatten};
use exedra_constructive::ir::Placement3;
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
    .compile_parts(&assembly, &CompilePolicy::default())
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

For authored appearance, use `export_glb_with_materials` or
`export_gltf_with_materials`. Assembly carries opaque material IDs. The caller
maps each ID to glTF JSON through `MaterialResolver` (including a closure), using
whatever material model it owns:

```rust,ignore
let resolve = |id: &str| match id {
    "paint.red" => Some(serde_json::json!({
        "pbrMetallicRoughness": {
            "baseColorFactor": [0.65, 0.025, 0.015, 1.0], // Linear RGBA.
            "metallicFactor": 0.0,
            "roughnessFactor": 0.6
        }
    })),
    _ => None,
};
assembly.set_part_material(part, "finish", "paint.red")?;
let list = exedra_assembly::flatten(&assembly, &compiled);
let export = exedra_gltf::export_glb_with_materials(
    &assembly, &compiled, &list, &resolve,
    exedra_gltf::GltfExportOptions::z_up_to_y_up(),
)?;
```

The current export subset supports core untextured material factors, alpha, and
sidedness. Names and `extras` are preserved. Missing IDs, invalid fields, and
unsupported textures/extensions are typed errors; unassigned regions remain
unassigned. Material-only edits reuse `compiled`, and differing instance
finishes share geometry buffers.

The original functions below use hashed preview colors. See the runnable
[material gallery](../../examples/material_gallery).

- `export_gltf` and `export_glb` preserve authored coordinates.
- `export_gltf_with_options` and `export_glb_with_options` accept explicit
  coordinate conversion through `GltfExportOptions`.
- `GltfExport` and `GlbExport` return bytes plus `GltfStats` work counters.
- `GlbDocument` is a focused inspection helper for tests, not a general glTF
  loader or validator.

## License

Apache-2.0 OR MIT
