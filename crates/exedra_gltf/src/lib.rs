// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! glTF 2.0 export from compiled geometry and caller-owned structure.
//!
//! Supply a one-shot [`GltfScene`] or implement [`GltfSource`] in an adapter.
//! Nodes carry local transforms, resolved geometry/material references and extras.
//! Geometry stays in independent [`CompiledParts`]; export never constructs or
//! evaluates it. This crate owns format encoding and resource validation, not
//! authored hierarchy, binding inheritance, scene composition or persistent identity.
//!
//! Equal geometry shares buffers across material resolutions. Multi-body inputs
//! receive auxiliary geometry nodes. Explicit placement sets and repeated leaves
//! can use `EXT_mesh_gpu_instancing`; explicit LOD chains can use `MSFT_lod`.
//! Mirrored and sheared placements retain individual nodes. Strict materials,
//! textures and mapped attributes use the same rules for every source adapter.
//! Invalid hierarchy, content correspondence and nonfinite input are refused.
//! Output is deterministic; JSON embeds base64 buffers and GLB embeds a BIN chunk.
//!
//! # Example
//!
//! Export geometry without an Assembly or native stage:
//!
//! ```
//! use exedra_compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource};
//! use exedra_gltf::{GltfScene, GltfNode, GltfMesh, GlbDocument, export_glb};
//! use exedra_mesh::{BuildParams, Mesh};
//! let mesh = Mesh::from_indexed_triangles(
//!     &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
//!     &[[0, 1, 2]], &BuildParams::default(),
//! )?;
//! let source = PartSource::Baked(mesh);
//! let compiled = PartCompiler::new().compile_parts(
//!     &[PartInput::new("triangle", &source)], &CompilePolicy::default(),
//! )?;
//! let mut node = GltfNode::new("placed");
//! node.geometry = Some(GltfMesh::new(PartId(0), "triangle", &compiled)?.into());
//! let scene = GltfScene { nodes: vec![node], ..GltfScene::default() };
//! let export = export_glb(&scene, &compiled)?;
//! let document = GlbDocument::parse(&export.bytes)?;
//! assert_eq!(document.node_names(), ["placed"]);
//! assert_eq!(document.triangle_count(), 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod attributes;
mod input;
mod inspect;
mod instancing;
mod materials;
mod textures;

use exedra_compile::{CompilationMismatch, CompiledBody, CompiledParts, PartFingerprint, PartId};
use exedra_constructive::{
    evaluate::{GeometryReport, Severity},
    ir::Placement3,
};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, fmt::Write as _};

use attributes::Site;
pub use attributes::{GltfAttribute, GltfSemantic, IntegerEncoding};
pub use input::{
    GltfExtras, GltfGeometry, GltfLod, GltfMesh, GltfNode, GltfPlacementSet, GltfScene, GltfSource,
    JsonValue,
};
pub use inspect::GlbDocument;
pub use materials::{MaterialResolver, Texture};

#[cfg(test)]
mod encoding_tests;
#[cfg(test)]
mod input_tests;

#[derive(Clone, Debug)]
struct ResolvedRegion {
    region: u32,
    start: u32,
    count: u32,
    material: Option<String>,
}

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
    /// Every node emitted: logical instance nodes, placement nodes,
    /// instanced batch nodes, auxiliary multi-body nodes, `MSFT_lod` level
    /// groups and lower-level nodes, and the optional coordinate-conversion
    /// root.
    pub nodes: u64,
    /// Distinct glTF meshes emitted (shared across matching items).
    pub meshes: u64,
    /// Primitives emitted across all meshes.
    pub primitives: u64,
    /// Distinct materials emitted.
    pub materials: u64,
    /// Distinct encoded images embedded, after byte-content sharing.
    pub images: u64,
    /// Distinct image/sampler combinations emitted.
    pub textures: u64,
    /// Encoded image bytes, excluding alignment padding (part of `buffer_bytes`).
    pub image_bytes: u64,
    /// Total bytes in the embedded buffer.
    pub buffer_bytes: u64,
    /// Accessors emitted for `TANGENT` and mapped attribute streams.
    pub attribute_accessors: u64,
    /// Bytes of `TANGENT` and mapped attribute data (part of `buffer_bytes`).
    pub attribute_bytes: u64,
    /// Extracted streams, per exported geometry, that no
    /// [`GltfExportOptions::attributes`] mapping names. They are not exported.
    pub unmapped_attribute_streams: u64,
    /// Mappings, per exported geometry, whose stream that geometry does not
    /// carry. The attribute is omitted from its primitives.
    pub missing_attribute_streams: u64,
    /// Primitives whose material has a `normalTexture` or a clearcoat normal
    /// texture but whose geometry carries no `TANGENT`. Exported as is: glTF
    /// consumers then derive MikkTSpace tangents themselves, which may not
    /// match the baker.
    pub normal_maps_without_tangents: u64,
    /// Nodes written with `EXT_mesh_gpu_instancing`: one per body of each
    /// batch (see [`GltfInstancing::GpuInstancing`]).
    pub instanced_nodes: u64,
    /// Logical instances folded into instanced nodes instead of getting a
    /// node of their own.
    pub batched_instances: u64,
    /// Instances kept as their own node because their placement is a
    /// reflection, counted only when another instance with the same parent,
    /// part and material resolution means they would otherwise have batched.
    pub unbatched_mirrored_instances: u64,
    /// Instances kept as their own node because their placement is sheared or
    /// degenerate and has no TRS decomposition, counted like
    /// [`Self::unbatched_mirrored_instances`].
    pub unbatched_sheared_instances: u64,
    /// Candidate instances retained as matrices because FLOAT accessors cannot represent their transforms.
    pub unbatched_unrepresentable_instances: u64,
    /// Placement-set placements folded into instanced nodes.
    pub batched_placements: u64,
    /// Placement-set placements written as their own node: every placement
    /// under [`GltfInstancing::Nodes`], and mirrored or sheared placements
    /// under [`GltfInstancing::GpuInstancing`].
    pub placement_nodes: u64,
    /// Placements kept as their own node under
    /// [`GltfInstancing::GpuInstancing`] because they are reflections.
    pub unbatched_mirrored_placements: u64,
    /// Placements kept as their own node under
    /// [`GltfInstancing::GpuInstancing`] because they are sheared or
    /// degenerate.
    pub unbatched_sheared_placements: u64,
    /// Set placements retained as matrices because FLOAT accessors cannot represent their transforms.
    pub unbatched_unrepresentable_placements: u64,
    /// Lower-level nodes written for `MSFT_lod` chains (see
    /// [`GltfLods::MsftLod`]). They are reachable only through `MSFT_lod`.
    pub lod_nodes: u64,
    /// Instance and placement nodes written with a sheared or degenerate
    /// `matrix`. glTF requires node matrices to decompose into TRS, so the
    /// Khronos validator reports these (`NODE_MATRIX_NON_TRS`); Exedra
    /// writes the placement as authored rather than approximating it.
    pub non_trs_matrices: u64,
}

