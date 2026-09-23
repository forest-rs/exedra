// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::attributes::{AttrKey, Domain, LayerValue};

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

    /// Returns the explicit vertex sharpness override, when present.
    #[must_use]
    pub fn vertex_sharpness(&self, vertex: VertexId) -> Option<f32> {
        self.mesh.vertex_sharpness(vertex)
    }

    pub(crate) fn set_vertex_sharpness_impl(&mut self, vertex: VertexId, sharpness: f32) -> bool {
        let updated = self.mesh.set_vertex_sharpness(vertex, sharpness);
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

    /// Returns the authored corner normal override for `corner`, when present.
    #[must_use]
    pub fn corner_normal_override(&self, corner: CornerId) -> Option<[f32; 3]> {
        self.mesh
            .attrs()
            .sparse(attr::CORNER_NORMAL_OVERRIDE)
            .and_then(|layer| layer.get(corner.as_id()).copied())
    }

    pub(crate) fn set_corner_normal_override_impl(
        &mut self,
        corner: CornerId,
        normal: Option<[f32; 3]>,
    ) -> bool {
        if self.mesh.half_edges.get(corner.as_id()).is_none() {
            return false;
        }
        if self
            .mesh
            .attrs()
            .sparse(attr::CORNER_NORMAL_OVERRIDE)
            .is_none()
        {
            let _ = self
                .mesh
                .attrs_mut()
                .define_sparse(attr::CORNER_NORMAL_OVERRIDE);
        }
        let updated = self
            .mesh
            .attrs_mut()
            .sparse_mut(attr::CORNER_NORMAL_OVERRIDE)
            .is_some_and(|layer| {
                match normal {
                    Some(value) => layer.set(corner.as_id(), value),
                    None => {
                        let _ = layer.remove(corner.as_id());
                    }
                }
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

/// Outcome of a generic attribute write, mapped to public errors by the ops.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum AttributeWrite {
    Written,
    NotLive,
    Reserved,
    TypeMismatch,
}

impl<S: ChangeSink> EditSession<'_, S> {
    /// True when `id` is live in `domain`. Half-edges include boundary
    /// (OUTSIDE-face) half-edges, matching [`Self::set_corner_uv_impl`].
    fn element_live(&self, domain: Domain, id: Id) -> bool {
        match domain {
            Domain::Vertex => self.mesh.vertices.get(id).is_some(),
            Domain::Face => self.mesh.faces.get(id).is_some(),
            Domain::HalfEdge => self.mesh.half_edges.get(id).is_some(),
        }
    }

    fn mark_element_dirty(&mut self, domain: Domain, id: Id) {
        match domain {
            Domain::Vertex => self.sink.mark_vertex_dirty(VertexId::from(id)),
            Domain::Face => self.sink.mark_face_dirty(FaceId::from(id)),
            Domain::HalfEdge => self.sink.mark_corner_dirty(CornerId::from(id)),
        }
    }

    pub(crate) fn set_attribute_impl<T: LayerValue>(
        &mut self,
        key: AttrKey<T>,
        id: Id,
        value: T,
    ) -> AttributeWrite {
        if attr::is_reserved(key.domain(), key.name()) {
            return AttributeWrite::Reserved;
        }
        if !self.element_live(key.domain(), id) {
            return AttributeWrite::NotLive;
        }
        let attrs = self.mesh.attrs_mut();
        if let Some(layer) = attrs.dense_mut(key) {
            let set = layer.set(id, value);
            debug_assert!(set, "dense layer must cover every live slot");
        } else if let Some(layer) = attrs.sparse_mut(key) {
            layer.set(id, value);
        } else if attrs.define_sparse(key).is_ok() {
            attrs
                .sparse_mut(key)
                .expect("sparse layer was just defined")
                .set(id, value);
        } else {
            return AttributeWrite::TypeMismatch;
        }
        self.mark_element_dirty(key.domain(), id);
        AttributeWrite::Written
    }

    pub(crate) fn clear_attribute_impl<T: LayerValue>(
        &mut self,
        key: AttrKey<T>,
        id: Id,
    ) -> AttributeWrite {
        if attr::is_reserved(key.domain(), key.name()) {
            return AttributeWrite::Reserved;
        }
        if !self.element_live(key.domain(), id) {
            return AttributeWrite::NotLive;
        }
        let attrs = self.mesh.attrs_mut();
        if let Some(layer) = attrs.dense_mut(key) {
            let default = layer.default().clone();
            let set = layer.set(id, default);
            debug_assert!(set, "dense layer must cover every live slot");
        } else if let Some(layer) = attrs.sparse_mut(key) {
            let _ = layer.remove(id);
        } else if attrs.layer(key.domain(), key.name()).is_some() {
            return AttributeWrite::TypeMismatch;
        } else {
            // No layer means no value to clear.
            return AttributeWrite::Written;
        }
        self.mark_element_dirty(key.domain(), id);
        AttributeWrite::Written
    }
}
