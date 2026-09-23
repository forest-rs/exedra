// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic render extraction from polygonal mesh to triangle mesh.
//!
//! Use [`Mesh::to_trimesh`](crate::Mesh::to_trimesh) to produce [`TriMesh`]
//! output for downstream rendering.

use alloc::vec::Vec;
use hashbrown::HashMap;

#[cfg(test)]
mod attribute_tests;
mod attributes;
mod validate;
pub use attributes::{
    AttributeBuffer, AttributeKind, AttributeStream, AttributeValue, ExtractAttribute, StreamValue,
};
pub use validate::TriMeshGeometryError;

use attributes::BoundAttribute;

use crate::attributes::{AttrKey, SparseLayer};
use crate::{
    BoxPlane, CornerId, DEFAULT_BOX_NORMAL_EPSILON, DerivedCornerNormals, FaceId,
    FaceTriangulation, Mesh, NormalParams, NormalsSource, UvSource, VertexId, attr,
    dominant_box_plane, project_box_position,
};

/// Triangle mesh suitable for GPU upload.
///
/// Produced by [`Mesh::to_trimesh`]. The buffers are parallel by
/// render-vertex index: `positions[i]`, `uvs[i]`, `normals[i]`, and value
/// `i` of every stream in `attributes` describe vertex `i`, and `indices`
/// references those vertices in triangle order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TriMesh {
    /// Triangle index buffer.
    pub indices: Vec<u32>,
    /// Render-vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Render-vertex UVs.
    pub uvs: Vec<[f32; 2]>,
    /// Render-vertex normals.
    pub normals: Vec<[f32; 3]>,
    /// Extra render-vertex streams, in [`ExtractParams::attributes`] order.
    pub attributes: Vec<AttributeStream>,
}

impl TriMesh {
    /// Returns the values extracted from the layer named by `key`.
    ///
    /// Matches both the key's domain and name. When a request list carried the
    /// same layer twice, both streams hold identical values and the first is
    /// returned.
    #[must_use]
    pub fn attribute<T>(&self, key: AttrKey<T>) -> Option<&AttributeBuffer> {
        self.attributes
            .iter()
            .find(|stream| stream.domain == key.domain() && stream.name == key.name())
            .map(|stream| &stream.values)
    }
}

/// Extraction mode used by [`ExtractParams`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ExtractMode {
    /// Rebuilds output from full mesh state.
    #[default]
    FullRebuild,
    /// Requests reuse of prior extraction output.
    ///
    /// A bare [`Mesh::to_trimesh`] call has no prior state to reuse, so it
    /// performs a full rebuild and reports it in
    /// [`ExtractStats::incremental_fallbacks`] — visible, never silent.
    /// Actual reuse routes through [`Mesh::to_trimesh_cached`] with a
    /// caller-owned [`TrimeshCache`].
    Incremental,
}

/// Render extraction parameters for [`Mesh::to_trimesh`](crate::Mesh::to_trimesh).
///
/// [`ExtractMode::Incremental`] reuses prior output only through
/// [`Mesh::to_trimesh_cached`]; without a cache it is a counted full
/// rebuild.
///
/// `normals` selects whether extraction uses derived geometry normals,
/// authored corner overrides, or a hybrid of both. `uvs` selects what a
/// corner without an authored UV emits: zero, or a box projection of its
/// position. `attributes` lists further layers to emit as extra streams.
///
/// # Example
/// ```rust
/// use exedra_mesh::{ExtractMode, ExtractParams, NormalsSource, UvSource};
///
/// let params = ExtractParams {
///     mode: ExtractMode::FullRebuild,
///     normals: NormalsSource::Derived,
///     ..ExtractParams::default()
/// };
/// assert_eq!(params.mode, ExtractMode::FullRebuild);
/// assert_eq!(params.face_triangulation, exedra_mesh::FaceTriangulation::Fan);
/// assert_eq!(params.uvs, UvSource::CustomOnly);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractParams {
    /// Extraction mode.
    pub mode: ExtractMode,
    /// Normal source policy used for emitted render vertices.
    pub normals: NormalsSource,
    /// Parameters used when deriving geometry normals.
    pub normal_params: NormalParams,
    /// Per-face triangle enumeration strategy.
    ///
    /// [`FaceTriangulation::Fan`] preserves the historical byte-identical
    /// output; [`FaceTriangulation::Robust`] handles concave ngon faces via
    /// the shared deterministic triangulator.
    pub face_triangulation: FaceTriangulation,
    /// UV source policy used for emitted render vertices.
    ///
    /// [`UvSource::CustomOnly`] preserves the historical output: corners
    /// without an authored UV emit `[0.0, 0.0]`. Projected UVs take part in
    /// render-vertex splitting exactly like authored ones.
    pub uvs: UvSource,
    /// Attribute layers emitted as extra streams in
    /// [`TriMesh::attributes`], in this order.
    ///
    /// Empty by default, which preserves the historical output exactly.
    /// See [`ExtractAttribute`] for resolution and splitting rules.
    pub attributes: Vec<ExtractAttribute>,
}

impl Default for ExtractParams {
    fn default() -> Self {
        Self {
            mode: ExtractMode::FullRebuild,
            normals: NormalsSource::Derived,
            normal_params: NormalParams::default(),
            face_triangulation: FaceTriangulation::Fan,
            uvs: UvSource::CustomOnly,
            attributes: Vec::new(),
        }
    }
}