/// How part level-of-detail chains are written.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum GltfLods {
    /// Only level 0, the placed part itself. Lower levels are not written.
    #[default]
    BaseLevel,
    /// Every level, through `MSFT_lod`. The geometry of an occurrence whose
    /// part has a chain of at least two levels moves to a child node named
    /// `… [lod 0]` with an identity transform. That node lists the lower
    /// levels in `extensions.MSFT_lod.ids`, and their `min_coverage` values,
    /// one per level, in `extras.MSFT_screencoverage` (crossfade bands in
    /// `extras.exedraLodCrossfade`). Lower-level nodes, named `… [lod k]`,
    /// also have identity transforms and are referenced only by those ids.
    /// A viewer that replaces nodes switches levels; one that swaps only the
    /// extension node's own mesh finds none on multi-body or instanced
    /// chains and keeps drawing level 0, a graceful degradation rather than
    /// a level switch.
    ///
    /// Instanced occurrences group their per-body nodes under a node that
    /// carries `MSFT_lod` and keeps the batch name (not `… [lod 0]`); its
    /// level-0 bodies are `… [lod 0] [body i]` and a lower level `k` is a
    /// group `… [lod k]` with bodies `… [lod k] [body i]`, all reusing the
    /// same instance transforms. The extension is listed as used, not
    /// required: viewers without it draw level 0.
    MsftLod,
}

/// How repeated placements of a part are written.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum GltfInstancing {
    /// One node per logical instance, each with its own matrix, and one
    /// node per placement-set placement, named and addressed
    /// `set-path#index`, carrying its seed (as a decimal string, exact for
    /// all `u64`) and tint in `extras`.
    #[default]
    Nodes,
    /// Leaf instances of one part, with the same material resolution and the
    /// same parent, become one node per body carrying
    /// `EXT_mesh_gpu_instancing` `TRANSLATION`, `ROTATION` and `SCALE`
    /// accessors, in instance order. Their identities move to the node's
    /// `extras.instances` table. Groups of one keep their node, as do
    /// instances with children or placement sets and mirrored or sheared
    /// placements (counted in [`GltfStats`]).
    ///
    /// Every placement set becomes one such batch, in placement order, even
    /// with a single placement. Its identity is `extras.setPath` plus
    /// `extras.placementCount`, and `extras.placementIndices` when mirrored
    /// or sheared placements were left out; those keep their own
    /// `set-path#index` node as under [`Self::Nodes`]. Set seeds and tints
    /// become the custom instance attributes `_SEED` (`VEC4` of
    /// `UNSIGNED_SHORT`, the seed's four 16-bit words, least significant
    /// first, so `u64` seeds stay exact) and `_TINT` (`VEC4` of `FLOAT`,
    /// linear RGBA).
    ///
    /// A batched part gets one node per non-empty body; unlike unbatched
    /// multi-body instances, empty bodies get no node. The extension is then
    /// listed as required: nothing else describes the batched instances.
    GpuInstancing,
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
///
/// Options borrow their attribute mappings, so one value can be reused
/// across exports.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct GltfExportOptions<'a> {
    /// Coordinate-system handling for the exported scene.
    pub coordinates: GltfCoordinates,
    /// Extracted vertex streams to export as primitive attributes, in this
    /// accessor order.
    ///
    /// `POSITION`, `NORMAL`, and `TEXCOORD_0` are always written, and
    /// `TANGENT` is written for every body with
    /// [`TriMesh::tangents`](exedra_mesh::TriMesh::tangents). Streams without
    /// a mapping are not exported; see
    /// [`GltfStats::unmapped_attribute_streams`].
    pub attributes: &'a [GltfAttribute],
    /// List `KHR_texture_transform` in `extensionsRequired`, not only in
    /// `extensionsUsed`, when a material uses it.
    ///
    /// Off by default: a viewer without the extension still loads the file
    /// and samples untransformed coordinates. Require it when a wrong tiling
    /// is worse than refusing to load.
    pub require_texture_transform: bool,
    /// How repeated placements are written; see [`GltfInstancing`].
    pub instancing: GltfInstancing,
    /// How part level-of-detail chains are written; see [`GltfLods`].
    pub lods: GltfLods,
}

impl<'a> GltfExportOptions<'a> {
    /// Options that present an Exedra Z-up scene in glTF's conventional
    /// Y-up frame.
    #[must_use]
    pub const fn z_up_to_y_up() -> Self {
        Self {
            coordinates: GltfCoordinates::ZUpToYUp,
            attributes: &[],
            require_texture_transform: false,
            instancing: GltfInstancing::Nodes,
            lods: GltfLods::BaseLevel,
        }
    }

    /// Replaces [`Self::attributes`] with `mappings`.
    #[must_use]
    pub const fn with_attributes(mut self, mappings: &'a [GltfAttribute]) -> Self {
        self.attributes = mappings;
        self
    }

    /// Sets [`Self::require_texture_transform`].
    #[must_use]
    pub const fn with_required_texture_transform(mut self, required: bool) -> Self {
        self.require_texture_transform = required;
        self
    }

    /// Sets [`Self::instancing`].
    #[must_use]
    pub const fn with_instancing(mut self, instancing: GltfInstancing) -> Self {
        self.instancing = instancing;
        self
    }

    /// Sets [`Self::lods`].
    #[must_use]
    pub const fn with_lods(mut self, lods: GltfLods) -> Self {
        self.lods = lods;
        self
    }
}

