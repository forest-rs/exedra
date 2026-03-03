// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit mesh transactions and deterministic change summaries.

use alloc::vec::Vec;

use understory_dirty::{Channel, DirtySet as UnderstoryDirtySet};

use crate::{CornerId, FaceId, HalfEdgeId, Id, Mesh, VertexId};

const DIRTY_FACES_CHANNEL: Channel = Channel::new(0);
const DIRTY_VERTICES_CHANNEL: Channel = Channel::new(1);
const DIRTY_CORNERS_CHANNEL: Channel = Channel::new(2);

/// Conservative dirty summary for incremental systems.
///
/// This wraps [`understory_dirty`] primitives while exposing typed Exedra
/// domains. Deterministic snapshot accessors allocate and sort on each call.
#[derive(Clone, Debug, Default)]
pub struct DirtySet {
    inner: UnderstoryDirtySet<Id>,
}

impl DirtySet {
    /// Creates an empty dirty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when no dirty items exist in any channel.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns a mutation generation counter from the underlying dirty set.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner.generation()
    }

    /// Marks a face as dirty.
    pub fn mark_face(&mut self, face: FaceId) {
        self.inner.mark(face.as_id(), DIRTY_FACES_CHANNEL);
    }

    /// Marks a vertex as dirty.
    pub fn mark_vertex(&mut self, vertex: VertexId) {
        self.inner.mark(vertex.as_id(), DIRTY_VERTICES_CHANNEL);
    }

    /// Marks a corner as dirty.
    pub fn mark_corner(&mut self, corner: CornerId) {
        self.inner.mark(corner.as_id(), DIRTY_CORNERS_CHANNEL);
    }

    /// Returns dirty faces without ordering guarantees and without allocation.
    pub fn dirty_faces_unordered(&self) -> impl Iterator<Item = FaceId> + '_ {
        self.inner.iter(DIRTY_FACES_CHANNEL).map(FaceId::from)
    }

    /// Returns dirty vertices without ordering guarantees and without allocation.
    pub fn dirty_vertices_unordered(&self) -> impl Iterator<Item = VertexId> + '_ {
        self.inner.iter(DIRTY_VERTICES_CHANNEL).map(VertexId::from)
    }

    /// Returns dirty corners without ordering guarantees and without allocation.
    pub fn dirty_corners_unordered(&self) -> impl Iterator<Item = CornerId> + '_ {
        self.inner.iter(DIRTY_CORNERS_CHANNEL).map(CornerId::from)
    }

    /// Returns sorted dirty faces in deterministic ID order.
    #[must_use]
    pub fn dirty_faces(&self) -> Vec<FaceId> {
        let mut ids = self.dirty_faces_unordered().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    /// Returns sorted dirty vertices in deterministic ID order.
    #[must_use]
    pub fn dirty_vertices(&self) -> Vec<VertexId> {
        let mut ids = self.dirty_vertices_unordered().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    /// Returns sorted dirty corners in deterministic ID order.
    #[must_use]
    pub fn dirty_corners(&self) -> Vec<CornerId> {
        let mut ids = self.dirty_corners_unordered().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }
}

/// Deterministic summary of mesh changes produced by a committed transaction.
#[derive(Clone, Debug, Default)]
pub struct ChangeSet {
    /// Conservative invalidation summary.
    pub dirty: DirtySet,
    /// Created vertices in deterministic ID order.
    pub created_vertices: Vec<VertexId>,
    /// Created half-edges in deterministic ID order.
    pub created_half_edges: Vec<HalfEdgeId>,
    /// Created faces in deterministic ID order.
    pub created_faces: Vec<FaceId>,
    /// Deleted vertices in deterministic ID order.
    pub deleted_vertices: Vec<VertexId>,
    /// Deleted half-edges in deterministic ID order.
    pub deleted_half_edges: Vec<HalfEdgeId>,
    /// Deleted faces in deterministic ID order.
    pub deleted_faces: Vec<FaceId>,
}

/// Single-writer transaction over a mesh.
///
/// Mutations are applied eagerly to the underlying mesh. Dropping a transaction
/// does not roll back mesh changes; it only discards accumulated bookkeeping.
#[derive(Debug)]
pub struct Txn<'a> {
    mesh: &'a mut Mesh,
    dirty: DirtySet,
    created_vertices: Vec<VertexId>,
    created_half_edges: Vec<HalfEdgeId>,
    created_faces: Vec<FaceId>,
    deleted_vertices: Vec<VertexId>,
    deleted_half_edges: Vec<HalfEdgeId>,
    deleted_faces: Vec<FaceId>,
}