/// Deterministic extraction counters returned by [`Mesh::to_trimesh`].
///
/// `split_count` counts each render vertex created for a topology vertex that
/// already had one. The per-cause counters (`uv_split_count`,
/// `normal_split_count`, `attribute_split_count`) count that new render vertex
/// under every cause whose values already vary at the vertex, not only the
/// cause that forced this split. They can therefore sum to more than
/// `split_count`, and carrying an attribute can raise `uv_split_count` at a
/// vertex that also has UV seams.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtractStats {
    /// Number of emitted triangles.
    pub triangle_count: u64,
    /// Number of emitted render vertices.
    pub render_vertex_count: u64,
    /// Number of seam-driven render-vertex splits.
    pub split_count: u64,
    /// Number of UV-driven render-vertex splits.
    pub uv_split_count: u64,
    /// Number of normal-driven render-vertex splits.
    pub normal_split_count: u64,
    /// Number of render-vertex splits driven by a carried attribute value.
    pub attribute_split_count: u64,
    /// Number of carried attribute values, per render vertex and attribute,
    /// that used [`ExtractAttribute::missing`] because the layer had no value.
    pub attribute_fallback_count: u64,
    /// Number of carried attributes with no layer of the requested value type.
    /// Every value of such a stream is its missing value.
    pub missing_attribute_layers: u64,
    /// Number of faces where [`FaceTriangulation::Robust`] fell back to the
    /// fan because the projected polygon was not simple. Always zero under
    /// [`FaceTriangulation::Fan`].
    pub robust_fallback_count: u64,
    /// Number of faces whose degenerate normal forced the `+Z` plane while
    /// box-projecting corners under [`UvSource::CustomOrBoxProjected`].
    /// Always zero under [`UvSource::CustomOnly`] and for fully authored
    /// faces, which never project.
    pub uv_projection_fallback_count: u64,
    /// Number of full rebuilds performed where reuse was requested but no
    /// usable prior output existed ([`ExtractMode::Incremental`] without a
    /// cache, an empty [`TrimeshCache`], or a stale one).
    pub incremental_fallbacks: u64,
    /// Number of times [`Mesh::to_trimesh_cached`] returned the cached
    /// output wholesale (unchanged revision and parameters).
    pub full_reuses: u64,
}

/// Caller-owned reuse state for [`Mesh::to_trimesh_cached`].
///
/// Pinned to [`Mesh::revision`] exactly like
/// `exedra_constructive`'s source maps: extraction is a pure function of
/// mesh state, so an unchanged revision under unchanged parameters means
/// the cached output *is* the full rebuild's output, bit for bit. Any
/// revision or parameter change falls back to a counted full rebuild that
/// refreshes the cache.
///
/// Like a source map, a cache is bound to one logical mesh: reusing it
/// across unrelated meshes whose revisions happen to coincide is a caller
/// contract violation the pin cannot detect.
#[derive(Clone, Debug, Default)]
pub struct TrimeshCache {
    entry: Option<CacheEntry>,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    revision: crate::MeshRevision,
    params: ExtractParams,
    mesh: TriMesh,
    stats: ExtractStats,
}

impl TrimeshCache {
    /// Creates an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True when the cache holds a prior extraction.
    #[must_use]
    pub fn is_primed(&self) -> bool {
        self.entry.is_some()
    }

    /// Drops any cached extraction.
    pub fn clear(&mut self) {
        self.entry = None;
    }
}

#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct RenderVertexKey {
    vertex: VertexId,
    uv_bits: [u32; 2],
    normal_bits: [u32; 3],
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct VertexVariants {
    uv_bits: Vec<[u32; 2]>,
    normal_bits: Vec<[u32; 3]>,
    /// Render vertices of this vertex; recorded only with carried attributes.
    render_vertices: Vec<u32>,
}

impl VertexVariants {
    fn has_other_uv(&self, uv_bits: [u32; 2]) -> bool {
        self.uv_bits.iter().any(|seen| *seen != uv_bits)
    }

    fn has_other_normal(&self, normal_bits: [u32; 3]) -> bool {
        self.normal_bits.iter().any(|seen| *seen != normal_bits)
    }

    fn record(&mut self, uv_bits: [u32; 2], normal_bits: [u32; 3]) {
        if !self.uv_bits.contains(&uv_bits) {
            self.uv_bits.push(uv_bits);
        }
        if !self.normal_bits.contains(&normal_bits) {
            self.normal_bits.push(normal_bits);
        }
    }
}

impl Mesh {
    /// Extracts a deterministic triangle mesh representation.
    ///
    /// Deterministic ordering:
    /// - faces in arena slot order
    /// - per-face triangles from [`Mesh::face_triangles`] under
    ///   [`ExtractParams::face_triangulation`] (fan by default)
    /// - render vertices appended on first encounter during traversal
    ///
    /// Render vertex splitting:
    /// - keys are `(VertexId, corner_uv_bits, corner_normal_bits)` plus the
    ///   bits of every carried [`ExtractParams::attributes`] value
    /// - shared topology vertices split when corner UVs, corner normals, or
    ///   carried values differ
    /// - the UV and normal of a corner are whatever [`ExtractParams::uvs`] and
    ///   [`ExtractParams::normals`] resolve for it, so projected UVs and
    ///   derived normals split exactly like authored ones
    ///
    /// # Example
    /// ```rust
    /// use exedra_mesh::{ExtractParams, Mesh};
    ///
    /// let positions = [
    ///     [0.0, 0.0, 0.0],
    ///     [1.0, 0.0, 0.0],
    ///     [0.0, 1.0, 0.0],
    /// ];
    /// let triangles = [[0_u32, 1, 2]];
    /// let mesh = Mesh::from_indexed_triangles(&positions, &triangles, &Default::default())?;
    ///
    /// let (tri, stats) = mesh.to_trimesh(&ExtractParams::default());
    /// assert_eq!(tri.indices, vec![0, 1, 2]);
    /// assert_eq!(stats.triangle_count, 1);
    /// # Ok::<(), exedra_mesh::BuildError>(())
    /// ```
    pub fn to_trimesh(&self, params: &ExtractParams) -> (TriMesh, ExtractStats) {
        let (mesh, mut stats) = self.extract_full(params);
        if params.mode == ExtractMode::Incremental {
            // No prior state to reuse here — a full rebuild, counted so
            // callers see the fallback instead of assuming reuse happened.
            stats.incremental_fallbacks = 1;
        }
        (mesh, stats)
    }