/// Typed export failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GltfError {
    /// A referenced caller-local texture index has no resource.
    MissingTexture {
        /// Index supplied by the material resolver's texture reference.
        index: u32,
    },
    /// A texture resource has an invalid encoding signature or sampler field.
    InvalidTexture {
        /// Caller-local texture index.
        index: u32,
        /// Rejected resource field.
        field: &'static str,
    },
    /// A texture samples `TEXCOORD_0`, but its region contains missing or
    /// non-finite authored UVs.
    MissingTextureCoordinates {
        /// Bound material key.
        material: String,
        /// Compilation-local part.
        part: u32,
        /// Body within the part.
        body: usize,
        /// Geometric region within the body.
        region: u32,
    },
    /// A material's texture samples `TEXCOORD_n` (`n >= 1`), but the
    /// primitive's geometry exports no such attribute.
    MissingTextureCoordinateSet {
        /// Bound material key.
        material: String,
        /// The texture reference, for example `"normalTexture"`.
        texture: &'static str,
        /// The UV set the texture samples.
        set: u32,
        /// Compilation-local part.
        part: u32,
        /// Body within the part.
        body: usize,
        /// Geometric region within the body.
        region: u32,
    },
    /// A bound material key has no description in the supplied resolver.
    MissingMaterial {
        /// The unresolved caller material key.
        key: String,
    },
    /// A resolved material has an invalid object, field type, or factor value.
    InvalidMaterial {
        /// The caller material key.
        key: String,
        /// The invalid glTF material field.
        field: String,
    },
    /// A material requests a field outside the supported export subset.
    UnsupportedMaterialField {
        /// The caller material key.
        key: String,
        /// The unsupported glTF material field.
        field: String,
    },
    /// The supplied compilation does not match the export records' captured geometry.
    CompilationMismatch(CompilationMismatch),
    /// An export record violates hierarchy, geometry or attribute correspondence.
    InvalidInput {
        /// Precise input field or geometry handle.
        path: String,
        /// Checked invariant that failed.
        reason: &'static str,
    },
    /// A compiled part retains error-level diagnostics from a partial evaluation.
    IncompleteGeometry {
        /// Compilation-local part with the error-level report.
        part: u32,
        /// Complete evaluation evidence, retained independently of the snapshot.
        report: Box<GeometryReport>,
    },
    /// An attribute mapping is not valid glTF: `TEXCOORD_0` (the primary UV
    /// set), a custom name without a leading `_`, a repeated semantic, or an
    /// indexed set whose lower sets are not mapped (`TEXCOORD` sets must be
    /// `1..=k`, `COLOR` sets `0..=k`).
    InvalidAttributeMapping {
        /// The mapped stream.
        stream: &'static str,
        /// The rejected semantic.
        semantic: String,
        /// Why the mapping was rejected.
        reason: &'static str,
    },
    /// A body lacks the stream of an indexed set while a higher set of the
    /// same semantic is present, so exporting it would leave a gap glTF
    /// forbids. A missing highest set is skipped and counted instead.
    AttributeSetGap {
        /// The missing stream.
        stream: &'static str,
        /// The semantic it maps to.
        semantic: String,
        /// Compilation-local part.
        part: u32,
        /// Body within the part.
        body: usize,
    },
    /// A mapped stream's value type does not fit its semantic, for example a
    /// `u32` stream mapped to `COLOR_0`.
    AttributeTypeMismatch {
        /// The mapped stream.
        stream: &'static str,
        /// The target semantic.
        semantic: String,
        /// Compilation-local part.
        part: u32,
        /// Body within the part.
        body: usize,
    },
    /// A `u32` stream holds a value its [`IntegerEncoding`] cannot represent
    /// exactly.
    UnrepresentableAttribute {
        /// The mapped stream.
        stream: &'static str,
        /// The target semantic.
        semantic: String,
        /// Compilation-local part.
        part: u32,
        /// Body within the part.
        body: usize,
        /// The first value out of range.
        value: u32,
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
            Self::MissingTexture { index } => write!(f, "no texture resource for index {index}"),
            Self::InvalidTexture { index, field } => {
                write!(f, "texture {index} has invalid field {field:?}")
            }
            Self::MissingTextureCoordinates {
                material,
                part,
                body,
                region,
            } => write!(
                f,
                "material {material:?} requires authored UVs in part {part}, body {body}, region {region}"
            ),
            Self::MissingTextureCoordinateSet {
                material,
                texture,
                set,
                part,
                body,
                region,
            } => write!(
                f,
                "material {material:?} {texture} samples TEXCOORD_{set}, which part {part}, body {body}, region {region} does not export"
            ),
            Self::MissingMaterial { key } => write!(f, "no material description for {key:?}"),
            Self::InvalidMaterial { key, field } => {
                write!(f, "material {key:?} has invalid field {field:?}")
            }
            Self::UnsupportedMaterialField { key, field } => {
                write!(f, "material {key:?} requests unsupported field {field:?}")
            }
            Self::CompilationMismatch(error) => error.fmt(f),
            Self::InvalidInput { path, reason } => {
                write!(f, "invalid export input {path}: {reason}")
            }
            Self::IncompleteGeometry { part, .. } => write!(
                f,
                "compiled part {part} has error-level geometry diagnostics"
            ),
            Self::InvalidAttributeMapping {
                stream,
                semantic,
                reason,
            } => write!(f, "stream {stream:?} cannot map to {semantic}: {reason}"),
            Self::AttributeSetGap {
                stream,
                semantic,
                part,
                body,
            } => write!(
                f,
                "part {part}, body {body} lacks stream {stream:?} for {semantic} below a higher set"
            ),
            Self::AttributeTypeMismatch {
                stream,
                semantic,
                part,
                body,
            } => write!(
                f,
                "stream {stream:?} in part {part}, body {body} has the wrong type for {semantic}"
            ),
            Self::UnrepresentableAttribute {
                stream,
                semantic,
                part,
                body,
                value,
            } => write!(
                f,
                "stream {stream:?} in part {part}, body {body} holds {value}, which {semantic} cannot represent exactly"
            ),
            Self::GlbTooLarge => f.write_str("GLB output exceeds the 32-bit container limit"),
            Self::InvalidGlb { reason } => write!(f, "invalid GLB container: {reason}"),
            Self::InvalidGlbJson { message } => write!(f, "invalid GLB JSON chunk: {message}"),
        }
    }
}

impl std::error::Error for GltfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CompilationMismatch(error) => Some(error),
            _ => None,
        }
    }
}

/// Exports the caller-prepared scene as a single-file glTF 2.0 JSON document.
///
/// Parent/local placements and geometry-free frames are retained. Each logical
/// node carries caller metadata and its display name; auxiliary body nodes
/// carry only part/body identity. Geometry comes from the matching compilation.
/// Material keys use deterministic preview colors. For authored appearance,
/// use [`export_gltf_with_materials`].
///
/// # Errors
///
/// Fails for mismatched compiled sources or error-level geometry diagnostics.
pub fn export_gltf(
    source: &impl GltfSource,
    compiled: &impl AsRef<CompiledParts>,
) -> Result<GltfExport, GltfError> {
    export_gltf_with_options(source, compiled, GltfExportOptions::default())
}

/// Exports the caller-prepared scene with explicit coordinate-system options.
///
/// The Z-up to Y-up conversion is represented by one scene-root node. Mesh
/// buffers, accessor bounds, normals, winding, and item transforms remain in
/// their authored local coordinates and are transformed coherently by the
/// glTF node hierarchy.
///
/// # Errors
///
/// Fails for mismatched compiled sources or error-level geometry diagnostics.
pub fn export_gltf_with_options(
    source: &impl GltfSource,
    compiled: &impl AsRef<CompiledParts>,
    options: GltfExportOptions<'_>,
) -> Result<GltfExport, GltfError> {
    finish_gltf(build_export(source, compiled, options, None)?)
}

