// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic render extraction from polygonal mesh to triangle mesh.
//!
//! Use [`Mesh::to_trimesh`](crate::Mesh::to_trimesh) to produce [`TriMesh`]
//! output for downstream rendering.

use alloc::vec::Vec;
use hashbrown::HashMap;

use crate::attributes::SparseLayer;
use crate::{
    CornerId, FaceId, FaceTriangulation, Mesh, NormalParams, NormalsSource, VertexId, attr,
};

/// Triangle mesh suitable for GPU upload.
///
/// Produced by [`Mesh::to_trimesh`]. The buffers are parallel by
/// render-vertex index: `positions[i]`, `uvs[i]`, and `normals[i]`
/// describe vertex `i`, and `indices` references those vertices in
/// triangle order.
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
/// authored corner overrides, or a hybrid of both.
///
/// # Example
/// ```rust
/// use exedra::{ExtractMode, ExtractParams, NormalsSource};
///
/// let params = ExtractParams {
///     mode: ExtractMode::FullRebuild,
///     normals: NormalsSource::Derived,
///     ..ExtractParams::default()
/// };
/// assert_eq!(params.mode, ExtractMode::FullRebuild);
/// assert_eq!(params.face_triangulation, exedra::FaceTriangulation::Fan);
/// ```
#[derive(Copy, Clone, Debug, PartialEq)]
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
}

impl Default for ExtractParams {
    fn default() -> Self {
        Self {
            mode: ExtractMode::FullRebuild,
            normals: NormalsSource::Derived,
            normal_params: NormalParams::default(),
            face_triangulation: FaceTriangulation::Fan,
        }
    }
}

/// Deterministic extraction counters returned by [`Mesh::to_trimesh`].
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
    /// Number of faces where [`FaceTriangulation::Robust`] fell back to the
    /// fan because the projected polygon was not simple. Always zero under
    /// [`FaceTriangulation::Fan`].
    pub robust_fallback_count: u64,
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
    /// - keys are `(VertexId, corner_uv_bits, corner_normal_bits)`
    /// - shared topology vertices split when corner UVs or corner normals differ
    ///
    /// # Example
    /// ```rust
    /// use exedra::{ExtractParams, Mesh};
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
    /// # Ok::<(), exedra::BuildError>(())
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
            params: *params,
            mesh: mesh.clone(),
            stats,
        });
        (mesh, stats)
    }

    /// Unconditional full extraction (shared by both entry points).
    fn extract_full(&self, params: &ExtractParams) -> (TriMesh, ExtractStats) {
        let corner_uvs = self.attrs().sparse(attr::CORNER_UV);
        let derived_normals = self.derive_corner_normals(&params.normal_params);
        let normal_overrides = self.attrs().sparse(attr::CORNER_NORMAL_OVERRIDE);

        let mut mesh = TriMesh::default();
        let mut stats = ExtractStats::default();
        let mut key_to_index = HashMap::<RenderVertexKey, u32>::new();
        let mut vertex_variants = HashMap::<VertexId, VertexVariants>::new();

        for face in self.faces() {
            emit_face(
                self,
                face,
                params.face_triangulation,
                corner_uvs,
                normal_overrides,
                &derived_normals,
                params.normals,
                &mut mesh,
                &mut key_to_index,
                &mut vertex_variants,
                &mut stats,
            );
        }

        stats.render_vertex_count = mesh.positions.len() as u64;
        (mesh, stats)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal helper threading fixed extraction context"
)]
fn emit_face(
    source: &Mesh,
    face: FaceId,
    strategy: FaceTriangulation,
    corner_uvs: Option<&SparseLayer<[f32; 2]>>,
    normal_overrides: Option<&SparseLayer<[f32; 3]>>,
    derived_normals: &crate::DerivedCornerNormals,
    normals_source: NormalsSource,
    mesh: &mut TriMesh,
    key_to_index: &mut HashMap<RenderVertexKey, u32>,
    vertex_variants: &mut HashMap<VertexId, VertexVariants>,
    stats: &mut ExtractStats,
) {
    let (triangles, fell_back) = source.face_triangles_counted(face, strategy);
    if fell_back {
        stats.robust_fallback_count = stats.robust_fallback_count.saturating_add(1);
    }
    for triangle in triangles {
        for corner in triangle {
            let index = resolve_render_vertex(
                source,
                corner,
                corner_uvs,
                normal_overrides,
                derived_normals,
                normals_source,
                mesh,
                key_to_index,
                vertex_variants,
                stats,
            );
            mesh.indices.push(index);
        }
        stats.triangle_count = stats.triangle_count.saturating_add(1);
    }
}

fn resolve_render_vertex(
    source: &Mesh,
    corner: CornerId,
    corner_uvs: Option<&SparseLayer<[f32; 2]>>,
    normal_overrides: Option<&SparseLayer<[f32; 3]>>,
    derived_normals: &crate::DerivedCornerNormals,
    normals_source: NormalsSource,
    mesh: &mut TriMesh,
    key_to_index: &mut HashMap<RenderVertexKey, u32>,
    vertex_variants: &mut HashMap<VertexId, VertexVariants>,
    stats: &mut ExtractStats,
) -> u32 {
    let vertex = source
        .to_vertex(corner)
        .expect("face triangulation corner must have destination vertex");
    let uv = corner_uvs
        .and_then(|layer| layer.get(corner.as_id()).copied())
        .unwrap_or([0.0, 0.0]);
    let normal = effective_corner_normal(corner, normal_overrides, derived_normals, normals_source);
    let key = RenderVertexKey {
        vertex,
        uv_bits: [uv[0].to_bits(), uv[1].to_bits()],
        normal_bits: [
            normal[0].to_bits(),
            normal[1].to_bits(),
            normal[2].to_bits(),
        ],
    };

    if let Some(&index) = key_to_index.get(&key) {
        return index;
    }

    let variants = vertex_variants.entry(vertex).or_default();
    let uv_split = variants.has_other_uv(key.uv_bits);
    let normal_split = variants.has_other_normal(key.normal_bits);
    if uv_split || normal_split {
        stats.split_count = stats.split_count.saturating_add(1);
        if uv_split {
            stats.uv_split_count = stats.uv_split_count.saturating_add(1);
        }
        if normal_split {
            stats.normal_split_count = stats.normal_split_count.saturating_add(1);
        }
    }
    variants.record(key.uv_bits, key.normal_bits);

    let position = *source
        .vertex_position(vertex)
        .expect("live vertex must have builtin position");
    let index = u32::try_from(mesh.positions.len()).expect("render vertex index overflowed u32");
    key_to_index.insert(key, index);
    mesh.positions.push(position);
    mesh.uvs.push(uv);
    mesh.normals.push(normal);
    index
}

fn effective_corner_normal(
    corner: CornerId,
    normal_overrides: Option<&SparseLayer<[f32; 3]>>,
    derived_normals: &crate::DerivedCornerNormals,
    source: NormalsSource,
) -> [f32; 3] {
    let override_normal = normal_overrides.and_then(|layer| layer.get(corner.as_id()).copied());
    match source {
        NormalsSource::Derived => derived_normals.get(corner).unwrap_or([0.0, 0.0, 0.0]),
        NormalsSource::CustomOrDerived => override_normal
            .or_else(|| derived_normals.get(corner))
            .unwrap_or([0.0, 0.0, 0.0]),
        NormalsSource::CustomOnly => override_normal.unwrap_or([0.0, 0.0, 0.0]),
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use crate::{ExtractParams, FaceTriangulation, MeshBuilder, NormalsSource, TriMesh, attr, op};

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
}
