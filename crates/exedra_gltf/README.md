# `exedra_gltf`

glTF 2.0 export from independent compiled geometry and caller-owned structure.
Supply one-shot `GltfScene` records or implement `GltfSource` in an adapter.
Nodes carry parent/local placements, geometry references, resolved material keys
and arbitrary JSON extras. Geometry-free frames remain nodes; multi-body geometry
gets auxiliary children. Output is deterministic JSON or binary GLB.

```rust
use exedra_compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource};
use exedra_gltf::{GltfScene, GltfNode, GltfMesh, GlbDocument, export_glb};
use exedra_mesh::{BuildParams, Mesh};

let mesh = Mesh::from_indexed_triangles(
    &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    &[[0, 1, 2]], &BuildParams::default(),
).expect("valid triangle");
let source = PartSource::Baked(mesh);
let compiled = PartCompiler::new().compile_parts(
    &[PartInput::new("triangle", &source)], &CompilePolicy::default(),
).expect("compiled geometry");
let mut node = GltfNode::new("placed");
node.geometry = Some(GltfMesh::new(PartId(0), "triangle", &compiled).unwrap().into());
let scene = GltfScene { nodes: vec![node], ..GltfScene::default() };
let export = export_glb(&scene, &compiled).expect("GLB export");
let document = GlbDocument::parse(&export.bytes).expect("valid GLB");
assert_eq!(document.node_names(), ["placed"]);
assert_eq!(document.triangle_count(), 1);
```

The exporter owns format encoding and validation. It has no part registry,
authored hierarchy lifecycle, binding inheritance or scene composition. Parents
can occur anywhere in the input list; invalid parents, cycles, stale geometry,
nonfinite streams and mismatched material/placement arrays are refused before
resource resolution. Material lists follow body and compiled range order; empty
lists mean unassigned. Geometry fingerprints bind references to their compilation.
`GltfPlacementSet` and `GltfLod` express explicit batches and occurrence LOD chains.
See the [complete caller example](../../examples/caller_owned_export/).

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
parsers or asserting against JSON whitespace and key order. Caller metadata
is emitted directly in node `extras`; exporter-reserved keys remain
authoritative on collisions where emitted: `name` in batch identity tables,
`partKey`, `body`, `instances`,
`seed`, `tint`, `setPath`, `placementCount`, `placementIndices`, `lodLevel`,
`MSFT_screencoverage`, and `exedraLodCrossfade`. The Assembly adapter additionally reserves
`instancePath` for its stable paths. Sheared or degenerate
placements are written with their authored `matrix`, which glTF's validator
reports as non-TRS; `GltfStats::non_trs_matrices` counts them.

## Optional Assembly adapter

Enable `exedra_assembly/gltf`, or facade features `assembly,gltf`, to keep existing
`export_glb(&assembly, &compiled)` calls and their material/options variants.
Assembly implements `GltfSource`; it resolves stable paths, material inheritance
and part LODs into export records. The dependency points from optional Assembly
integration to this exporter, never from the exporter to Assembly.

The adapter preserves logical `instancePath` metadata, geometry-free frames,
multi-body children and placement sets. It validates installed source identities
without rescanning geometry. Use `append_selected` to prepare an Assembly subset;
independent callers prepare their own selected nodes and referenced geometry.
Export rejects error-level partial evaluations, retaining the complete
`GeometryReport` in its error. Equal compiled content shares geometry buffers;
different resolved materials can require separate glTF meshes.

## Feature migration

Facade `gltf` now selects `compile,std`; `serde` selects `constructive,std`.
Neither enables Assembly. Add `assembly` explicitly to existing structural
callers. Serialization of recipes and export of caller-owned structure require
no Assembly.

## Main APIs