/// Exports glTF using real, caller-resolved glTF materials.
///
/// Resolution is strict: bound keys must have valid descriptions. Unassigned
/// regions remain without a material. Geometry is read from `compiled`; material
/// edits never invoke compilation. Instances with different materials share
/// geometry buffers while retaining separate mesh primitives.
///
/// # Errors
///
/// Returns [`GltfError::MissingMaterial`], [`GltfError::InvalidMaterial`], or
/// [`GltfError::UnsupportedMaterialField`] for unresolved, invalid, or unsupported
/// used descriptions. Texture references can return [`GltfError::MissingTexture`],
/// [`GltfError::InvalidTexture`], or [`GltfError::MissingTextureCoordinates`].
/// Geometry errors follow the same conditions as [`export_gltf`].
pub fn export_gltf_with_materials(
    source: &impl GltfSource,
    compiled: &impl AsRef<CompiledParts>,
    materials: &dyn MaterialResolver,
    options: GltfExportOptions<'_>,
) -> Result<GltfExport, GltfError> {
    finish_gltf(build_export(source, compiled, options, Some(materials))?)
}

fn finish_gltf(built: BuiltExport) -> Result<GltfExport, GltfError> {
    let mut document = built.document;
    if !built.buffer.is_empty() {
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
    }
    let json = serde_json::to_string_pretty(&Value::Object(document))
        .expect("document is finite JSON by construction");
    Ok(GltfExport {
        json,
        stats: built.stats,
    })
}

/// Exports the caller-prepared scene as a binary glTF 2.0 container.
///
/// Material keys use deterministic preview colors. For authored appearance,
/// use [`export_glb_with_materials`].
///
/// # Errors
///
/// Fails for invalid export records, mismatched compiled sources, error-level geometry diagnostics,
/// or a finished GLB exceeding its 32-bit container length.
pub fn export_glb(
    source: &impl GltfSource,
    compiled: &impl AsRef<CompiledParts>,
) -> Result<GlbExport, GltfError> {
    export_glb_with_options(source, compiled, GltfExportOptions::default())
}

/// Exports the caller-prepared scene as GLB with explicit coordinate-system options.
///
/// # Errors
///
/// Fails for invalid export records, mismatched compiled sources, error-level geometry diagnostics,
/// or a finished GLB exceeding its 32-bit container length.
pub fn export_glb_with_options(
    source: &impl GltfSource,
    compiled: &impl AsRef<CompiledParts>,
    options: GltfExportOptions<'_>,
) -> Result<GlbExport, GltfError> {
    finish_glb(build_export(source, compiled, options, None)?)
}

/// Exports a GLB using real, caller-resolved glTF materials.
///
/// Uses the same strict resolution and geometry sharing contract as
/// [`export_gltf_with_materials`], with geometry embedded in the GLB BIN chunk.
///
/// # Errors
///
/// Fails for invalid export records, missing, invalid, or unsupported used materials/resources or missing
/// authored texture coordinates, mismatched compiled
/// sources, error-level geometry diagnostics, or a GLB exceeding its length limit.
pub fn export_glb_with_materials(
    source: &impl GltfSource,
    compiled: &impl AsRef<CompiledParts>,
    materials: &dyn MaterialResolver,
    options: GltfExportOptions<'_>,
) -> Result<GlbExport, GltfError> {
    finish_glb(build_export(source, compiled, options, Some(materials))?)
}

fn finish_glb(built: BuiltExport) -> Result<GlbExport, GltfError> {
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
    source: &impl GltfSource,
    compiled: &impl AsRef<CompiledParts>,
    options: GltfExportOptions<'_>,
    resolver: Option<&dyn MaterialResolver>,
) -> Result<BuiltExport, GltfError> {
    let compiled = compiled.as_ref();
    let prepared = source.gltf_scene(compiled)?;
    let scene = prepared.as_ref();
    scene.validate(compiled)?;
    attributes::validate_mappings(options.attributes)?;
    for index in 0..compiled.parts().len() {
        let part = PartId(u32::try_from(index).expect("compiled input count"));
        if let Some(report) = compiled.report(part)
            && !report.clean_at(Severity::Error)
        {
            return Err(GltfError::IncompleteGeometry {
                part: part.0,
                report: Box::new(report.clone()),
            });
        }
    }
    let mut out = Emitter {
        attributes: options.attributes,
        resolver,
        buffer: Vec::new(),
        buffer_views: Vec::new(),
        accessors: Vec::new(),
        meshes: Vec::new(),
        materials: Vec::new(),
        material_index: HashMap::new(),
        mesh_index: HashMap::new(),
        geometry_index: HashMap::new(),
        stats: GltfStats::default(),
    };
    let lods = options.lods == GltfLods::MsftLod;
    let plan = instancing::plan(
        scene,
        compiled,
        options.instancing,
        options.lods,
        &mut out.stats,
    );
    let mut node_of = vec![None; scene.nodes.len()];
    let mut next = 0;
    for (index, batched) in plan.batched.iter().enumerate() {
        if !batched {
            node_of[index] = Some(next);
            next += 1;
        }
    }
    let mut children = vec![Vec::new(); scene.nodes.len()];
    for (index, node) in scene.nodes.iter().enumerate() {
        if let Some(parent) = node.parent
            && let Some(child) = node_of[index]
        {
            children[parent].push(child);
        }
    }
    let mut nodes: Vec<Value> = scene
        .nodes
        .iter()
        .enumerate()
        .filter(|(id, _)| node_of[*id].is_some())
        .map(|(id, node)| {
            if sheared(&node.transform) {
                out.stats.non_trs_matrices += 1;
            }
            logical_node(node, &children[id])
        })
        .collect();
    let mut lod_used = false;
    for (id, node) in scene.nodes.iter().enumerate() {
        let (Some(logical), Some(geometry)) = (node_of[id], &node.geometry) else {
            continue;
        };
        lod_used |= out.attach_occurrence(&mut nodes, compiled, geometry, logical, lods)?;
    }
    let mut batch_roots = Vec::new();
    for batch in &plan.batches {
        let (first, _) = batch.members[0];
        let geometry = scene.nodes[first]
            .geometry
            .as_ref()
            .expect("batched geometry");
        let trs: Vec<instancing::Trs> = batch.members.iter().map(|(_, trs)| *trs).collect();
        let transforms = out.instance_transforms(&trs, None, None);
        let table: Vec<Value> = batch
            .members
            .iter()
            .map(|(id, _)| {
                let mut identity = scene.nodes[*id].extras.clone();
                identity.remove("partKey");
                identity.insert("name".into(), json!(scene.nodes[*id].name));
                Value::Object(identity)
            })
            .collect();
        let mut extras = Map::new();
        extras.insert("instances".into(), Value::Array(table));
        let name = format!("{} [{} instances]", geometry.mesh.name, batch.members.len());
        let (top, used) = out.instanced_occurrence(
            &mut nodes,
            compiled,
            geometry,
            (&transforms, &name, extras),
            lods,
        )?;
        lod_used |= used;
        for index in top {
            match batch.parent {
                Some(parent) => append_child(
                    &mut nodes[node_of[parent].expect("parents retain nodes")],
                    index,
                ),
                None => batch_roots.push(index),
            }
        }
        out.stats.batched_instances += batch.members.len() as u64;
    }
    let mut set_roots = Vec::new();
    for set in &scene.placement_sets {
        let (top, used) = out.placement_set(&mut nodes, compiled, set, options.instancing, lods)?;
        lod_used |= used;
        for node in top {
            match set.parent {
                Some(parent) => append_child(
                    &mut nodes[node_of[parent].expect("set parents retain nodes")],
                    node,
                ),
                None => set_roots.push(node),
            }
        }
    }
    let Emitter {
        mut buffer,
        mut buffer_views,
        accessors,
        meshes,
        mut materials,
        mut stats,
        ..
    } = out;
    stats.nodes = nodes.len() as u64;

    let textures = match resolver {
        Some(resolver) => textures::embed(
            &mut materials,
            resolver,
            &mut buffer,
            &mut buffer_views,
            &mut stats,
        )?,
        None => textures::TextureTables::default(),
    };

    stats.buffer_bytes = buffer.len() as u64;
    let instance_roots: Vec<usize> = scene
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.parent.is_none())
        .filter_map(|(id, _)| node_of[id])
        .chain(set_roots)
        .chain(batch_roots)
        .collect();
    let scene_nodes = match options.coordinates {
        GltfCoordinates::Preserve => instance_roots,
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
            }));
            if !instance_roots.is_empty() {
                nodes[root]["children"] = json!(instance_roots);
            }
            stats.nodes += 1;
            vec![root]
        }
    };
    let mut document = Map::new();
    document.insert(
        "asset".into(),
        json!({ "version": "2.0", "generator": "exedra_gltf" }),
    );
    let mut used: Vec<&str> = Vec::new();
    for material in &materials {
        for name in materials::used_extensions(material) {
            if !used.contains(&name) {
                used.push(name);
            }
        }
    }
    if stats.instanced_nodes > 0 {
        used.push(instancing::EXTENSION);
    }
    if lod_used {
        used.push(LOD_EXTENSION);
    }
    used.sort_unstable();
    if !used.is_empty() {
        let required: Vec<&str> = used
            .iter()
            .copied()
            .filter(|name| {
                *name == instancing::EXTENSION
                    || (*name == materials::TEXTURE_TRANSFORM && options.require_texture_transform)
            })
            .collect();
        document.insert("extensionsUsed".into(), json!(used));
        if !required.is_empty() {
            document.insert("extensionsRequired".into(), json!(required));
        }
    }
    document.insert("scene".into(), json!(0));
    let scene = if scene_nodes.is_empty() {
        json!({})
    } else {
        json!({ "nodes": scene_nodes })
    };
    document.insert("scenes".into(), json!([scene]));
    // Optional glTF arrays must be omitted, rather than present and empty.
    for (name, values) in [
        ("nodes", nodes),
        ("meshes", meshes),
        ("materials", materials),
        ("images", textures.images),
        ("textures", textures.textures),
        ("samplers", textures.samplers),
        ("accessors", accessors),
        ("bufferViews", buffer_views),
    ] {
        if !values.is_empty() {
            document.insert(name.into(), Value::Array(values));
        }
    }
    Ok(BuiltExport {
        document,
        buffer,
        stats,
    })
}

