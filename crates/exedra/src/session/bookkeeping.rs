// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

impl EditSession<'_> {
    /// Marks a face dirty.
    pub fn mark_face_dirty(&mut self, face: FaceId) {
        self.dirty.mark_face(face);
    }

    /// Marks a vertex dirty.
    pub fn mark_vertex_dirty(&mut self, vertex: VertexId) {
        self.dirty.mark_vertex(vertex);
    }

    /// Marks a corner dirty.
    pub fn mark_corner_dirty(&mut self, corner: CornerId) {
        self.dirty.mark_corner(corner);
    }

    /// Records a created half-edge in this transaction.
    ///
    /// This is intended for topology-edit kernels that mutate half-edge
    /// storage and must report created IDs in the resulting [`ChangeSet`].
    pub fn record_created_half_edge(&mut self, half_edge: HalfEdgeId) {
        self.created_half_edges.push(half_edge);
    }

    /// Records a created face in this transaction.
    ///
    /// This is intended for topology-edit kernels that mutate face storage and
    /// must report created IDs in the resulting [`ChangeSet`].
    pub fn record_created_face(&mut self, face: FaceId) {
        self.created_faces.push(face);
    }

    /// Records a deleted vertex in this transaction.
    ///
    /// Topology-edit kernels should call this when a vertex is removed.
    pub fn record_deleted_vertex(&mut self, vertex: VertexId) {
        self.deleted_vertices.push(vertex);
        self.dirty.mark_vertex(vertex);
    }

    /// Records a deleted half-edge in this transaction.
    ///
    /// Topology-edit kernels should call this when a half-edge is removed.
    pub fn record_deleted_half_edge(&mut self, half_edge: HalfEdgeId) {
        self.deleted_half_edges.push(half_edge);
        self.dirty.mark_corner(half_edge);
    }

    /// Records a deleted face in this transaction.
    ///
    /// Topology-edit kernels should call this when a face is removed.
    pub fn record_deleted_face(&mut self, face: FaceId) {
        self.deleted_faces.push(face);
        self.dirty.mark_face(face);
    }

    /// Commits this transaction and returns a deterministic change summary.
    ///
    /// Commit always increments [`Mesh::revision`](crate::Mesh::revision)
    /// exactly once, even when no mesh fields were changed.
    #[must_use]
    pub fn commit(mut self) -> ChangeSet {
        sort_dedup(&mut self.created_vertices);
        sort_dedup(&mut self.created_half_edges);
        sort_dedup(&mut self.created_faces);
        sort_dedup(&mut self.deleted_vertices);
        sort_dedup(&mut self.deleted_half_edges);
        sort_dedup(&mut self.deleted_faces);
        self.mesh.revision = self
            .mesh
            .revision
            .checked_add(1)
            .expect("mesh revision overflowed u64");

        ChangeSet {
            dirty: self.dirty,
            created_vertices: self.created_vertices,
            created_half_edges: self.created_half_edges,
            created_faces: self.created_faces,
            deleted_vertices: self.deleted_vertices,
            deleted_half_edges: self.deleted_half_edges,
            deleted_faces: self.deleted_faces,
        }
    }

    /// Explicitly discards transaction bookkeeping without rollback.
    pub fn abort(self) {}
}