    /// Extracts through a caller-owned reuse cache.
    ///
    /// When `cache` holds output for this mesh's current
    /// [`revision`](Mesh::revision) under equal `params`, that output is
    /// returned wholesale (counted in [`ExtractStats::full_reuses`]) —
    /// bit-identical to a fresh [`Mesh::to_trimesh`] because extraction is
    /// a pure function of mesh state. Otherwise a full rebuild runs, the
    /// cache is refreshed, and the fallback is counted in
    /// [`ExtractStats::incremental_fallbacks`] (except on the very first,
    /// unprimed use, which is an ordinary rebuild).
    ///
    /// See [`TrimeshCache`] for the single-mesh binding contract.
    pub fn to_trimesh_cached(
        &self,
        params: &ExtractParams,
        cache: &mut TrimeshCache,
    ) -> (TriMesh, ExtractStats) {
        let revision = self.revision();
        if let Some(entry) = &cache.entry
            && entry.revision == revision
            && entry.params == *params
        {
            let mut stats = entry.stats;
            stats.full_reuses = 1;
            return (entry.mesh.clone(), stats);
        }
        let was_primed = cache.entry.is_some();
        let (mesh, mut stats) = self.extract_full(params);
        if was_primed {
            stats.incremental_fallbacks = 1;
        }
        cache.entry = Some(CacheEntry {
            revision,
            params: params.clone(),
            mesh: mesh.clone(),
            stats,
        });
        (mesh, stats)
    }

    /// Unconditional full extraction (shared by both entry points).
    fn extract_full(&self, params: &ExtractParams) -> (TriMesh, ExtractStats) {
        let corner_uvs = self.attrs().sparse(attr::CORNER_UV);
        let normal_overrides = self.attrs().sparse(attr::CORNER_NORMAL_OVERRIDE);
        // Authored meshes need no geometric normals unless a live face corner
        // actually falls back to them. Sparse-layer length is insufficient:
        // it can include boundary corners or stale IDs.
        let needs_derived = match params.normals {
            NormalsSource::Derived => true,
            NormalsSource::CustomOnly => false,
            NormalsSource::CustomOrDerived => self.faces().any(|face| {
                self.face_loop(face).any(|corner| {
                    normal_overrides.is_none_or(|layer| layer.get(corner.as_id()).is_none())
                })
            }),
        };
        let derived_normals = if needs_derived {
            self.derive_corner_normals(&params.normal_params)
        } else {
            DerivedCornerNormals::default()
        };

        let attributes: Vec<BoundAttribute<'_>> = params
            .attributes
            .iter()
            .map(|request| BoundAttribute::bind(self, *request))
            .collect();
        let inputs = CornerInputs {
            corner_uvs,
            normal_overrides,
            derived_normals: &derived_normals,
            normals: params.normals,
            uvs: params.uvs,
            attributes: &attributes,
        };
        let mut out = Emission {
            mesh: TriMesh {
                attributes: attributes
                    .iter()
                    .map(BoundAttribute::empty_stream)
                    .collect(),
                ..TriMesh::default()
            },
            stats: ExtractStats {
                missing_attribute_layers: attributes
                    .iter()
                    .filter(|bound| !bound.is_bound())
                    .count() as u64,
                ..ExtractStats::default()
            },
            stride: params
                .attributes
                .iter()
                .map(|request| request.kind().components())
                .sum(),
            ..Emission::default()
        };

        for face in self.faces() {
            emit_face(self, face, params.face_triangulation, &inputs, &mut out);
        }

        let Emission {
            mesh, mut stats, ..
        } = out;
        stats.render_vertex_count = mesh.positions.len() as u64;
        (mesh, stats)
    }
}

/// Read-only per-corner attribute sources and policies for one extraction.
#[derive(Copy, Clone, Debug)]
struct CornerInputs<'a> {
    corner_uvs: Option<&'a SparseLayer<[f32; 2]>>,
    normal_overrides: Option<&'a SparseLayer<[f32; 3]>>,
    derived_normals: &'a DerivedCornerNormals,
    normals: NormalsSource,
    uvs: UvSource,
    attributes: &'a [BoundAttribute<'a>],
}

/// Sentinel ending a [`Emission::next_same_key`] chain.
const NO_RENDER_VERTEX: u32 = u32::MAX;

/// Growing output of one extraction.
#[derive(Debug, Default)]
struct Emission {
    mesh: TriMesh,
    stats: ExtractStats,
    /// First render vertex emitted for each `(vertex, uv, normal)` key.
    key_to_index: HashMap<RenderVertexKey, u32>,
    vertex_variants: HashMap<VertexId, VertexVariants>,
    /// Total carried components per render vertex.
    stride: usize,
    /// Carried value bits, `stride` words per render vertex.
    carried_bits: Vec<u32>,
    /// Next render vertex sharing a key but differing in carried values.
    next_same_key: Vec<u32>,
    /// Scratch buffer for one corner's carried bits.
    scratch: Vec<u32>,
    /// Scratch flags: which carried attributes of the corner fell back.
    scratch_fallbacks: Vec<bool>,
}

impl Emission {
    fn carried(&self, index: u32) -> &[u32] {
        let start = index as usize * self.stride;
        &self.carried_bits[start..start + self.stride]
    }
}

/// UV fallback resolved once per face for corners without an authored UV.
#[derive(Copy, Clone, Debug, PartialEq)]
enum FaceUvFallback {
    /// Historical behaviour: missing corners emit `[0.0, 0.0]`.
    Zero,
    /// Project the corner position on `plane` and multiply by `scale`.
    Box { plane: BoxPlane, scale: f32 },
}