fn logical_node(input: &GltfNode, children: &[usize]) -> Value {
    let mut node = json!({ "name": input.name, "extras": input.extras });
    if input.transform != Placement3::IDENTITY {
        node["matrix"] = json!(matrix_column_major(&input.transform.rows));
    }
    if !children.is_empty() {
        node["children"] = json!(children);
    }
    node
}

/// Growing export state shared by instance and batch nodes.
struct Emitter<'a> {
    attributes: &'a [GltfAttribute],
    resolver: Option<&'a dyn MaterialResolver>,
    buffer: Vec<u8>,
    buffer_views: Vec<Value>,
    accessors: Vec<Value>,
    meshes: Vec<Value>,
    materials: Vec<Value>,
    material_index: HashMap<String, usize>,
    // Content sharing is independent of local PartId assignment. Different
    // material resolutions reuse the same immutable vertex and index buffers.
    mesh_index: HashMap<(PartFingerprint, usize, Vec<Option<String>>), usize>,
    geometry_index: HashMap<(PartFingerprint, usize), GeometryAccessors>,
    stats: GltfStats,
}

impl Emitter<'_> {
    /// Returns the mesh for one body of `part` as `occurrence` resolves its
    /// materials, emitting geometry and primitives on first use.
    fn mesh(
        &mut self,
        mesh: &GltfMesh,
        body_index: usize,
        body: &CompiledBody,
    ) -> Result<usize, GltfError> {
        let part = mesh.part;
        let fingerprint = mesh.fingerprint;
        let regions: Vec<ResolvedRegion> = body
            .regions
            .iter()
            .enumerate()
            .map(|(index, range)| ResolvedRegion {
                region: range.region,
                start: range.start,
                count: range.count,
                material: mesh.material(body_index, index).map(str::to_owned),
            })
            .collect();
        let resolution = regions.iter().map(|r| r.material.clone()).collect();
        let key = (fingerprint, body_index, resolution);
        if let Some(&index) = self.mesh_index.get(&key) {
            return Ok(index);
        }
        let geometry = match self.geometry_index.entry((fingerprint, body_index)) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.get().clone(),
            std::collections::hash_map::Entry::Vacant(entry) => entry
                .insert(emit_geometry(
                    body,
                    self.attributes,
                    Site {
                        part: part.0,
                        body: body_index,
                    },
                    &mut self.buffer,
                    &mut self.buffer_views,
                    &mut self.accessors,
                    &mut self.stats,
                )?)
                .clone(),
        };
        let mesh = emit_mesh(
            &geometry,
            &regions,
            body,
            part,
            body_index,
            &mut self.accessors,
            &mut self.materials,
            &mut self.material_index,
            &mut self.stats,
            self.resolver,
        )?;
        let index = self.meshes.len();
        self.meshes.push(mesh);
        self.mesh_index.insert(key, index);
        self.stats.meshes += 1;
        Ok(index)
    }

    /// Writes one batch's `TRANSLATION`, `ROTATION` and `SCALE` accessors
    /// and returns the extension's `attributes` object.
    /// Writes `EXT_mesh_gpu_instancing` attribute accessors for `trs`, with
    /// optional `_SEED` and `_TINT` custom attributes of the same count.
    fn instance_transforms(
        &mut self,
        trs: &[instancing::Trs],
        seeds: Option<&[u64]>,
        tints: Option<&[[f32; 4]]>,
    ) -> Value {
        let count = trs.len();
        let translation: Vec<[f32; 3]> = trs.iter().map(|t| t.translation).collect();
        let rotation: Vec<[f32; 4]> = trs.iter().map(|t| t.rotation).collect();
        let scale: Vec<[f32; 3]> = trs.iter().map(|t| t.scale).collect();
        let mut push = |bytes: Vec<u8>, component: u32, kind: &str| {
            let view = push_view(&mut self.buffer, &mut self.buffer_views, &bytes);
            self.accessors.push(json!({
                "bufferView": view,
                "componentType": component,
                "count": count,
                "type": kind,
            }));
            self.accessors.len() - 1
        };
        let mut attributes = json!({
            "TRANSLATION": push(vec3_bytes(&translation), 5126, "VEC3"),
            "ROTATION": push(vec4_bytes(&rotation), 5126, "VEC4"),
            "SCALE": push(vec3_bytes(&scale), 5126, "VEC3"),
        });
        if let Some(seeds) = seeds {
            let bytes: Vec<u8> = seeds.iter().flat_map(|seed| seed.to_le_bytes()).collect();
            attributes["_SEED"] = json!(push(bytes, 5123, "VEC4"));
        }
        if let Some(tints) = tints {
            attributes["_TINT"] = json!(push(vec4_bytes(tints), 5126, "VEC4"));
        }
        attributes
    }

    /// Attaches the bodies of `level` to `node` as `occurrence` resolves its
    /// materials: a one-body part puts its mesh on `node`, a multi-body part
    /// gets one child per body named `name [body i]`.
    fn attach_bodies(
        &mut self,
        nodes: &mut Vec<Value>,
        compiled: &CompiledParts,
        mesh: &GltfMesh,
        node: usize,
        name: &str,
    ) -> Result<(), GltfError> {
        let entry = compiled.part(mesh.part).expect("validated compiled input");
        for (body_index, body) in entry.bodies.iter().enumerate() {
            let target = if entry.bodies.len() == 1 {
                node
            } else {
                let child = nodes.len();
                nodes.push(json!({
                    "name": format!("{name} [body {body_index}]"),
                    "extras": { "partKey": mesh.name, "body": body_index }
                }));
                append_child(&mut nodes[node], child);
                child
            };
            nodes[target]["extras"]["body"] = json!(body_index);
            // Empty geometry retains its logical node without illegal zero-count
            // mesh/accessor records. A geometry-free frame has no partKey at all.
            if body.tri.indices.is_empty() {
                continue;
            }
            let mesh_index = self.mesh(mesh, body_index, body)?;
            nodes[target]["mesh"] = json!(mesh_index);
        }
        Ok(())
    }

    /// Attaches the geometry of one occurrence of `part` to its logical node.
    /// Under [`GltfLods::MsftLod`], a part with a chain of at least two
    /// levels gets a `[lod 0]` child carrying `MSFT_lod`; returns whether it
    /// did.
    fn attach_occurrence(
        &mut self,
        nodes: &mut Vec<Value>,
        compiled: &CompiledParts,
        geometry: &GltfGeometry,
        logical: usize,
        lods: bool,
    ) -> Result<bool, GltfError> {
        let name = nodes[logical]["name"]
            .as_str()
            .expect("logical node name")
            .to_owned();
        let mesh = &geometry.mesh;
        let chain = &geometry.lods;
        if !lods || chain.len() < 2 {
            self.attach_bodies(nodes, compiled, mesh, logical, &name)?;
            return Ok(false);
        }
        let geometry = nodes.len();
        let base = format!("{name} [lod 0]");
        let mut extras = Map::new();
        extras.insert("partKey".into(), json!(mesh.name));
        extras.insert("lodLevel".into(), json!(0));
        lod_extras(&mut extras, chain);
        nodes.push(json!({ "name": base, "extras": extras }));
        append_child(&mut nodes[logical], geometry);
        self.attach_bodies(nodes, compiled, mesh, geometry, &base)?;
        let mut ids = Vec::new();
        for (level_index, level) in chain.iter().enumerate().skip(1) {
            let level_name = format!("{name} [lod {level_index}]");
            let index = nodes.len();
            nodes.push(json!({
                "name": level_name,
                "extras": { "partKey": level.mesh.name, "lodLevel": level_index },
            }));
            self.attach_bodies(nodes, compiled, &level.mesh, index, &level_name)?;
            ids.push(index);
            self.stats.lod_nodes += 1;
        }
        nodes[geometry]["extensions"] = json!({ LOD_EXTENSION: { "ids": ids } });
        Ok(true)
    }

    /// One instanced node per non-empty body of `level`, not yet attached.
    fn instanced_bodies(
        &mut self,
        nodes: &mut Vec<Value>,
        compiled: &CompiledParts,
        mesh: &GltfMesh,
        (transforms, name, extras): (&Value, &str, &Map<String, Value>),
        suffix_bodies: bool,
    ) -> Result<Vec<usize>, GltfError> {
        let entry = compiled.part(mesh.part).expect("validated compiled input");
        let mut created = Vec::new();
        for (body_index, body) in entry.bodies.iter().enumerate() {
            if body.tri.indices.is_empty() {
                continue;
            }
            let mesh_index = self.mesh(mesh, body_index, body)?;
            let mut body_name = name.to_owned();
            if suffix_bodies || entry.bodies.len() > 1 {
                let _ = write!(body_name, " [body {body_index}]");
            }
            let mut body_extras = Map::new();
            body_extras.insert("partKey".into(), json!(mesh.name));
            body_extras.insert("body".into(), json!(body_index));
            body_extras.extend(extras.iter().map(|(k, v)| (k.clone(), v.clone())));
            let index = nodes.len();
            nodes.push(json!({
                "name": body_name,
                "mesh": mesh_index,
                "extensions": {
                    instancing::EXTENSION: { "attributes": transforms.clone() }
                },
                "extras": body_extras,
            }));
            created.push(index);
            self.stats.instanced_nodes += 1;
        }
        Ok(created)
    }

    /// The instanced nodes of one batch (instances or a placement set), not
    /// yet attached. Without an `MSFT_lod` chain these are one node per
    /// non-empty body carrying `extras`; with one they are grouped under a
    /// single node that carries `extras` and `MSFT_lod`, whose lower levels
    /// reuse `transforms`. Returns the top nodes and whether `MSFT_lod` was
    /// written.
    fn instanced_occurrence(
        &mut self,
        nodes: &mut Vec<Value>,
        compiled: &CompiledParts,
        geometry: &GltfGeometry,
        (transforms, name, extras): (&Value, &str, Map<String, Value>),
        lods: bool,
    ) -> Result<(Vec<usize>, bool), GltfError> {
        let mesh = &geometry.mesh;
        let chain = &geometry.lods;
        if !lods || chain.len() < 2 {
            let top =
                self.instanced_bodies(nodes, compiled, mesh, (transforms, name, &extras), false)?;
            return Ok((top, false));
        }
        let group = nodes.len();
        let mut group_extras = extras;
        group_extras.insert("partKey".into(), json!(mesh.name));
        group_extras.insert("lodLevel".into(), json!(0));
        lod_extras(&mut group_extras, chain);
        nodes.push(json!({ "name": name, "extras": group_extras }));
        let level_extras = |level: usize| {
            let mut extras = Map::new();
            extras.insert("lodLevel".into(), json!(level));
            extras
        };
        let children = self.instanced_bodies(
            nodes,
            compiled,
            mesh,
            (transforms, &format!("{name} [lod 0]"), &level_extras(0)),
            // Every level names its bodies, so a level's grouping node and
            // its single body never share a name.
            true,
        )?;
        for child in children {
            append_child(&mut nodes[group], child);
        }
        let mut ids = Vec::new();
        for (level_index, level) in chain.iter().enumerate().skip(1) {
            let level_name = format!("{name} [lod {level_index}]");
            let index = nodes.len();
            nodes.push(json!({
                "name": level_name,
                "extras": { "partKey": level.mesh.name, "lodLevel": level_index },
            }));
            let children = self.instanced_bodies(
                nodes,
                compiled,
                &level.mesh,
                (transforms, &level_name, &level_extras(level_index)),
                true,
            )?;
            for child in children {
                append_child(&mut nodes[index], child);
            }
            ids.push(index);
            self.stats.lod_nodes += 1;
        }
        nodes[group]["extensions"] = json!({ LOD_EXTENSION: { "ids": ids } });
        Ok((vec![group], true))
    }

    /// The nodes of one placement set, not yet attached: under
    /// [`GltfInstancing::GpuInstancing`] one batch of every decomposable
    /// placement plus a node per mirrored or sheared placement; otherwise a
    /// node per placement. Returns the top nodes and whether `MSFT_lod` was
    /// written.
    fn placement_set(
        &mut self,
        nodes: &mut Vec<Value>,
        compiled: &CompiledParts,
        set: &GltfPlacementSet,
        mode: GltfInstancing,
        lods: bool,
    ) -> Result<(Vec<usize>, bool), GltfError> {
        let geometry = &set.geometry;
        let mesh = &geometry.mesh;
        let path = &set.name;
        let has_geometry = compiled
            .part(mesh.part)
            .expect("validated compiled input")
            .bodies
            .iter()
            .any(|body| !body.tri.indices.is_empty());
        let mut members: Vec<(usize, instancing::Trs)> = Vec::new();
        let mut singles: Vec<usize> = Vec::new();
        for (index, placement) in set.transforms.iter().enumerate() {
            if mode != GltfInstancing::GpuInstancing || !has_geometry {
                singles.push(index);
                continue;
            }
            match instancing::decompose(placement) {
                Ok(trs) => members.push((index, trs)),
                Err(instancing::Unbatchable::Mirrored) => {
                    self.stats.unbatched_mirrored_placements += 1;
                    singles.push(index);
                }
                Err(instancing::Unbatchable::Sheared) => {
                    self.stats.unbatched_sheared_placements += 1;
                    singles.push(index);
                }
                Err(instancing::Unbatchable::Unrepresentable) => {
                    self.stats.unbatched_unrepresentable_placements += 1;
                    singles.push(index);
                }
            }
        }
        let metadata = &set.extras;
        let mut top = Vec::new();
        let mut lod_used = false;
        if !members.is_empty() {
            let trs: Vec<instancing::Trs> = members.iter().map(|(_, trs)| *trs).collect();
            let seeds: Option<Vec<u64>> = set
                .seeds
                .as_deref()
                .map(|seeds| members.iter().map(|(i, _)| seeds[*i]).collect());
            let tints: Option<Vec<[f32; 4]>> = set
                .tints
                .as_deref()
                .map(|tints| members.iter().map(|(i, _)| tints[*i]).collect());
            let transforms = self.instance_transforms(&trs, seeds.as_deref(), tints.as_deref());
            let mut extras = metadata.clone();
            extras.insert("setPath".into(), json!(path));
            extras.insert("placementCount".into(), json!(members.len()));
            if members.len() != set.transforms.len() {
                let indices: Vec<usize> = members.iter().map(|(i, _)| *i).collect();
                extras.insert("placementIndices".into(), json!(indices));
            }
            let name = format!("{path} [{} placements]", members.len());
            let (nodes_top, used) = self.instanced_occurrence(
                nodes,
                compiled,
                geometry,
                (&transforms, &name, extras),
                lods,
            )?;
            top.extend(nodes_top);
            lod_used |= used;
            self.stats.batched_placements += members.len() as u64;
        }
        for index in singles {
            let placement_path = format!("{path}#{index}");
            let mut extras = metadata.clone();
            extras.insert("instancePath".into(), json!(placement_path));
            extras.insert("partKey".into(), json!(mesh.name));
            if let Some(seeds) = set.seeds.as_deref() {
                extras.insert("seed".into(), json!(seeds[index].to_string()));
            }
            if let Some(tints) = set.tints.as_deref() {
                extras.insert("tint".into(), json!(tints[index].map(shortest_decimal)));
            }
            let mut node = json!({ "name": placement_path, "extras": extras });
            let placement = set.transforms[index];
            if placement != Placement3::IDENTITY {
                node["matrix"] = json!(matrix_column_major(&placement.rows));
            }
            if sheared(&placement) {
                self.stats.non_trs_matrices += 1;
            }
            let logical = nodes.len();
            nodes.push(node);
            lod_used |= self.attach_occurrence(nodes, compiled, geometry, logical, lods)?;
            top.push(logical);
            self.stats.placement_nodes += 1;
        }
        Ok((top, lod_used))
    }
}