For authored appearance, use `export_glb_with_materials` or
`export_gltf_with_materials`. Export records carry resolved opaque material IDs. The caller
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
// Resolve the caller's chosen finish into every compiled range.
mesh.materials = compiled.part(part).unwrap().bodies.iter().map(|body| {
    vec![Some("paint.red".into()); body.regions.len()]
}).collect();
node.geometry = Some(mesh.into());
let scene = GltfScene { nodes: vec![node], ..GltfScene::default() };
let export = exedra_gltf::export_glb_with_materials(
    &scene, &compiled, &resolve,
    exedra_gltf::GltfExportOptions::z_up_to_y_up(),
)?;
```

The current export subset supports core material factors, base-color,
metallic-roughness, normal and occlusion textures, alpha, sidedness, and an
allowlist of material extensions (see below). Names and `extras` are
preserved. Missing IDs, invalid fields, and unsupported texture
fields/extensions are typed errors; unassigned regions remain
unassigned. Material-only edits reuse `compiled`, and differing instance
finishes share geometry buffers.

The original functions below use hashed preview colors. See the runnable
[material gallery](../../examples/material_gallery).

- `export_gltf` and `export_glb` preserve authored coordinates.
- `export_gltf_with_options` and `export_glb_with_options` accept explicit
  coordinate conversion and vertex attribute mappings through
  `GltfExportOptions`.
- `GltfExport` and `GlbExport` return bytes plus `GltfStats` work counters.
- `GlbDocument` is a focused inspection helper for tests, not a general glTF
  loader or validator.

## Vertex attributes

Bodies compiled with `CompilePolicy::tangents` export `TANGENT`
automatically. Other extracted streams (`CompilePolicy::attributes`) are
exported only through explicit `GltfAttribute` mappings:

```rust,ignore
use exedra_gltf::{GltfAttribute, GltfExportOptions};
use exedra_mesh::attr;
use exedra_mesh::attributes::{AttrKey, Domain};

const PIVOT: AttrKey<[f32; 4]> = AttrKey::new(Domain::Vertex, "vertex.pivot");

let mappings = [
    GltfAttribute::tex_coord(attr::CORNER_UV1, 1),
    GltfAttribute::color(attr::CORNER_COLOR, 0),
    GltfAttribute::custom(PIVOT, "_WIND_PIVOT"),
];
let options = GltfExportOptions::z_up_to_y_up().with_attributes(&mappings);
```

Mappings name streams by their layer's `AttrKey`, matching domain and name.
The same example is compiled as a doctest on `GltfAttribute`.
Invalid mappings (including indexed sets that are not contiguous:
`TEXCOORD_1..=k`, `COLOR_0..=k`), stream types that do not fit their
semantic, a body missing a lower set below a present higher one, and `u32`
values the chosen `IntegerEncoding` cannot hold exactly are typed errors.
`GltfStats` counts unmapped and missing streams. See
[ADR-0002](docs/adr-0002-vertex-attribute-export.md).

## License

Apache-2.0 OR MIT

## Texture resources

`MaterialResolver::resolve_texture` supplies encoded PNG/JPEG bytes and an
optional core glTF sampler for caller-local texture indices referenced by
`pbrMetallicRoughness.baseColorTexture`,
`pbrMetallicRoughness.metallicRoughnessTexture`, `normalTexture` (with
`scale`) and `occlusionTexture` (with `strength`). The exporter remaps those
indices, embeds only used images, and shares identical image bytes and sampler
objects. Textures of the material extensions below are handled the same way.
Other texture fields (such as `emissiveTexture`) and unlisted extensions remain
explicit errors. The exporter checks image signatures, not full image
decodability; callers are responsible for supplying valid encoded images.

Each image's encoding and channels are fixed by glTF and are the resolver's
contract, since the exporter checks only signatures: base color is sRGB (alpha
in A); metallic-roughness is linear with roughness in G and metalness in B;
occlusion is linear in R, so one ORM image can pack all three; normal maps are
linear tangent-space XYZ with +Y up (the OpenGL convention).

A texture's `texCoord` (default 0) selects its UV set. Set 0 is `TEXCOORD_0`;
set `n >= 1` must be exported through a `TEXCOORD_n` attribute mapping, or the
export fails with `GltfError::MissingTextureCoordinateSet`. That check is for
the exported attribute only: corners a carried stream fills with its missing
value are not checked for authored coverage. Integer fields such as `index`
and `texCoord` must be JSON integers; `1.0` is refused. A normal map on
geometry without tangents is exported and counted in
`GltfStats::normal_maps_without_tangents`: glTF consumers then derive
MikkTSpace tangents themselves, which may not match the baker. Compile with
tangents to pin them.

## GPU instancing

By default every logical instance becomes its own node with a matrix. With
`GltfExportOptions::with_instancing(GltfInstancing::GpuInstancing)`, leaf
instances of one part that share a parent and a material resolution become
one node per body carrying `EXT_mesh_gpu_instancing` `TRANSLATION`,
`ROTATION` and `SCALE` accessors, in instance order. The node's
`extras.instances` table keeps each instance's name and metadata, so identity
stays inspectable, and `GlbDocument::instancing_components` reads the
transforms back. Groups of one, instances with children, and mirrored or
sheared placements keep their own node; the last two are counted in
`GltfStats`. Transforms are stored as `FLOAT` accessors, so translations lose
precision beyond `f32`; overflow or scale underflow retains matrix nodes and
is counted in `unbatched_unrepresentable_instances` or
`unbatched_unrepresentable_placements`. The extension is listed as required whenever it is
used, because no fallback node describes the batched instances.

## Placement sets

Placement sets always export. By default, each placement becomes a node
named `set-path#index` with its matrix, and its seed (a decimal string) and
tint in `extras`. Under `GpuInstancing`, a set becomes one batch in
placement order, identified by `extras.setPath` and
`extras.placementCount`. Seeds are written as `_SEED` (`VEC4` of
`UNSIGNED_SHORT`, four 16-bit words, least significant first) and tints as
`_TINT` (`VEC4` of `FLOAT`). Mirrored or sheared placements keep their own
`set-path#index` node and are listed out of the batch by
`extras.placementIndices`.