impl FaceUvFallback {
    /// Selects the fallback for `face`. The plane is chosen per face, so all
    /// projected corners of one face share it; only faces that actually have a
    /// corner without an authored UV pay for the face-normal computation.
    /// The second value reports a degenerate normal that forced the `+Z`
    /// plane.
    fn for_face(source: &Mesh, face: FaceId, inputs: &CornerInputs<'_>) -> (Self, bool) {
        match inputs.uvs {
            UvSource::CustomOnly => (Self::Zero, false),
            UvSource::CustomOrBoxProjected { scale } => {
                let fully_authored = inputs.corner_uvs.is_some_and(|layer| {
                    source
                        .face_loop(face)
                        .all(|corner| layer.get(corner.as_id()).is_some())
                });
                if fully_authored {
                    return (Self::Zero, false);
                }
                let (plane, fell_back) =
                    dominant_box_plane(source, face, DEFAULT_BOX_NORMAL_EPSILON);
                (Self::Box { plane, scale }, fell_back)
            }
        }
    }

    fn resolve(self, position: [f32; 3]) -> [f32; 2] {
        match self {
            Self::Zero => [0.0, 0.0],
            Self::Box { plane, scale } => project_box_position(position, plane, scale, [0.0, 0.0]),
        }
    }
}

fn emit_face(
    source: &Mesh,
    face: FaceId,
    strategy: FaceTriangulation,
    inputs: &CornerInputs<'_>,
    out: &mut Emission,
) {
    let (triangles, fell_back) = source.face_triangles_counted(face, strategy);
    if fell_back {
        out.stats.robust_fallback_count = out.stats.robust_fallback_count.saturating_add(1);
    }
    let (uv_fallback, projection_fell_back) = FaceUvFallback::for_face(source, face, inputs);
    if projection_fell_back {
        out.stats.uv_projection_fallback_count =
            out.stats.uv_projection_fallback_count.saturating_add(1);
    }
    for triangle in triangles {
        for corner in triangle {
            let index = resolve_render_vertex(source, face, corner, uv_fallback, inputs, out);
            out.mesh.indices.push(index);
        }
        out.stats.triangle_count = out.stats.triangle_count.saturating_add(1);
    }
}

fn resolve_render_vertex(
    source: &Mesh,
    face: FaceId,
    corner: CornerId,
    uv_fallback: FaceUvFallback,
    inputs: &CornerInputs<'_>,
    out: &mut Emission,
) -> u32 {
    let vertex = source
        .to_vertex(corner)
        .expect("face triangulation corner must have destination vertex");
    let position = *source
        .vertex_position(vertex)
        .expect("live vertex must have builtin position");
    let uv = inputs
        .corner_uvs
        .and_then(|layer| layer.get(corner.as_id()).copied())
        .unwrap_or_else(|| uv_fallback.resolve(position));
    let normal = effective_corner_normal(corner, inputs);
    let key = RenderVertexKey {
        vertex,
        uv_bits: [uv[0].to_bits(), uv[1].to_bits()],
        normal_bits: [
            normal[0].to_bits(),
            normal[1].to_bits(),
            normal[2].to_bits(),
        ],
    };

    out.scratch.clear();
    out.scratch_fallbacks.clear();
    for bound in inputs.attributes {
        let found = bound.push_corner_bits(corner, vertex, face, &mut out.scratch);
        out.scratch_fallbacks.push(!found);
    }

    // Render vertices sharing a key form a chain in emission order; the one
    // whose carried bits match is reused. Without carried attributes every
    // chain has one element, so output matches the historical keying.
    let mut tail = None;
    let mut cursor = out.key_to_index.get(&key).copied();
    while let Some(index) = cursor {
        if out.carried(index) == out.scratch.as_slice() {
            return index;
        }
        tail = Some(index);
        let next = out.next_same_key[index as usize];
        cursor = (next != NO_RENDER_VERTEX).then_some(next);
    }

    let index =
        u32::try_from(out.mesh.positions.len()).expect("render vertex index overflowed u32");
    let variants = out.vertex_variants.entry(vertex).or_default();
    let uv_split = variants.has_other_uv(key.uv_bits);
    let normal_split = variants.has_other_normal(key.normal_bits);
    let attribute_split = out.stride > 0
        && variants.render_vertices.iter().any(|&other| {
            let start = other as usize * out.stride;
            out.carried_bits[start..start + out.stride] != *out.scratch
        });
    if uv_split || normal_split || attribute_split {
        out.stats.split_count = out.stats.split_count.saturating_add(1);
        if uv_split {
            out.stats.uv_split_count = out.stats.uv_split_count.saturating_add(1);
        }
        if normal_split {
            out.stats.normal_split_count = out.stats.normal_split_count.saturating_add(1);
        }
        if attribute_split {
            out.stats.attribute_split_count = out.stats.attribute_split_count.saturating_add(1);
        }
    }
    variants.record(key.uv_bits, key.normal_bits);
    if out.stride > 0 {
        variants.render_vertices.push(index);
    }

    match tail {
        Some(tail) => out.next_same_key[tail as usize] = index,
        None => {
            out.key_to_index.insert(key, index);
        }
    }
    out.next_same_key.push(NO_RENDER_VERTEX);
    out.mesh.positions.push(position);
    out.mesh.uvs.push(uv);
    out.mesh.normals.push(normal);
    let mut offset = 0;
    for (stream, &fell_back) in out.mesh.attributes.iter_mut().zip(&out.scratch_fallbacks) {
        let width = stream.values.kind().components();
        attributes::push_stream_bits(stream, &out.scratch[offset..offset + width]);
        offset += width;
        if fell_back {
            out.stats.attribute_fallback_count =
                out.stats.attribute_fallback_count.saturating_add(1);
        }
    }
    out.carried_bits.extend_from_slice(&out.scratch);
    index
}