/// An authored `f32` as the `f64` with its shortest decimal form, so JSON
/// reads `0.05`, not the widened `0.05000000074505806`. Non-finite values
/// never reach extras: set tints and LOD thresholds are validated finite.
fn shortest_decimal(value: f32) -> f64 {
    value.to_string().parse::<f64>().expect("finite f32")
}

/// The `MSFT_lod` node extension name.
const LOD_EXTENSION: &str = "MSFT_lod";

/// Adds a chain's `MSFT_screencoverage` thresholds, one per level, and the
/// crossfade bands that `MSFT_lod` has no field for.
fn lod_extras(extras: &mut Map<String, Value>, chain: &[GltfLod]) {
    let coverage: Vec<f64> = chain
        .iter()
        .map(|l| shortest_decimal(l.min_coverage))
        .collect();
    let crossfade: Vec<f64> = chain
        .iter()
        .map(|l| shortest_decimal(l.crossfade))
        .collect();
    extras.insert("MSFT_screencoverage".into(), json!(coverage));
    extras.insert("exedraLodCrossfade".into(), json!(crossfade));
}

/// True when `placement` has no TRS decomposition (sheared or degenerate).
fn sheared(placement: &Placement3) -> bool {
    matches!(
        instancing::decompose(placement),
        Err(instancing::Unbatchable::Sheared)
    )
}