## Levels of detail

`GltfExportOptions::with_lods(GltfLods::MsftLod)` writes part level-of-detail
chains as `MSFT_lod`. Each chained occurrence's geometry moves to an
identity child `… [lod 0]`, which lists `… [lod k]` nodes in
`MSFT_lod.ids`. It records each level's minimum screen coverage in
`extras.MSFT_screencoverage` and crossfade bands in
`extras.exedraLodCrossfade`. Instanced chains group their per-body nodes, and
every level reuses the same instance transforms.
`GlbDocument::lod_node_names` reads chains back. The extension is used, not
required, so viewers without it draw level 0. See ADR-0006.

## Material extensions

Material `extensions` may use this allowlist, each validated field by field
against its Khronos specification (ranges, colors, texture references):

| Extension | Fields |
|---|---|
| `KHR_materials_diffuse_transmission` | `diffuseTransmissionFactor`, `diffuseTransmissionTexture` (A), `diffuseTransmissionColorFactor`, `diffuseTransmissionColorTexture` (sRGB) |
| `KHR_materials_transmission` | `transmissionFactor`, `transmissionTexture` (R) |
| `KHR_materials_volume` | `thicknessFactor`, `thicknessTexture` (G), `attenuationDistance`, `attenuationColor` |
| `KHR_materials_ior` | `ior` (`0` or at least `1`) |
| `KHR_materials_specular` | `specularFactor`, `specularTexture` (A), `specularColorFactor`, `specularColorTexture` (sRGB) |
| `KHR_materials_clearcoat` | `clearcoatFactor`, `clearcoatTexture` (R), `clearcoatRoughnessFactor`, `clearcoatRoughnessTexture` (G), `clearcoatNormalTexture` |
| `KHR_materials_sheen` | `sheenColorFactor`, `sheenColorTexture` (sRGB), `sheenRoughnessFactor`, `sheenRoughnessTexture` (A) |
| `KHR_materials_emissive_strength` | `emissiveStrength` |

A volume without a transmission extension on the same material has no effect
in glTF and is refused. Any texture reference, core or extension, may carry
`extensions.KHR_texture_transform` (`offset`, `rotation`, `scale`,
`texCoord`); its `texCoord` overrides the reference's own for viewers that
support the extension. Both sets are checked per primitive, because a viewer
without the extension samples the reference's own set. Used extensions are listed in `extensionsUsed`; none
are required, except `KHR_texture_transform` when
`GltfExportOptions::require_texture_transform` is set. Exedra has no material
model of its own: projecting OpenPBR or another model onto these extensions is
the caller's conversion.

Existing untextured resolver closures need no changes. Implement the optional
method on a resolver type to supply textures; resource indices belong to that
resolver, never to the assembly or a previous export. Regions whose material
samples `TEXCOORD_0` require finite UVs at every emitted triangle corner. A default zero supplied
by render extraction does not count as an authored coordinate.

For example, a resolver can expose one caller-owned image:

```rust
use exedra_gltf::{MaterialResolver, Texture};
use serde_json::{Value, json};

struct Finish<'a> {
    base_color_png: &'a [u8],
}
impl MaterialResolver for Finish<'_> {
    fn resolve(&self, key: &str) -> Option<Value> {
        (key == "wood").then(|| json!({
            "pbrMetallicRoughness": {
                "baseColorTexture": {"index": 0},
                "metallicFactor": 0.0
            }
        }))
    }
    fn resolve_texture(&self, index: u32) -> Option<Texture<'_>> {
        (index == 0).then(|| Texture {
            image: self.base_color_png,
            mime_type: "image/png",
            sampler: Some(json!({"wrapS": 10497, "wrapT": 10497})),
        })
    }
}
```

`GltfStats` now includes `images`, `textures`, and `image_bytes`. For hand-built
stats values, initialize those counters or use `..GltfStats::default()`.
