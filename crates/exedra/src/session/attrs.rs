// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

impl<S: ChangeSink> EditSession<'_, S> {
    /// Returns an immutable view of the mesh being edited.
    #[must_use]
    pub fn mesh(&self) -> &Mesh {
        self.mesh
    }

    pub(crate) fn mesh_mut(&mut self) -> &mut Mesh {
        self.mesh
    }

    pub(crate) fn add_vertex_impl(&mut self, position: [f32; 3]) -> VertexId {
        let vertex = self.mesh.add_vertex(position);
        self.sink.record_created_vertex(vertex);
        self.sink.mark_vertex_dirty(vertex);
        vertex
    }

    pub(crate) fn set_vertex_position_impl(
        &mut self,
        vertex: VertexId,
        position: [f32; 3],
    ) -> bool {
        let updated = self.mesh.set_vertex_position(vertex, position);
        if updated {
            self.sink.mark_vertex_dirty(vertex);
        }
        updated
    }

    pub(crate) fn set_face_region_impl(&mut self, face: FaceId, region: u32) -> bool {
        let updated = self
            .mesh
            .attrs_mut()
            .dense_mut(attr::FACE_REGION)
            .is_some_and(|layer| layer.set(face.as_id(), region));
        if updated {
            self.sink.mark_face_dirty(face);
        }
        updated
    }

    /// Returns the corner UV value for `corner`, when present.
    #[must_use]
    pub fn corner_uv(&self, corner: CornerId) -> Option<[f32; 2]> {
        self.mesh
            .attrs()
            .sparse(attr::CORNER_UV)
            .and_then(|layer| layer.get(corner.as_id()).copied())
    }

    pub(crate) fn set_corner_uv_impl(&mut self, corner: CornerId, uv: [f32; 2]) -> bool {
        if self.mesh.half_edges.get(corner.as_id()).is_none() {
            return false;
        }
        if self.mesh.attrs().sparse(attr::CORNER_UV).is_none() {
            let _ = self.mesh.attrs_mut().define_sparse(attr::CORNER_UV);
        }
        let updated = self
            .mesh
            .attrs_mut()
            .sparse_mut(attr::CORNER_UV)
            .is_some_and(|layer| {
                layer.set(corner.as_id(), uv);
                true
            });
        if updated {
            self.sink.mark_corner_dirty(corner);
        }
        updated
    }

    /// Returns explicit seam state for an undirected edge.
    #[must_use]
    pub fn edge_seam(&self, half_edge: HalfEdgeId) -> Option<bool> {
        self.mesh.edge_seam(half_edge)
    }

    pub(crate) fn set_edge_seam_impl(&mut self, half_edge: HalfEdgeId, seam: bool) -> bool {
        let Some(twin) = self.mesh.twin(half_edge) else {
            return false;
        };
        let updated = self.mesh.set_edge_seam(half_edge, seam);
        if updated {
            self.sink.mark_corner_dirty(half_edge);
            self.sink.mark_corner_dirty(twin);
        }
        updated
    }

    /// Returns explicit sharpness value for an undirected edge.
    #[must_use]
    pub fn edge_sharpness(&self, half_edge: HalfEdgeId) -> Option<f32> {
        self.mesh.edge_sharpness(half_edge)
    }

    pub(crate) fn set_edge_sharpness_impl(&mut self, half_edge: HalfEdgeId, sharp: f32) -> bool {
        let Some(twin) = self.mesh.twin(half_edge) else {
            return false;
        };
        let updated = self.mesh.set_edge_sharpness(half_edge, sharp);
        if updated {
            self.sink.mark_corner_dirty(half_edge);
            self.sink.mark_corner_dirty(twin);
        }
        updated
    }
}
