// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

impl<S: ChangeSink> EditSession<'_, S> {
    /// Marks a face dirty.
    pub(crate) fn mark_face_dirty(&mut self, face: FaceId) {
        self.sink.mark_face_dirty(face);
    }

    /// Marks a vertex dirty.
    pub(crate) fn mark_vertex_dirty(&mut self, vertex: VertexId) {
        self.sink.mark_vertex_dirty(vertex);
    }

    /// Marks a corner dirty.
    pub(crate) fn mark_corner_dirty(&mut self, corner: CornerId) {
        self.sink.mark_corner_dirty(corner);
    }

    /// Records a created half-edge in this edit scope.
    ///
    /// This is intended for topology-edit kernels that mutate half-edge
    /// storage and must report created IDs in the resulting [`ChangeSet`].
    pub(crate) fn record_created_half_edge(&mut self, half_edge: HalfEdgeId) {
        self.sink.record_created_half_edge(half_edge);
    }

    /// Records a created face in this edit scope.
    ///
    /// This is intended for topology-edit kernels that mutate face storage and
    /// must report created IDs in the resulting [`ChangeSet`].
    pub(crate) fn record_created_face(&mut self, face: FaceId) {
        self.sink.record_created_face(face);
    }

    /// Records a deleted vertex in this edit scope.
    ///
    /// Topology-edit kernels should call this when a vertex is removed.
    pub(crate) fn record_deleted_vertex(&mut self, vertex: VertexId) {
        self.sink.record_deleted_vertex(vertex);
        self.clear_caller_values(crate::attributes::Domain::Vertex, vertex.as_id());
    }

    /// Records a deleted half-edge in this edit scope.
    ///
    /// Topology-edit kernels should call this when a half-edge is removed.
    pub(crate) fn record_deleted_half_edge(&mut self, half_edge: HalfEdgeId) {
        self.sink.record_deleted_half_edge(half_edge);
        self.clear_caller_values(crate::attributes::Domain::HalfEdge, half_edge.as_id());
    }

    /// Records a deleted face in this edit scope.
    ///
    /// Topology-edit kernels should call this when a face is removed.
    pub(crate) fn record_deleted_face(&mut self, face: FaceId) {
        self.sink.record_deleted_face(face);
        self.clear_caller_values(crate::attributes::Domain::Face, face.as_id());
    }

    /// Clears a deleted element's caller-defined values, so a recycled slot
    /// never inherits them, and reports how many carried information.
    fn clear_caller_values(&mut self, domain: crate::attributes::Domain, id: Id) {
        let cleared = self.mesh.attrs.clear_caller_values(domain, id);
        if cleared > 0 {
            self.sink.record_cleared_attribute_values(cleared);
        }
    }

    /// Carries caller-defined layers of `domain` onto `target` from `source`,
    /// or from `blend` under [`Propagation::Interpolate`](crate::attributes::Propagation::Interpolate),
    /// and reports what an unspecified rule could not carry.
    pub(crate) fn propagate_caller_layers(
        &mut self,
        domain: crate::attributes::Domain,
        target: Id,
        source: Id,
        blend: Option<[(Id, f32); 2]>,
    ) {
        let carry = self
            .mesh
            .attrs
            .propagate_caller(domain, target, source, blend);
        if carry.unpropagated > 0 {
            self.sink
                .record_unpropagated_attribute_values(carry.unpropagated);
        }
    }

    /// Restores captured caller-defined values onto the replacing element.
    pub(crate) fn restore_caller_layers(
        &mut self,
        domain: crate::attributes::Domain,
        target: Id,
        captured: &crate::attributes::CallerValues,
    ) {
        let carry = self.mesh.attrs.restore_caller(domain, target, captured);
        if carry.unpropagated > 0 {
            self.sink
                .record_unpropagated_attribute_values(carry.unpropagated);
        }
    }

    /// Finishes this eager edit scope.
    ///
    /// Finishing always increments [`Mesh::revision`](crate::Mesh::revision)
    /// exactly once, even when no mesh fields were changed. If this session was
    /// created through [`Mesh::edit_with`](crate::Mesh::edit_with), the sink
    /// output is returned here.
    #[must_use]
    pub fn finish(self) -> S::Output {
        let Self {
            mesh,
            outgoing_index: _,
            boundary_index: _,
            outgoing_index_valid: _,
            sink,
        } = self;
        mesh.revision = mesh
            .revision
            .checked_add(1)
            .expect("mesh revision overflowed u64");
        sink.finish()
    }
}