fn append_child(node: &mut Value, child: usize) {
    let children = &mut node["children"];
    if children.is_null() {
        *children = json!([child]);
    } else {
        children
            .as_array_mut()
            .expect("node children are an array")
            .push(json!(child));
    }
}

fn pack_glb(mut document: Map<String, Value>, mut buffer: Vec<u8>) -> Result<Vec<u8>, GltfError> {
    if !buffer.is_empty() {
        document.insert("buffers".into(), json!([{ "byteLength": buffer.len() }]));
    }
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
        .and_then(|length| {
            if buffer.is_empty() {
                Some(length)
            } else {
                length.checked_add(8)?.checked_add(buffer.len())
            }
        })
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
    if !buffer.is_empty() {
        glb.extend_from_slice(&buffer_len.to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&buffer);
    }
    Ok(glb)
}

#[derive(Clone, Debug)]
struct GeometryAccessors {
    positions: usize,
    normals: usize,
    uvs: usize,
    /// `TANGENT` and mapped streams, in accessor order.
    extra: Vec<(String, usize)>,
    indices_view: usize,
}

/// Emits a compiled body's geometry once, independently of material binding.
fn emit_geometry(
    body: &CompiledBody,
    mappings: &[GltfAttribute],
    site: Site,
    buffer: &mut Vec<u8>,
    buffer_views: &mut Vec<Value>,
    accessors: &mut Vec<Value>,
    stats: &mut GltfStats,
) -> Result<GeometryAccessors, GltfError> {
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

    let mut extra = Vec::new();
    let mut push_extra = |semantic: String, bytes: &[u8], mut accessor: Value, stride| {
        let view = push_view(buffer, buffer_views, bytes);
        if let Some(stride) = stride {
            buffer_views[view]["byteStride"] = json!(stride);
        }
        accessor["bufferView"] = json!(view);
        extra.push((semantic, accessors.len()));
        accessors.push(accessor);
        stats.attribute_accessors += 1;
        stats.attribute_bytes += bytes.len() as u64;
    };
    if !tri.tangents.is_empty() {
        push_extra(
            "TANGENT".to_owned(),
            &vec4_bytes(&tri.tangents),
            json!({ "componentType": 5126, "count": vertex_count, "type": "VEC4" }),
            None,
        );
    }
    let (encoded, missing) = attributes::encode(tri, mappings, site)?;
    stats.missing_attribute_streams += missing;
    stats.unmapped_attribute_streams += tri
        .attributes
        .iter()
        .filter(|stream| !mappings.iter().any(|m| m.selects(stream)))
        .count() as u64;
    for stream in encoded {
        push_extra(
            stream.semantic,
            &stream.bytes,
            stream.accessor,
            stream.byte_stride,
        );
    }

    let mut index_bytes = Vec::with_capacity(tri.indices.len() * 4);
    for &i in &tri.indices {
        index_bytes.extend_from_slice(&i.to_le_bytes());
    }
    let indices_view = push_view(buffer, buffer_views, &index_bytes);

    Ok(GeometryAccessors {
        positions: positions_accessor,
        normals: normals_accessor,
        uvs: uvs_accessor,
        extra,
        indices_view,
    })
}

