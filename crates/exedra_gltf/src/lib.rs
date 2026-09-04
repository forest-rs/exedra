// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! glTF 2.0 export for Exedra assembly render lists.
//!
//! [`export_gltf`] and [`export_glb`] convert a flattened [`RenderList`] plus
//! its [`CompiledParts`] into a single-file glTF 2.0 asset. JSON glTF embeds a
//! base64 buffer; GLB stores the same buffer in its binary chunk.
//!
//! - every render item becomes a glTF *node* carrying the item's world
//!   matrix, with the instance path, part key, and opaque instance metadata
//!   in `extras`;
//! - every per-region index range becomes a mesh *primitive*, so region
//!   materials survive as real material bindings;
//! - material keys become named PBR material stubs with a deterministic
//!   base color derived from the key;
//! - items that share a part, body, and material resolution share one
//!   glTF mesh (glTF-level instancing).
//!
//! The output is deterministic: identical inputs produce byte-identical
//! JSON. No external glTF or base64 dependency is used.
//!
//! # Example
//!
//! Export a placed baked mesh, then inspect the GLB semantically:
//!
//! ```
//! use exedra_assembly::{Assembly, PartCompiler, flatten};
//! use exedra_constructive::{ir::Placement3, tessellate::EvalPolicy};
//! use exedra_gltf::{GlbDocument, export_glb};
//! use exedra_mesh::{BuildParams, Mesh};
//!
//! let mesh = Mesh::from_indexed_triangles(
//!     &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
//!     &[[0, 1, 2]],
//!     &BuildParams::default(),
//! )?;
//! let mut assembly = Assembly::new();
//! let part = assembly.add_baked_part("triangle", mesh, &[])?;
//! assembly.add_instance(None, "placed", part, Placement3::IDENTITY)?;
//!
//! let compiled = PartCompiler::new().compile_parts(&assembly, &EvalPolicy::default())?;
//! let list = flatten(&assembly, &compiled);
//! let export = export_glb(&assembly, &compiled, &list)?;
//! let document = GlbDocument::parse(&export.bytes)?;
//!
//! assert_eq!(document.node_names(), ["placed"]);
//! assert_eq!(document.triangle_count(), 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod inspect;

pub use inspect::GlbDocument;

use std::collections::HashMap;
use std::fmt::Write as _;

use exedra_assembly::{Assembly, CompiledParts, RenderItem, RenderList};
use serde_json::{Map, Value, json};

/// A finished export.
#[derive(Clone, Debug)]
pub struct GltfExport {
    /// The glTF 2.0 document as pretty JSON.
    pub json: String,
    /// Introspection counters for the export.
    pub stats: GltfStats,
}

/// A finished binary glTF export.
#[derive(Clone, Debug)]
pub struct GlbExport {
    /// Complete GLB 2.0 bytes.
    pub bytes: Vec<u8>,
    /// Introspection counters for the export.
    pub stats: GltfStats,
}

/// Deterministic export counters.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct GltfStats {
    /// glTF nodes emitted (one per render item, plus an optional coordinate
    /// conversion root).
    pub nodes: u64,
    /// Distinct glTF meshes emitted (shared across matching items).
    pub meshes: u64,
    /// Primitives emitted across all meshes.
    pub primitives: u64,
    /// Distinct materials emitted.
    pub materials: u64,
    /// Total bytes in the embedded buffer.
    pub buffer_bytes: u64,
}

/// Coordinate-system handling for a glTF export.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum GltfCoordinates {
    /// Preserve Exedra coordinates exactly as authored.
    #[default]
    Preserve,
    /// Convert Exedra's Z-up coordinates to glTF's conventional Y-up frame.
    ///
    /// The conversion is the right-handed rotation `(x, y, z) ->
    /// (x, z, -y)`.
    ZUpToYUp,
}

/// Options controlling glTF export.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct GltfExportOptions {
    /// Coordinate-system handling for the exported scene.
    pub coordinates: GltfCoordinates,
}

