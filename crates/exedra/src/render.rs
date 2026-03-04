// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic render extraction from polygonal mesh to triangle mesh.
//!
//! Use [`Mesh::to_trimesh`](crate::Mesh::to_trimesh) to produce [`TriMesh`]
//! output for downstream rendering.

use alloc::vec::Vec;

use crate::attributes::SparseLayer;
use crate::{CornerId, FaceId, Mesh, VertexId, attr};

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
    /// Render-vertex normals (placeholder in v0.1).
    ///
    /// v0.1 always emits one zero normal per render vertex.
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
/// # Example
/// ```rust
/// use exedra::{ExtractMode, ExtractParams};
///
/// let params = ExtractParams {
///     mode: ExtractMode::FullRebuild,
/// };
/// assert_eq!(params.mode, ExtractMode::FullRebuild);
/// ```
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExtractParams {
    /// Extraction mode.
    pub mode: ExtractMode,
}

impl Default for ExtractParams {
    fn default() -> Self {
        Self {
            mode: ExtractMode::FullRebuild,
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
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct RenderVertexKey {
    vertex: VertexId,
    uv_bits: [u32; 2],
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
    /// - keys are `(VertexId, corner_uv_bits)`
    /// - shared topology vertices split when corner UVs differ
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

        let mut mesh = TriMesh::default();
        let mut stats = ExtractStats::default();
        let mut keys = Vec::<RenderVertexKey>::new();
        let mut seen_vertex_uv = Vec::<(VertexId, [u32; 2])>::new();

        for face in self.faces() {
            emit_face(
                self,
                face,
                corner_uvs,
                &mut mesh,
                &mut keys,
                &mut seen_vertex_uv,
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
    mesh: &mut TriMesh,
    keys: &mut Vec<RenderVertexKey>,
    seen_vertex_uv: &mut Vec<(VertexId, [u32; 2])>,
    stats: &mut ExtractStats,
) {
    for triangle in source.triangulate_face_fan(face) {
        for corner in triangle {
            let index = resolve_render_vertex(
                source,
                corner,
                corner_uvs,
                mesh,
                keys,
                seen_vertex_uv,
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
    mesh: &mut TriMesh,
    keys: &mut Vec<RenderVertexKey>,
    seen_vertex_uv: &mut Vec<(VertexId, [u32; 2])>,
    stats: &mut ExtractStats,
) -> u32 {
    let vertex = source
        .from_vertex(corner)
        .expect("face triangulation corner must have origin vertex");
    let uv = corner_uvs
        .and_then(|layer| layer.get(corner.as_id()).copied())
        .unwrap_or([0.0, 0.0]);
    let key = RenderVertexKey {
        vertex,
        uv_bits: [uv[0].to_bits(), uv[1].to_bits()],
    };

    // TODO(exe-qcmn): replace linear key scan with a hash map for large meshes.
    if let Some(index) = keys.iter().position(|existing| *existing == key) {
        return u32::try_from(index).expect("render vertex index overflowed u32");
    }

    // v0.1 split semantics: count every new UV variant encountered for a
    // previously seen topology vertex.
    // TODO(exe-qcmn): replace linear scans with per-vertex variant tracking.
    if seen_vertex_uv
        .iter()
        .any(|(seen_vertex, seen_uv)| *seen_vertex == vertex && *seen_uv != key.uv_bits)
    {
        stats.split_count = stats.split_count.saturating_add(1);
    }
    if !seen_vertex_uv
        .iter()
        .any(|(seen_vertex, _)| *seen_vertex == vertex)
    {
        seen_vertex_uv.push((vertex, key.uv_bits));
    }

    let position = *source
        .vertex_position(vertex)
        .expect("live vertex must have builtin position");
    keys.push(key);
    mesh.positions.push(position);
    mesh.uvs.push(uv);
    mesh.normals.push([0.0, 0.0, 0.0]);
    u32::try_from(mesh.positions.len() - 1).expect("render vertex index overflowed u32")
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use crate::{ExtractParams, MeshBuilder, attr};

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
                    .from_vertex(*corner)
                    .is_some_and(|v| v.index() == 1)
            })
            .expect("shared vertex corner should exist");
        let corner1 = built
            .mesh
            .face_loop(face1)
            .find(|corner| {
                built
                    .mesh
                    .from_vertex(*corner)
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
        assert!(stats.render_vertex_count > 4);
        assert_eq!(mesh.indices.len(), 6);
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
