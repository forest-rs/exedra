// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic render extraction from polygonal mesh to triangle mesh.
//!
//! Use [`Mesh::to_trimesh`](crate::Mesh::to_trimesh) to produce [`TriMesh`]
//! output for downstream rendering.

use alloc::vec::Vec;
use hashbrown::HashMap;

use crate::attributes::SparseLayer;
use crate::{CornerId, FaceId, Mesh, NormalParams, NormalsSource, VertexId, attr};

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
    /// v0.1 placeholder: currently behaves as full rebuild.
    Incremental,
}

/// Render extraction parameters for [`Mesh::to_trimesh`](crate::Mesh::to_trimesh).
///
/// In v0.1, [`ExtractMode::Incremental`] behaves as full rebuild.
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
///     normal_params: Default::default(),
/// };
/// assert_eq!(params.mode, ExtractMode::FullRebuild);
/// ```
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ExtractParams {
    /// Extraction mode.
    pub mode: ExtractMode,
    /// Normal source policy used for emitted render vertices.
    pub normals: NormalsSource,
    /// Parameters used when deriving geometry normals.
    pub normal_params: NormalParams,
}

impl Default for ExtractParams {
    fn default() -> Self {
        Self {
            mode: ExtractMode::FullRebuild,
            normals: NormalsSource::Derived,
            normal_params: NormalParams::default(),
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
    /// - per-face fan triangulation from [`Mesh::triangulate_face_fan`]
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
        if params.mode == ExtractMode::Incremental {
            debug_assert!(
                false,
                "ExtractMode::Incremental currently falls back to full rebuild in v0.1"
            );
        }
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

fn emit_face(
    source: &Mesh,
    face: FaceId,
    corner_uvs: Option<&SparseLayer<[f32; 2]>>,
    normal_overrides: Option<&SparseLayer<[f32; 3]>>,
    derived_normals: &crate::DerivedCornerNormals,
    normals_source: NormalsSource,
    mesh: &mut TriMesh,
    key_to_index: &mut HashMap<RenderVertexKey, u32>,
    vertex_variants: &mut HashMap<VertexId, VertexVariants>,
    stats: &mut ExtractStats,
) {
    for triangle in source.triangulate_face_fan(face) {
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

    use crate::{ExtractParams, MeshBuilder, NormalsSource, attr, op};

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