/// Emits the primitive wrapper for one material resolution of shared geometry.
fn emit_mesh(
    geometry: &GeometryAccessors,
    regions: &[ResolvedRegion],
    body: &CompiledBody,
    part: PartId,
    body_index: usize,
    accessors: &mut Vec<Value>,
    materials: &mut Vec<Value>,
    material_index: &mut HashMap<String, usize>,
    stats: &mut GltfStats,
    resolver: Option<&dyn MaterialResolver>,
) -> Result<Value, GltfError> {
    let mut primitives: Vec<Value> = Vec::new();
    for (region, source) in regions.iter().zip(&body.regions) {
        let indices_accessor = accessors.len();
        accessors.push(json!({
            "bufferView": geometry.indices_view,
            "byteOffset": region.start as usize * 4,
            "componentType": 5125,
            "count": region.count,
            "type": "SCALAR",
        }));
        let mut primitive = Map::new();
        let mut attributes = json!({
            "POSITION": geometry.positions,
            "NORMAL": geometry.normals,
            "TEXCOORD_0": geometry.uvs,
        });
        for (semantic, accessor) in &geometry.extra {
            attributes[semantic.as_str()] = json!(accessor);
        }
        primitive.insert("attributes".into(), attributes);
        primitive.insert("indices".into(), json!(indices_accessor));
        primitive.insert("extras".into(), json!({ "faceRegion": region.region }));
        if let Some(material) = &region.material {
            let index = match material_index.entry(material.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let value = match resolver {
                        Some(resolver) => materials::validate(
                            resolver.resolve(material).ok_or_else(|| {
                                GltfError::MissingMaterial {
                                    key: material.clone(),
                                }
                            })?,
                            material,
                        )?,
                        None => material_stub(material),
                    };
                    materials.push(value);
                    stats.materials += 1;
                    *entry.insert(materials.len() - 1)
                }
            };
            let exports = |semantic: &str| geometry.extra.iter().any(|(s, _)| s == semantic);
            for slot in materials::TEXTURE_SLOTS {
                let Some(info) = materials[index].pointer(slot.pointer) else {
                    continue;
                };
                for set in materials::tex_coords(info) {
                    match set {
                        0 if !source.has_uvs => {
                            return Err(GltfError::MissingTextureCoordinates {
                                material: material.clone(),
                                part: part.0,
                                body: body_index,
                                region: region.region,
                            });
                        }
                        0 => {}
                        set if !exports(&format!("TEXCOORD_{set}")) => {
                            return Err(GltfError::MissingTextureCoordinateSet {
                                material: material.clone(),
                                texture: slot.field,
                                set,
                                part: part.0,
                                body: body_index,
                                region: region.region,
                            });
                        }
                        _ => {}
                    }
                }
            }
            let normal_mapped = materials::NORMAL_SLOTS
                .iter()
                .any(|pointer| materials[index].pointer(pointer).is_some());
            if normal_mapped && !exports("TANGENT") {
                stats.normal_maps_without_tangents += 1;
            }
            primitive.insert("material".into(), json!(index));
        }
        primitives.push(Value::Object(primitive));
        stats.primitives += 1;
    }

    Ok(json!({ "primitives": primitives }))
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

fn vec4_bytes(values: &[[f32; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 16);
    for c in values.as_flattened() {
        out.extend_from_slice(&c.to_le_bytes());
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