impl GltfExportOptions {
    /// Options that present an Exedra Z-up scene in glTF's conventional
    /// Y-up frame.
    #[must_use]
    pub const fn z_up_to_y_up() -> Self {
        Self {
            coordinates: GltfCoordinates::ZUpToYUp,
        }
    }
}

/// Typed export failure.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GltfError {
    /// A render item references a part with no compiled entry.
    MissingPart {
        /// The part index the item referenced.
        part: u32,
    },
    /// A render item references a body index out of range.
    MissingBody {
        /// The part index.
        part: u32,
        /// The body index the item referenced.
        body: u32,
    },
    /// The finished GLB would exceed its unsigned 32-bit container length.
    GlbTooLarge,
    /// A GLB container header or chunk layout is invalid.
    InvalidGlb {
        /// Stable explanation of the rejected container invariant.
        reason: &'static str,
    },
    /// The GLB JSON chunk is not valid JSON.
    InvalidGlbJson {
        /// Parser detail suitable for diagnostics and test failures.
        message: String,
    },
}

impl std::fmt::Display for GltfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPart { part } => write!(f, "no compiled entry for part {part}"),
            Self::MissingBody { part, body } => {
                write!(f, "part {part} has no compiled body {body}")
            }
            Self::GlbTooLarge => f.write_str("GLB output exceeds the 32-bit container limit"),
            Self::InvalidGlb { reason } => write!(f, "invalid GLB container: {reason}"),
            Self::InvalidGlbJson { message } => write!(f, "invalid GLB JSON chunk: {message}"),
        }
    }
}

impl std::error::Error for GltfError {}

/// Exports a render list as a single-file glTF 2.0 JSON document.
///
/// `assembly` supplies part keys for `extras`; `compiled` supplies the
/// geometry the list references.
///
/// # Errors
///
/// Fails when the list references parts or bodies absent from `compiled`.
pub fn export_gltf(
    assembly: &Assembly,
    compiled: &CompiledParts,
    list: &RenderList,
) -> Result<GltfExport, GltfError> {
    export_gltf_with_options(assembly, compiled, list, GltfExportOptions::default())
}

/// Exports a render list with explicit coordinate-system options.
///
/// The Z-up to Y-up conversion is represented by one scene-root node. Mesh
/// buffers, accessor bounds, normals, winding, and item transforms remain in
/// their authored local coordinates and are transformed coherently by the
/// glTF node hierarchy.
///
/// # Errors
///
/// Fails when the list references parts or bodies absent from `compiled`.
pub fn export_gltf_with_options(
    assembly: &Assembly,
    compiled: &CompiledParts,
    list: &RenderList,
    options: GltfExportOptions,
) -> Result<GltfExport, GltfError> {
    let built = build_export(assembly, compiled, list, options)?;
    let mut document = built.document;
    document.insert(
        "buffers".into(),
        json!([{
            "byteLength": built.buffer.len(),
            "uri": format!(
                "data:application/octet-stream;base64,{}",
                base64(&built.buffer)
            ),
        }]),
    );
    let json = serde_json::to_string_pretty(&Value::Object(document))
        .expect("document is finite JSON by construction");
    Ok(GltfExport {
        json,
        stats: built.stats,
    })
}

/// Exports a render list as a binary glTF 2.0 container.
///
/// # Errors
///
/// Fails when the list references absent compiled geometry or when the
/// finished GLB exceeds its 32-bit container length.
pub fn export_glb(
    assembly: &Assembly,
    compiled: &CompiledParts,
    list: &RenderList,
) -> Result<GlbExport, GltfError> {
    export_glb_with_options(assembly, compiled, list, GltfExportOptions::default())
}