fn effective_corner_normal(corner: CornerId, inputs: &CornerInputs<'_>) -> [f32; 3] {
    let override_normal = inputs
        .normal_overrides
        .and_then(|layer| layer.get(corner.as_id()).copied());
    match inputs.normals {
        NormalsSource::Derived => inputs
            .derived_normals
            .get(corner)
            .unwrap_or([0.0, 0.0, 0.0]),
        NormalsSource::CustomOrDerived => override_normal
            .or_else(|| inputs.derived_normals.get(corner))
            .unwrap_or([0.0, 0.0, 0.0]),
        NormalsSource::CustomOnly => override_normal.unwrap_or([0.0, 0.0, 0.0]),
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use crate::{
        DEFAULT_BOX_NORMAL_EPSILON, ExtractParams, FaceTriangulation, Mesh, MeshBuilder,
        NormalsSource, TriMesh, UvSource, attr, dominant_box_plane, op, project_corner_box,
    };

    /// Twice the signed XY area of trimesh triangle `t`.
    fn trimesh_tri_area2(tri: &TriMesh, t: usize) -> f32 {
        let i = |k: usize| tri.indices[t * 3 + k] as usize;
        let (a, b, c) = (
            tri.positions[i(0)],
            tri.positions[i(1)],
            tri.positions[i(2)],
        );
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }

    #[test]
    fn robust_extraction_handles_concave_ngon_faces() {
        // An L-shaped single-face ngon in the XY plane.
        let mut builder = MeshBuilder::new();
        for p in [
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 1.0],
            [1.0, 1.0],
            [1.0, 2.0],
            [0.0, 2.0],
        ] {
            builder.push_vertex([p[0], p[1], 0.0]);
        }
        builder
            .add_face(&[0, 1, 2, 3, 4, 5])
            .expect("L ngon should be valid");
        let built = builder.build().expect("build should succeed");

        let robust_params = ExtractParams {
            face_triangulation: FaceTriangulation::Robust,
            ..ExtractParams::default()
        };
        let (tri, stats) = built.mesh.to_trimesh(&robust_params);
        assert_eq!(stats.triangle_count, 4);
        assert_eq!(stats.robust_fallback_count, 0);
        let mut area2 = 0.0;
        for t in 0..tri.indices.len() / 3 {
            let a2 = trimesh_tri_area2(&tri, t);
            assert!(a2 > 0.0, "robust extraction must not invert triangles");
            area2 += a2;
        }
        assert!(
            (area2 - 6.0).abs() < 1e-5,
            "area sum {area2} must cover the L"
        );

        // The fan strategy inverts at least one triangle on this face —
        // the documented limitation robust extraction exists to fix.
        let (fan_tri, fan_stats) = built.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(fan_stats.robust_fallback_count, 0);
        assert!(
            (0..fan_tri.indices.len() / 3).any(|t| trimesh_tri_area2(&fan_tri, t) < 0.0),
            "fixture must demonstrate the fan limitation"
        );

        // Determinism double-run.
        let (again, _) = built.mesh.to_trimesh(&robust_params);
        assert_eq!(tri, again);
    }

    #[test]
    fn to_trimesh_triangle_without_uvs_uses_zero_uvs() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2])
            .expect("triangle should be valid");
        let built = builder.build().expect("build should succeed");

        let (mesh, stats) = built.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(mesh.uvs, vec![[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]]);
        assert_eq!(mesh.normals, vec![[0.0, 0.0, 1.0]; 3]);
        assert_eq!(stats.triangle_count, 1);
        assert_eq!(stats.render_vertex_count, 3);
        assert_eq!(stats.split_count, 0);
    }

    #[test]
    fn to_trimesh_quad_reuses_vertices_without_uv_seams() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 1.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("quad should be valid");
        let mut built = builder.build().expect("build should succeed");
        let _ = built.mesh.attrs_mut().define_sparse(attr::CORNER_UV);
        let corners = built.mesh.face_loop(built.face_ids[0]).collect::<Vec<_>>();
        let layer = built
            .mesh
            .attrs_mut()
            .sparse_mut(attr::CORNER_UV)
            .expect("corner uv layer must exist");
        for corner in corners {
            layer.set(corner.as_id(), [1.0, 1.0]);
        }

        let (mesh, stats) = built.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(mesh.indices.len(), 6);
        assert_eq!(stats.triangle_count, 2);
        assert_eq!(stats.render_vertex_count, 4);
        assert_eq!(stats.split_count, 0);
        assert_eq!(stats.uv_split_count, 0);
        assert_eq!(stats.normal_split_count, 0);
    }

    #[test]
    fn to_trimesh_splits_vertices_on_uv_discontinuity() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]); // 0
        builder.push_vertex([1.0, 0.0, 0.0]); // 1 shared
        builder.push_vertex([0.0, 1.0, 0.0]); // 2
        builder.push_vertex([1.0, 1.0, 0.0]); // 3
        builder
            .add_face(&[0, 1, 2])
            .expect("triangle face should be valid");
        builder
            .add_face(&[2, 1, 3])
            .expect("triangle face should be valid");
        let mut built = builder.build().expect("build should succeed");
        let _ = built.mesh.attrs_mut().define_sparse(attr::CORNER_UV);

        let face0 = built.face_ids[0];
        let face1 = built.face_ids[1];
        let corner0 = built
            .mesh
            .face_loop(face0)
            .find(|corner| {
                built
                    .mesh
                    .to_vertex(*corner)
                    .is_some_and(|v| v.index() == 1)
            })
            .expect("shared vertex corner should exist");
        let corner1 = built
            .mesh
            .face_loop(face1)
            .find(|corner| {
                built
                    .mesh
                    .to_vertex(*corner)
                    .is_some_and(|v| v.index() == 1)
            })
            .expect("shared vertex corner should exist");
        let layer = built
            .mesh
            .attrs_mut()
            .sparse_mut(attr::CORNER_UV)
            .expect("corner uv layer must exist");
        layer.set(corner0.as_id(), [0.0, 0.0]);
        layer.set(corner1.as_id(), [1.0, 0.0]);

        let (mesh, stats) = built.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats.triangle_count, 2);
        assert!(stats.split_count >= 1);
        assert!(stats.uv_split_count >= 1);
        assert!(stats.render_vertex_count > 4);
        assert_eq!(mesh.indices.len(), 6);
    }

    #[test]
    fn to_trimesh_splits_vertices_on_normal_discontinuity() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]); // 0
        builder.push_vertex([1.0, 0.0, 0.0]); // 1 shared
        builder.push_vertex([0.0, 1.0, 0.0]); // 2
        builder.push_vertex([1.0, 1.0, 0.0]); // 3
        builder
            .add_face(&[0, 1, 2])
            .expect("triangle face should be valid");
        builder
            .add_face(&[2, 1, 3])
            .expect("triangle face should be valid");
        let mut built = builder.build().expect("build should succeed");
        let face0 = built.face_ids[0];
        let face1 = built.face_ids[1];
        let corner0 = built
            .mesh
            .face_loop(face0)
            .find(|corner| {
                built
                    .mesh
                    .to_vertex(*corner)
                    .is_some_and(|v| v.index() == 1)
            })
            .expect("shared vertex corner should exist");
        let corner1 = built
            .mesh
            .face_loop(face1)
            .find(|corner| {
                built
                    .mesh
                    .to_vertex(*corner)
                    .is_some_and(|v| v.index() == 1)
            })
            .expect("shared vertex corner should exist");
        let mut edit = built.mesh.edit();
        op::set_corner_normal_override(&mut edit, corner0, Some([1.0, 0.0, 0.0]))
            .expect("corner override write should succeed");
        op::set_corner_normal_override(&mut edit, corner1, Some([0.0, 1.0, 0.0]))
            .expect("corner override write should succeed");
        let _: () = edit.finish();

        let (mesh, stats) = built.mesh.to_trimesh(&ExtractParams {
            normals: NormalsSource::CustomOrDerived,
            ..ExtractParams::default()
        });
        assert!(stats.split_count >= 1);
        assert!(stats.normal_split_count >= 1);
        assert!(stats.render_vertex_count > 4);
        assert_eq!(mesh.indices.len(), 6);
    }

    #[test]
    fn to_trimesh_custom_only_uses_authored_normals_when_present() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2])
            .expect("triangle should be valid");
        let mut built = builder.build().expect("build should succeed");
        let corner = built
            .mesh
            .faces()
            .flat_map(|face| built.mesh.face_loop(face))
            .next()
            .expect("triangle should have a corner");
        let mut edit = built.mesh.edit();
        op::set_corner_normal_override(&mut edit, corner, Some([1.0, 0.0, 0.0]))
            .expect("corner override write should succeed");
        let _: () = edit.finish();

        let (mesh, _) = built.mesh.to_trimesh(&ExtractParams {
            normals: NormalsSource::CustomOnly,
            ..ExtractParams::default()
        });
        assert_eq!(mesh.normals[0], [1.0, 0.0, 0.0]);
        assert_eq!(&mesh.normals[1..], &[[0.0; 3]; 2]);
    }

    #[test]
    fn authored_normal_coverage_uses_live_face_corners() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder.add_face(&[0, 1, 2]).expect("triangle");
        let mut mesh = builder.build().expect("build").mesh;
        let corners: Vec<_> = mesh.faces().flat_map(|face| mesh.face_loop(face)).collect();
        let params = ExtractParams {
            normals: NormalsSource::CustomOrDerived,
            ..ExtractParams::default()
        };
        let derived = mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(mesh.to_trimesh(&params), derived);

        let mut edit = mesh.edit();
        for &corner in &corners {
            op::set_corner_normal_override(&mut edit, corner, Some([1.0, 0.0, 0.0]))
                .expect("authored normal");
        }
        let _: () = edit.finish();
        let authored = mesh.to_trimesh(&params);
        assert_eq!(authored.0.normals, vec![[1.0, 0.0, 0.0]; 3]);
        assert_eq!(mesh.to_trimesh(&ExtractParams::default()), derived);
        assert_eq!(
            mesh.to_trimesh(&ExtractParams {
                normals: NormalsSource::CustomOnly,
                ..params.clone()
            }),
            authored
        );

        // Keep the same number of overrides, but move one to an OUTSIDE
        // half-edge. The now-missing face corner must still derive its normal.
        let missing = corners[2];
        let boundary = mesh.twin(missing).expect("boundary twin");
        let mut edit = mesh.edit();
        op::set_corner_normal_override(&mut edit, missing, None).expect("clear normal");
        op::set_corner_normal_override(&mut edit, boundary, Some([0.0, 1.0, 0.0]))
            .expect("boundary normal");
        let _: () = edit.finish();
        let (mixed, _) = mesh.to_trimesh(&params);
        assert_eq!(mixed.positions, authored.0.positions);
        assert_eq!(mixed.indices, authored.0.indices);
        assert_eq!(
            mixed.normals,
            vec![[1.0, 0.0, 0.0], [1.0, 0.0, 0.0], derived.0.normals[2]]
        );
    }

    #[test]
    fn incremental_mode_no_longer_panics_and_counts_its_fallback() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2])
            .expect("triangle should be valid");
        let built = builder.build().expect("build should succeed");

        let incremental = ExtractParams {
            mode: crate::ExtractMode::Incremental,
            ..ExtractParams::default()
        };
        let (tri, stats) = built.mesh.to_trimesh(&incremental);
        assert_eq!(stats.incremental_fallbacks, 1, "fallback is visible");
        let (full, full_stats) = built.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(full_stats.incremental_fallbacks, 0);
        assert_eq!(tri, full, "the fallback is an ordinary full rebuild");
    }

    #[test]
    fn cached_extraction_reuses_on_unchanged_revision_and_matches_rebuilds() {
        use crate::TrimeshCache;

        // Deterministic SplitMix64 for seeded edit sequences.
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = self.0;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                z ^ (z >> 31)
            }
            fn unit(&mut self) -> f32 {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "20 bits of test jitter into f32 is deliberate"
                )]
                {
                    (self.next() >> 44) as f32 / (1_u64 << 20) as f32
                }
            }
        }

        // A quad grid with enough structure for splits to matter.
        let mut builder = MeshBuilder::new();
        for y in 0..4_u32 {
            for x in 0..4_u32 {
                #[expect(clippy::cast_precision_loss, reason = "small grid coordinates")]
                builder.push_vertex([x as f32, y as f32, 0.0]);
            }
        }
        for y in 0..3_u32 {
            for x in 0..3_u32 {
                let i = y * 4 + x;
                builder
                    .add_face(&[i, i + 1, i + 5, i + 4])
                    .expect("grid quad should be valid");
            }
        }
        let mut built = builder.build().expect("build should succeed");
        let params = ExtractParams::default();
        let mut cache = TrimeshCache::new();
        let mut rng = Rng(0x00E1_D8A5_0000_0001);

        // First use: an ordinary rebuild primes the cache.
        let (first, first_stats) = built.mesh.to_trimesh_cached(&params, &mut cache);
        assert_eq!(first_stats.incremental_fallbacks, 0, "unprimed first use");
        assert_eq!(first, built.mesh.to_trimesh(&params).0);
        assert!(cache.is_primed());

        for round in 0..8 {
            // Unchanged revision: wholesale reuse, bit-identical.
            let (reused, reused_stats) = built.mesh.to_trimesh_cached(&params, &mut cache);
            assert_eq!(reused_stats.full_reuses, 1, "round {round}: reuse");
            assert_eq!(
                reused,
                built.mesh.to_trimesh(&params).0,
                "round {round}: reused output equals a fresh full rebuild"
            );

            // Edit: move a random vertex; the revision bumps and the next
            // cached extraction must fall back and match a fresh rebuild.
            let vertices: Vec<_> = built.mesh.vertices().collect();
            #[expect(
                clippy::cast_possible_truncation,
                reason = "test index selection over a tiny vertex set"
            )]
            let pick = vertices[(rng.next() % vertices.len() as u64) as usize];
            let jitter = [rng.unit(), rng.unit(), rng.unit()];
            let mut edit = built.mesh.edit();
            let base = edit
                .mesh()
                .vertex_position(pick)
                .copied()
                .expect("picked vertex is live");
            op::set_vertex_position(
                &mut edit,
                pick,
                [
                    base[0] + jitter[0],
                    base[1] + jitter[1],
                    base[2] + jitter[2],
                ],
            )
            .expect("vertex move should succeed");
            let _: () = edit.finish();

            let (rebuilt, rebuilt_stats) = built.mesh.to_trimesh_cached(&params, &mut cache);
            assert_eq!(
                rebuilt_stats.incremental_fallbacks, 1,
                "round {round}: revision change falls back, counted"
            );
            assert_eq!(
                rebuilt,
                built.mesh.to_trimesh(&params).0,
                "round {round}: fallback equals a fresh full rebuild"
            );
        }

        // A parameter change also refuses reuse.
        let robust = ExtractParams {
            face_triangulation: FaceTriangulation::Robust,
            ..ExtractParams::default()
        };
        let (tri, stats) = built.mesh.to_trimesh_cached(&robust, &mut cache);
        assert_eq!(stats.incremental_fallbacks, 1, "params are pinned");
        assert_eq!(tri, built.mesh.to_trimesh(&robust).0);
    }

    #[test]
    fn to_trimesh_is_deterministic_across_runs() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 1.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("quad should be valid");
        let built = builder.build().expect("build should succeed");

        let a = built.mesh.to_trimesh(&ExtractParams::default());
        let b = built.mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(a, b);
    }

    fn unit_quad_at_z1() -> Mesh {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 1.0]);
        builder.push_vertex([1.0, 0.0, 1.0]);
        builder.push_vertex([1.0, 1.0, 1.0]);
        builder.push_vertex([0.0, 1.0, 1.0]);
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("quad should be valid");
        builder.build().expect("build should succeed").mesh
    }

    fn box_projected(scale: f32) -> ExtractParams {
        ExtractParams {
            uvs: UvSource::CustomOrBoxProjected { scale },
            ..ExtractParams::default()
        }
    }

    #[test]
    fn box_projected_uvs_fill_missing_corners_from_positions() {
        let mesh = unit_quad_at_z1();
        let (tri, stats) = mesh.to_trimesh(&box_projected(1.0));
        assert_eq!(stats.render_vertex_count, 4);
        assert_eq!(stats.uv_split_count, 0);
        for (position, uv) in tri.positions.iter().zip(&tri.uvs) {
            assert_eq!(*uv, [position[0], position[1]]);
        }

        let (doubled, _) = mesh.to_trimesh(&box_projected(2.0));
        assert_eq!(doubled.positions, tri.positions);
        for (position, uv) in doubled.positions.iter().zip(&doubled.uvs) {
            assert_eq!(*uv, [position[0] * 2.0, position[1] * 2.0]);
        }

        // The default policy is untouched: missing corners still emit zero.
        let (zero, _) = mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(zero.uvs, vec![[0.0, 0.0]; 4]);
        assert_eq!(zero.positions, tri.positions);
        assert_eq!(zero.indices, tri.indices);
    }

    #[test]
    fn box_projection_keeps_authored_uvs_and_projects_the_rest() {
        let mut mesh = unit_quad_at_z1();
        let face = mesh.faces().next().expect("one face");
        let authored = mesh.face_loop(face).next().expect("quad corner");
        let authored_vertex = mesh.to_vertex(authored).expect("live corner");
        let mut edit = mesh.edit();
        op::set_corner_uv(&mut edit, authored, [7.0, 7.0]).expect("corner is live");
        let _: () = edit.finish();

        let (tri, stats) = mesh.to_trimesh(&box_projected(1.0));
        assert_eq!(stats.render_vertex_count, 4);
        let authored_position = *mesh
            .vertex_position(authored_vertex)
            .expect("live vertex has a position");
        let mut authored_seen = 0;
        for (position, uv) in tri.positions.iter().zip(&tri.uvs) {
            if *position == authored_position {
                assert_eq!(*uv, [7.0, 7.0], "authored UVs are never overwritten");
                authored_seen += 1;
            } else {
                assert_eq!(*uv, [position[0], position[1]]);
            }
        }
        assert_eq!(authored_seen, 1);
    }

    /// Closed axis-aligned box with outward-facing quads. Coordinates are
    /// chosen so no two box planes project any corner to the same UV.
    fn axis_aligned_box() -> Mesh {
        let (x0, x1, y0, y1, z0, z1) = (1.0, 2.0, 2.0, 3.0, 4.0, 6.0);
        let mut builder = MeshBuilder::new();
        for p in [
            [x0, y0, z0],
            [x1, y0, z0],
            [x1, y1, z0],
            [x0, y1, z0],
            [x0, y0, z1],
            [x1, y0, z1],
            [x1, y1, z1],
            [x0, y1, z1],
        ] {
            builder.push_vertex(p);
        }
        for face in [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [2, 3, 7, 6],
            [0, 4, 7, 3],
            [1, 2, 6, 5],
        ] {
            builder.add_face(&face).expect("box quad should be valid");
        }
        builder.build().expect("build should succeed").mesh
    }

    #[test]
    fn box_projection_splits_a_box_once_per_face_like_authored_uvs() {
        let projected_mesh = axis_aligned_box();
        let (projected, stats) = projected_mesh.to_trimesh(&box_projected(1.0));
        assert_eq!(
            stats.render_vertex_count, 24,
            "three planes meet at each corner"
        );
        assert_eq!(stats.uv_split_count, 16);
        assert_eq!(
            stats.normal_split_count, 0,
            "smooth derived normals never split"
        );

        // Author the same projection into the mesh and extract under the
        // default policy: the two paths must agree byte for byte.
        let mut authored_mesh = axis_aligned_box();
        let faces: Vec<_> = authored_mesh.faces().collect();
        let mut edit = authored_mesh.edit();
        for face in faces {
            let (plane, fell_back) =
                dominant_box_plane(edit.mesh(), face, DEFAULT_BOX_NORMAL_EPSILON);
            assert!(!fell_back, "box faces have well-defined normals");
            let corners: Vec<_> = edit.mesh().face_loop(face).collect();
            for corner in corners {
                let uv = project_corner_box(edit.mesh(), corner, plane, 1.0, [0.0, 0.0])
                    .expect("live corner");
                op::set_corner_uv(&mut edit, corner, uv).expect("corner is live");
            }
        }
        let _: () = edit.finish();
        let (authored, authored_stats) = authored_mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(authored_stats.render_vertex_count, 24);
        assert_eq!(authored, projected);

        // Fully authored faces are left alone under the projected policy too.
        assert_eq!(authored_mesh.to_trimesh(&box_projected(1.0)).0, authored);
    }

    #[test]
    fn degenerate_faces_report_the_projection_fallback() {
        // A collinear "quad" has no normal; projection must fall back to +Z
        // and say so. The default policy never projects, so it never counts.
        let mut builder = MeshBuilder::new();
        for x in [0.0, 1.0, 2.0, 3.0] {
            builder.push_vertex([x, 0.0, 0.0]);
        }
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("topologically valid quad");
        let mesh = builder.build().expect("build should succeed").mesh;

        let (tri, stats) = mesh.to_trimesh(&box_projected(1.0));
        assert_eq!(stats.uv_projection_fallback_count, 1);
        for (position, uv) in tri.positions.iter().zip(&tri.uvs) {
            assert_eq!(*uv, [position[0], position[1]], "+Z fallback projects XY");
        }
        let (_, zero_stats) = mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(zero_stats.uv_projection_fallback_count, 0);

        // Well-formed faces never count, regardless of their size.
        let small = {
            let mut builder = MeshBuilder::new();
            for p in [[0.0, 0.0, 0.0], [0.0, 5.0e-4, 0.0], [0.0, 5.0e-4, 5.0e-4]] {
                builder.push_vertex(p);
            }
            builder.add_face(&[0, 1, 2]).expect("small triangle");
            builder.build().expect("build should succeed").mesh
        };
        let (small_tri, small_stats) = small.to_trimesh(&box_projected(1.0));
        assert_eq!(small_stats.uv_projection_fallback_count, 0);
        for (position, uv) in small_tri.positions.iter().zip(&small_tri.uvs) {
            assert_eq!(
                *uv,
                [-position[2], position[1]],
                "+X plane, not the fallback"
            );
        }
    }

    #[test]
    fn uv_policy_is_pinned_by_the_extraction_cache() {
        use crate::TrimeshCache;

        let mesh = unit_quad_at_z1();
        let mut cache = TrimeshCache::new();
        let (zero, _) = mesh.to_trimesh_cached(&ExtractParams::default(), &mut cache);
        let (projected, stats) = mesh.to_trimesh_cached(&box_projected(1.0), &mut cache);
        assert_eq!(
            stats.incremental_fallbacks, 1,
            "a policy change refuses reuse"
        );
        assert_ne!(zero.uvs, projected.uvs);
        assert_eq!(projected, mesh.to_trimesh(&box_projected(1.0)).0);
    }
}