impl Mesh {
    /// Begins a new transaction borrowing this mesh mutably.
    ///
    /// Mutations performed through the transaction update the mesh immediately.
    /// Call [`Txn::commit`] to materialize a [`ChangeSet`], or [`Txn::abort`]
    /// to explicitly discard bookkeeping without rollback.
    pub fn begin(&mut self) -> Txn<'_> {
        Txn {
            mesh: self,
            dirty: DirtySet::new(),
            created_vertices: Vec::new(),
            created_half_edges: Vec::new(),
            created_faces: Vec::new(),
            deleted_vertices: Vec::new(),
            deleted_half_edges: Vec::new(),
            deleted_faces: Vec::new(),
        }
    }
}

impl Txn<'_> {
    /// Returns an immutable view of the mesh being edited.
    #[must_use]
    pub fn mesh(&self) -> &Mesh {
        self.mesh
    }

    /// Returns a mutable view of the mesh being edited.
    pub fn mesh_mut(&mut self) -> &mut Mesh {
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
    #[must_use]
    pub fn commit(mut self) -> ChangeSet {
        sort_dedup(&mut self.created_vertices);
        sort_dedup(&mut self.created_half_edges);
        sort_dedup(&mut self.created_faces);
        sort_dedup(&mut self.deleted_vertices);
        sort_dedup(&mut self.deleted_half_edges);
        sort_dedup(&mut self.deleted_faces);

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

fn sort_dedup<T: Ord>(values: &mut Vec<T>) {
    values.sort_unstable();
    values.dedup();
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::num::NonZeroU32;

    use crate::{Face, HalfEdge, Id};

    use super::*;

    #[test]
    fn txn_add_vertex_commits_created_and_dirty() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.begin();
        let vertex = txn.add_vertex([1.0, 2.0, 3.0]);
        let changes = txn.commit();

        assert_eq!(changes.created_vertices, vec![vertex]);
        assert_eq!(changes.dirty.dirty_vertices(), vec![vertex]);
        assert!(changes.dirty.dirty_faces().is_empty());
        assert!(changes.dirty.dirty_corners().is_empty());
    }

    #[test]
    fn txn_set_vertex_position_marks_vertex_dirty() {
        let mut mesh = Mesh::new();
        let vertex = mesh.add_vertex([0.0, 0.0, 0.0]);
        let mut txn = mesh.begin();
        assert!(txn.set_vertex_position(vertex, [4.0, 5.0, 6.0]));
        let changes = txn.commit();

        assert_eq!(changes.dirty.dirty_vertices(), vec![vertex]);
        assert_eq!(mesh.vertex_position(vertex), Some(&[4.0, 5.0, 6.0]));
    }

    #[test]
    fn txn_commit_sorts_and_dedups_change_lists() {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([2.0, 0.0, 0.0]);
        let f0 = FaceId::from(Id::new(3, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        let h0 = HalfEdgeId::from(Id::new(6, NonZeroU32::MIN));
        let h1 = HalfEdgeId::from(Id::new(2, NonZeroU32::MIN));

        let mut txn = mesh.begin();
        txn.record_deleted_vertex(v2);
        txn.record_deleted_vertex(v0);
        txn.record_deleted_vertex(v2);
        txn.record_created_face(f0);
        txn.record_created_face(f1);
        txn.record_created_face(f1);
        txn.record_created_half_edge(h0);
        txn.record_created_half_edge(h1);
        txn.record_created_half_edge(h1);
        txn.mark_corner_dirty(h1);
        txn.mark_face_dirty(f0);
        let changes = txn.commit();

        assert_eq!(changes.deleted_vertices, vec![v0, v2]);
        assert_eq!(changes.created_faces, vec![f1, f0]);
        assert_eq!(changes.created_half_edges, vec![h1, h0]);
        assert_eq!(changes.dirty.dirty_faces(), vec![f0]);
        assert_eq!(changes.dirty.dirty_vertices(), vec![v0, v2]);
        assert_eq!(changes.dirty.dirty_corners(), vec![h1]);

        let _ = v1;
    }

    #[test]
    fn dirty_set_generation_changes_on_marks() {
        let mut dirty = DirtySet::new();
        let initial = dirty.generation();
        let face = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let vertex = VertexId::from(Id::new(1, NonZeroU32::MIN));
        let corner = CornerId::from(Id::new(2, NonZeroU32::MIN));

        dirty.mark_face(face);
        dirty.mark_vertex(vertex);
        dirty.mark_corner(corner);

        assert!(dirty.generation() >= initial + 3);
        assert_eq!(dirty.dirty_faces(), vec![face]);
        assert_eq!(dirty.dirty_vertices(), vec![vertex]);
        assert_eq!(dirty.dirty_corners(), vec![corner]);
        assert!(dirty.dirty_faces_unordered().any(|id| id == face));
        assert!(dirty.dirty_vertices_unordered().any(|id| id == vertex));
        assert!(dirty.dirty_corners_unordered().any(|id| id == corner));
    }

    #[test]
    fn txn_mesh_view_reflects_mutations() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.begin();
        let _ = txn.add_vertex([0.0, 0.0, 0.0]);
        assert_eq!(txn.mesh().vertices().count(), 1);
        let _ = txn.commit();
    }

    #[test]
    fn txn_abort_keeps_eager_mesh_mutations() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.begin();
        let vertex = txn.add_vertex([0.0, 1.0, 2.0]);
        txn.abort();

        assert_eq!(mesh.vertex_position(vertex), Some(&[0.0, 1.0, 2.0]));
    }

    #[test]
    fn txn_can_record_deleted_topology_ids() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.begin();
        let face = FaceId::from(Id::new(5, NonZeroU32::MIN));
        let edge = HalfEdgeId::from(Id::new(6, NonZeroU32::MIN));
        let vertex = VertexId::from(Id::new(7, NonZeroU32::MIN));
        txn.record_deleted_face(face);
        txn.record_deleted_half_edge(edge);
        txn.record_deleted_vertex(vertex);
        let changes = txn.commit();

        assert_eq!(changes.deleted_faces, vec![face]);
        assert_eq!(changes.deleted_half_edges, vec![edge]);
        assert_eq!(changes.deleted_vertices, vec![vertex]);
        assert_eq!(changes.dirty.dirty_faces(), vec![face]);
        assert_eq!(changes.dirty.dirty_corners(), vec![edge]);
        assert_eq!(changes.dirty.dirty_vertices(), vec![vertex]);
    }

    #[test]
    fn txn_change_lists_are_deterministic_from_unsorted_records() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.begin();

        let face_a = FaceId::from(Id::new(9, NonZeroU32::MIN));
        let face_b = FaceId::from(Id::new(4, NonZeroU32::MIN));
        let edge_a = HalfEdgeId::from(Id::new(8, NonZeroU32::MIN));
        let edge_b = HalfEdgeId::from(Id::new(1, NonZeroU32::MIN));
        txn.record_created_face(face_a);
        txn.record_created_face(face_b);
        txn.record_created_half_edge(edge_a);
        txn.record_created_half_edge(edge_b);
        let changes = txn.commit();

        assert_eq!(changes.created_faces, vec![face_b, face_a]);
        assert_eq!(changes.created_half_edges, vec![edge_b, edge_a]);
    }

    #[test]
    fn txn_bookkeeping_does_not_require_live_topology_ids() {
        let mut mesh = Mesh::new();
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 3,
        }));
        let half_edge = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: VertexId::new(0, NonZeroU32::MIN),
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        let mut txn = mesh.begin();
        txn.record_created_face(face);
        txn.record_created_half_edge(half_edge);
        let changes = txn.commit();

        assert_eq!(changes.created_faces, vec![face]);
        assert_eq!(changes.created_half_edges, vec![half_edge]);
    }
}