/// Exports a render list as GLB with explicit coordinate-system options.
///
/// # Errors
///
/// Fails when the list references absent compiled geometry or when the
/// finished GLB exceeds its 32-bit container length.
pub fn export_glb_with_options(
    assembly: &Assembly,
    compiled: &CompiledParts,
    list: &RenderList,
    options: GltfExportOptions,
) -> Result<GlbExport, GltfError> {
    let built = build_export(assembly, compiled, list, options)?;
    let stats = built.stats;
    let bytes = pack_glb(built.document, built.buffer)?;
    Ok(GlbExport { bytes, stats })
}

#[derive(Debug)]
struct BuiltExport {
    document: Map<String, Value>,
    buffer: Vec<u8>,
    stats: GltfStats,
}

fn build_export(
    assembly: &Assembly,
    compiled: &CompiledParts,
    list: &RenderList,
    options: GltfExportOptions,
) -> Result<BuiltExport, GltfError> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut buffer_views: Vec<Value> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();
    let mut meshes: Vec<Value> = Vec::new();
    let mut materials: Vec<Value> = Vec::new();
    let mut material_index: HashMap<String, usize> = HashMap::new();
    // Mesh sharing: items with the same part, body, and material
    // resolution reference one glTF mesh.
    let mut mesh_index: HashMap<(u32, u32, Vec<Option<String>>), usize> = HashMap::new();
    let mut nodes: Vec<Value> = Vec::new();
    let mut stats = GltfStats::default();

    for item in &list.items {
        let entry = compiled
            .part(item.part)
            .ok_or(GltfError::MissingPart { part: item.part.0 })?;
        let body = entry
            .bodies
            .get(item.body as usize)
            .ok_or(GltfError::MissingBody {
                part: item.part.0,
                body: item.body,
            })?;

        let resolution: Vec<Option<String>> =
            item.regions.iter().map(|r| r.material.clone()).collect();
        let key = (item.part.0, item.body, resolution);
        let mesh = if let Some(&index) = mesh_index.get(&key) {
            index
        } else {
            let index = emit_mesh(
                body,
                item,
                &mut buffer,
                &mut buffer_views,
                &mut accessors,
                &mut materials,
                &mut material_index,
                &mut stats,
            );
            meshes.push(index);
            let mesh_number = meshes.len() - 1;
            mesh_index.insert(key, mesh_number);
            stats.meshes += 1;
            mesh_number
        };

        let mut node = Map::new();
        node.insert("name".into(), Value::String(item.path.to_string()));
        node.insert("mesh".into(), json!(mesh));
        if item.world.rows != exedra_constructive::ir::Placement3::IDENTITY.rows {
            node.insert(
                "matrix".into(),
                json!(matrix_column_major(&item.world.rows)),
            );
        }
        let part_key = assembly
            .part(item.part)
            .map(|def| def.key().to_string())
            .unwrap_or_default();
        let mut extras = Map::new();
        if let Some(instance) = assembly.instance(item.instance) {
            for (key, value) in instance.metadata() {
                extras.insert(key.clone(), Value::String(value.clone()));
            }
        }
        // Export identity is authoritative when opaque metadata reuses one
        // of these reserved keys.
        extras.insert("instancePath".into(), Value::String(item.path.to_string()));
        extras.insert("partKey".into(), Value::String(part_key));
        extras.insert("body".into(), json!(item.body));
        node.insert("extras".into(), Value::Object(extras));
        nodes.push(Value::Object(node));
        stats.nodes += 1;
    }

    stats.buffer_bytes = buffer.len() as u64;
    let item_nodes: Vec<usize> = (0..nodes.len()).collect();
    let scene_nodes = match options.coordinates {
        GltfCoordinates::Preserve => item_nodes,
        GltfCoordinates::ZUpToYUp => {
            let root = nodes.len();
            nodes.push(json!({
                "name": "Exedra Z-up to glTF Y-up",
                "matrix": [
                    1.0, 0.0, 0.0, 0.0,
                    0.0, 0.0, -1.0, 0.0,
                    0.0, 1.0, 0.0, 0.0,
                    0.0, 0.0, 0.0, 1.0
                ],
                "children": item_nodes,
            }));
            stats.nodes += 1;
            vec![root]
        }
    };
    let mut document = Map::new();
    document.insert(
        "asset".into(),
        json!({ "version": "2.0", "generator": "exedra_gltf" }),
    );
    document.insert("scene".into(), json!(0));
    document.insert("scenes".into(), json!([{ "nodes": scene_nodes }]));
    document.insert("nodes".into(), Value::Array(nodes));
    document.insert("meshes".into(), Value::Array(meshes));
    if !materials.is_empty() {
        document.insert("materials".into(), Value::Array(materials));
    }
    document.insert("accessors".into(), Value::Array(accessors));
    document.insert("bufferViews".into(), Value::Array(buffer_views));
    Ok(BuiltExport {
        document,
        buffer,
        stats,
    })
}

