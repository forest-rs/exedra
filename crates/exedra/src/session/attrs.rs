// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

impl EditSession<'_> {
    /// Returns the current edit propagation policy.
    #[must_use]
    pub const fn propagate_policy(&self) -> PropagatePolicy {
        self.propagate_policy
    }

    /// Replaces the edit propagation policy for this transaction.
    pub fn set_propagate_policy(&mut self, policy: PropagatePolicy) {
        self.propagate_policy = policy;
    }

    /// Returns an immutable view of the mesh being edited.
    #[must_use]
    pub fn mesh(&self) -> &Mesh {
        self.mesh
    }

    /// Adds a vertex and records deterministic change bookkeeping.
    pub fn add_vertex(&mut self, position: [f32; 3]) -> VertexId {
        let vertex = self.mesh.add_vertex(position);
        self.created_vertices.push(vertex);
        self.dirty.mark_vertex(vertex);
        vertex
    }

    /// Sets a vertex position and marks affected data dirty on success.
    pub fn set_vertex_position(&mut self, vertex: VertexId, position: [f32; 3]) -> bool {
        let updated = self.mesh.set_vertex_position(vertex, position);
        if updated {
            self.dirty.mark_vertex(vertex);
        }
        updated
    }

    /// Sets the built-in face region value and marks the face dirty on success.
    ///
    /// Returns `true` when `face` is live and writable.
    pub fn set_face_region(&mut self, face: FaceId, region: u32) -> bool {
        let updated = self
            .mesh
            .attrs_mut()
            .dense_mut(attr::FACE_REGION)
            .is_some_and(|layer| layer.set(face.as_id(), region));
        if updated {
            self.dirty.mark_face(face);
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

    /// Sets corner UV for `corner`, defining the sparse layer on first use.
    ///
    /// Returns `true` when `corner` is live and writable.
    pub fn set_corner_uv(&mut self, corner: CornerId, uv: [f32; 2]) -> bool {
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
            self.dirty.mark_corner(corner);
        }
        updated
    }

    /// Returns explicit seam state for an undirected edge.
    #[must_use]
    pub fn edge_seam(&self, half_edge: HalfEdgeId) -> Option<bool> {
        self.mesh.edge_seam(half_edge)
    }

    /// Sets explicit seam state for an undirected edge.
    ///
    /// Returns `true` when `half_edge` is live and writable.
    pub fn set_edge_seam(&mut self, half_edge: HalfEdgeId, seam: bool) -> bool {
        let Some(twin) = self.mesh.twin(half_edge) else {
            return false;
        };
        let updated = self.mesh.set_edge_seam(half_edge, seam);
        if updated {
            self.dirty.mark_corner(half_edge);
            self.dirty.mark_corner(twin);
        }
        updated
    }

    /// Returns explicit sharpness value for an undirected edge.
    #[must_use]
    pub fn edge_sharpness(&self, half_edge: HalfEdgeId) -> Option<f32> {
        self.mesh.edge_sharpness(half_edge)
    }

    /// Sets explicit sharpness value for an undirected edge.
    ///
    /// Returns `true` when `half_edge` is live and writable.
    pub fn set_edge_sharpness(&mut self, half_edge: HalfEdgeId, sharp: f32) -> bool {
        let Some(twin) = self.mesh.twin(half_edge) else {
            return false;
        };
        let updated = self.mesh.set_edge_sharpness(half_edge, sharp);
        if updated {
            self.dirty.mark_corner(half_edge);
            self.dirty.mark_corner(twin);
        }
        updated
    }
}