fn pack_glb(mut document: Map<String, Value>, mut buffer: Vec<u8>) -> Result<Vec<u8>, GltfError> {
    document.insert("buffers".into(), json!([{ "byteLength": buffer.len() }]));
    let mut json = serde_json::to_vec(&Value::Object(document))
        .expect("document is finite JSON by construction");
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !buffer.len().is_multiple_of(4) {
        buffer.push(0);
    }

    let total_len = 12_usize
        .checked_add(8)
        .and_then(|length| length.checked_add(json.len()))
        .and_then(|length| length.checked_add(8))
        .and_then(|length| length.checked_add(buffer.len()))
        .ok_or(GltfError::GlbTooLarge)?;
    let total_len = u32::try_from(total_len).map_err(|_| GltfError::GlbTooLarge)?;
    let json_len = u32::try_from(json.len()).map_err(|_| GltfError::GlbTooLarge)?;
    let buffer_len = u32::try_from(buffer.len()).map_err(|_| GltfError::GlbTooLarge)?;

    let mut glb = Vec::with_capacity(total_len as usize);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&total_len.to_le_bytes());
    glb.extend_from_slice(&json_len.to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&buffer_len.to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&buffer);
    Ok(glb)
}

/// Emits one glTF mesh (buffer data, views, accessors, primitives) for a
/// compiled body under a given material resolution. Returns the mesh JSON.
#[expect(
    clippy::too_many_arguments,
    reason = "internal helper threading fixed export context"
)]
fn emit_mesh(
    body: &exedra_assembly::CompiledBody,
    item: &RenderItem,
    buffer: &mut Vec<u8>,
    buffer_views: &mut Vec<Value>,
    accessors: &mut Vec<Value>,
    materials: &mut Vec<Value>,
    material_index: &mut HashMap<String, usize>,
    stats: &mut GltfStats,
) -> Value {
    let tri = &body.tri;
    let vertex_count = tri.positions.len();

    let positions_view = push_view(buffer, buffer_views, &vec3_bytes(&tri.positions));
    let (min, max) = position_bounds(&tri.positions);
    let positions_accessor = accessors.len();
    accessors.push(json!({
        "bufferView": positions_view,
        "componentType": 5126,
        "count": vertex_count,
        "type": "VEC3",
        "min": min,
        "max": max,
    }));

    let normals_view = push_view(buffer, buffer_views, &vec3_bytes(&tri.normals));
    let normals_accessor = accessors.len();
    accessors.push(json!({
        "bufferView": normals_view,
        "componentType": 5126,
        "count": vertex_count,
        "type": "VEC3",
    }));

    let uvs_view = push_view(buffer, buffer_views, &vec2_bytes(&tri.uvs));
    let uvs_accessor = accessors.len();
    accessors.push(json!({
        "bufferView": uvs_view,
        "componentType": 5126,
        "count": vertex_count,
        "type": "VEC2",
    }));

    let mut index_bytes = Vec::with_capacity(tri.indices.len() * 4);
    for &i in &tri.indices {
        index_bytes.extend_from_slice(&i.to_le_bytes());
    }
    let indices_view = push_view(buffer, buffer_views, &index_bytes);

    let mut primitives: Vec<Value> = Vec::new();
    for region in &item.regions {
        let indices_accessor = accessors.len();
        accessors.push(json!({
            "bufferView": indices_view,
            "byteOffset": region.start as usize * 4,
            "componentType": 5125,
            "count": region.count,
            "type": "SCALAR",
        }));
        let mut primitive = Map::new();
        primitive.insert(
            "attributes".into(),
            json!({
                "POSITION": positions_accessor,
                "NORMAL": normals_accessor,
                "TEXCOORD_0": uvs_accessor,
            }),
        );
        primitive.insert("indices".into(), json!(indices_accessor));
        primitive.insert("extras".into(), json!({ "faceRegion": region.region }));
        if let Some(material) = &region.material {
            let index = *material_index.entry(material.clone()).or_insert_with(|| {
                materials.push(material_stub(material));
                stats.materials += 1;
                materials.len() - 1
            });
            primitive.insert("material".into(), json!(index));
        }
        primitives.push(Value::Object(primitive));
        stats.primitives += 1;
    }

    json!({ "primitives": primitives })
}

/// A named PBR stub whose base color derives deterministically from the
/// material key.
fn material_stub(key: &str) -> Value {
    let h = fnv64(key.as_bytes());
    let channel = |shift: u32| {
        let byte = u8::try_from((h >> shift) & 0xFF).expect("masked to one byte");
        f64::from(byte) / 255.0 * 0.7 + 0.2
    };
    json!({
        "name": key,
        "pbrMetallicRoughness": {
            "baseColorFactor": [channel(0), channel(8), channel(16), 1.0],
            "metallicFactor": 0.0,
            "roughnessFactor": 0.8,
        },
    })
}

fn push_view(buffer: &mut Vec<u8>, views: &mut Vec<Value>, bytes: &[u8]) -> usize {
    let offset = buffer.len();
    buffer.extend_from_slice(bytes);
    views.push(json!({
        "buffer": 0,
        "byteOffset": offset,
        "byteLength": bytes.len(),
    }));
    views.len() - 1
}

fn vec3_bytes(values: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 12);
    for v in values {
        for c in v {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out
}

fn vec2_bytes(values: &[[f32; 2]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 8);
    for v in values {
        for c in v {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out
}

fn position_bounds(positions: &[[f32; 3]]) -> (Vec<f32>, Vec<f32>) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    if positions.is_empty() {
        (vec![0.0; 3], vec![0.0; 3])
    } else {
        (min.to_vec(), max.to_vec())
    }
}

/// 4x4 column-major matrix from 3x4 row-major placement rows.
fn matrix_column_major(rows: &[[f64; 4]; 3]) -> Vec<f64> {
    let mut out = Vec::with_capacity(16);
    for col in 0..4 {
        for row in rows {
            out.push(row[col]);
        }
        out.push(if col == 3 { 1.0 } else { 0.0 });
    }
    out
}

/// Standard-alphabet base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let _ = write!(
            out,
            "{}{}",
            ALPHABET[(n >> 18) as usize & 63] as char,
            ALPHABET[(n >> 12) as usize & 63] as char
        );
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_assembly::{PartCompiler, flatten};
    use exedra_constructive::builders;
    use exedra_constructive::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};
    use exedra_constructive::tessellate::EvalPolicy;

    fn example() -> (Assembly, CompiledParts, RenderList) {
        let mut b = RecipeBuilder::new();
        let front = b.material_slot("front");
        let _ = front;
        let profile = b.add_profile(builders::rect(40.0, 20.0).unwrap());
        let node = b
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 10.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let recipe = b.finish(node).unwrap();

        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", recipe).unwrap();
        asm.set_part_material(part, "front", "oak").unwrap();
        let a = asm
            .add_instance(None, "a", part, Placement3::IDENTITY)
            .unwrap();
        asm.set_metadata(a, "review.state", "ready").unwrap();
        asm.set_metadata(a, "instancePath", "opaque collision")
            .unwrap();
        let b2 = asm
            .add_instance(None, "b", part, Placement3::translate(60.0, 0.0, 0.0))
            .unwrap();
        asm.bind_material(b2, "front", "walnut").unwrap();

        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        let list = flatten(&asm, &compiled);
        (asm, compiled, list)
    }

    #[test]
    fn structural_validity() {
        let (asm, compiled, list) = example();
        let export = export_gltf(&asm, &compiled, &list).unwrap();
        let doc: Value = serde_json::from_str(&export.json).unwrap();

        assert_eq!(doc["asset"]["version"], "2.0");
        assert_eq!(doc["scene"], 0);
        let nodes = doc["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2, "one node per render item");
        assert_eq!(nodes[0]["extras"]["instancePath"], "a");
        assert_eq!(nodes[1]["extras"]["partKey"], "panel");
        assert!(
            nodes[1]["matrix"].as_array().is_some(),
            "translated node carries a matrix"
        );

        // Different material resolutions force distinct meshes here.
        let meshes = doc["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), 2);
        assert_eq!(export.stats.meshes, 2);

        // Buffer byteLength matches the embedded payload exactly.
        let byte_length = doc["buffers"][0]["byteLength"].as_u64().unwrap();
        let uri = doc["buffers"][0]["uri"].as_str().unwrap();
        let payload = uri.split(',').nth(1).unwrap();
        assert_eq!(payload.len() % 4, 0);
        let padding = payload.bytes().rev().take_while(|b| *b == b'=').count();
        assert_eq!(
            usize::try_from(byte_length).unwrap(),
            payload.len() / 4 * 3 - padding,
            "buffer length must match the base64 payload"
        );

        // Every accessor's bufferView window stays inside the buffer, and
        // every index accessor holds whole triangles.
        let views = doc["bufferViews"].as_array().unwrap();
        for view in views {
            let offset = view["byteOffset"].as_u64().unwrap();
            let length = view["byteLength"].as_u64().unwrap();
            assert!(offset + length <= byte_length);
        }
        for accessor in doc["accessors"].as_array().unwrap() {
            if accessor["type"] == "SCALAR" {
                assert_eq!(accessor["count"].as_u64().unwrap() % 3, 0);
            }
            if accessor["type"] == "VEC3" && accessor.get("min").is_some() {
                assert_eq!(accessor["min"].as_array().unwrap().len(), 3);
                assert_eq!(accessor["max"].as_array().unwrap().len(), 3);
            }
        }

        // Materials: oak and walnut, named, deterministic colors.
        let materials = doc["materials"].as_array().unwrap();
        let names: Vec<&str> = materials
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["oak", "walnut"]);
    }

    #[test]
    fn shared_resolution_shares_meshes() {
        let (asm, compiled, mut list) = example();
        // Make item b's materials identical to a's: same mesh key.
        for region in &mut list.items[1].regions {
            region.material = Some("oak".into());
        }
        let export = export_gltf(&asm, &compiled, &list).unwrap();
        assert_eq!(export.stats.nodes, 2);
        assert_eq!(export.stats.meshes, 1, "identical items share one mesh");
    }

    #[test]
    fn z_up_to_y_up_uses_one_right_handed_scene_root() {
        let (asm, compiled, list) = example();
        let export =
            export_gltf_with_options(&asm, &compiled, &list, GltfExportOptions::z_up_to_y_up())
                .unwrap();
        let doc: Value = serde_json::from_str(&export.json).unwrap();
        let nodes = doc["nodes"].as_array().unwrap();
        let root_index = usize::try_from(doc["scenes"][0]["nodes"][0].as_u64().unwrap()).unwrap();
        let root = &nodes[root_index];

        assert_eq!(root["children"], json!([0, 1]));
        assert_eq!(
            root["matrix"],
            json!([
                1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0
            ])
        );
        assert_eq!(export.stats.nodes, 3);
        assert_eq!(nodes[0]["extras"]["instancePath"], "a");
    }

    #[test]
    fn deterministic_output() {
        let (asm, compiled, list) = example();
        let a = export_gltf(&asm, &compiled, &list).unwrap();
        let b = export_gltf(&asm, &compiled, &list).unwrap();
        assert_eq!(a.json, b.json);
        assert_eq!(a.stats, b.stats);
    }

    #[test]
    fn binary_export_has_valid_glb_chunks() {
        let (asm, compiled, list) = example();
        let export =
            export_glb_with_options(&asm, &compiled, &list, GltfExportOptions::z_up_to_y_up())
                .unwrap();
        let bytes = &export.bytes;

        assert_eq!(&bytes[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        assert_eq!(
            usize::try_from(u32::from_le_bytes(bytes[8..12].try_into().unwrap())).unwrap(),
            bytes.len()
        );
        let json_len =
            usize::try_from(u32::from_le_bytes(bytes[12..16].try_into().unwrap())).unwrap();
        assert_eq!(&bytes[16..20], b"JSON");
        let json_end = 20 + json_len;
        let document: Value = serde_json::from_slice(&bytes[20..json_end]).unwrap();
        assert_eq!(document["asset"]["version"], "2.0");
        assert!(document["buffers"][0].get("uri").is_none());
        assert_eq!(&bytes[json_end + 4..json_end + 8], b"BIN\0");
        let bin_len = usize::try_from(u32::from_le_bytes(
            bytes[json_end..json_end + 4].try_into().unwrap(),
        ))
        .unwrap();
        assert_eq!(json_end + 8 + bin_len, bytes.len());
        assert!(bin_len >= usize::try_from(export.stats.buffer_bytes).unwrap());
        assert!(bin_len - usize::try_from(export.stats.buffer_bytes).unwrap() < 4);
    }

    #[test]
    fn binary_output_is_deterministic() {
        let (asm, compiled, list) = example();
        let a = export_glb(&asm, &compiled, &list).unwrap();
        let b = export_glb(&asm, &compiled, &list).unwrap();
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(a.stats, b.stats);
    }

    #[test]
    fn binary_export_has_semantic_inspection_queries() {
        // Integration tests should ask about exporter semantics instead of
        // slicing GLB offsets or depending on serialized JSON whitespace and
        // key order. The coordinate-conversion root leaves mesh accessors in
        // the authored local frame by design.
        let (asm, compiled, list) = example();
        let export =
            export_glb_with_options(&asm, &compiled, &list, GltfExportOptions::z_up_to_y_up())
                .unwrap();
        let document = GlbDocument::parse(&export.bytes).expect("exporter emits valid GLB");

        assert_eq!(
            document.node_names(),
            vec!["a", "b", "Exedra Z-up to glTF Y-up"]
        );
        let extras = document.node_extras("a").expect("named node extras");
        assert_eq!(extras["instancePath"], "a");
        assert_eq!(extras["partKey"], "panel");
        assert_eq!(extras["review.state"], "ready");
        assert_eq!(document.material_names(), vec!["oak", "walnut"]);
        assert_eq!(
            document.position_bounds(),
            Some(([0.0, 0.0, 0.0], [40.0, 20.0, 10.0]))
        );
        assert_eq!(document.triangle_count(), 24);
        assert_eq!(document.json()["asset"]["version"], "2.0");
        assert!(!document.bin().is_empty());
    }

    #[test]
    fn base64_reference_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64(b"abcdef"), "YWJjZGVm");
    }

    #[test]
    fn matrix_layout_is_column_major() {
        let p = Placement3::translate(1.0, 2.0, 3.0);
        let m = matrix_column_major(&p.rows);
        assert_eq!(&m[12..16], &[1.0, 2.0, 3.0, 1.0]);
        assert_eq!(m[0], 1.0);
        assert_eq!(m[15], 1.0);
    }
}
