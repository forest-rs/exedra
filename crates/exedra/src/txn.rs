// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit mesh transactions and deterministic change summaries.

use alloc::vec::Vec;
use core::fmt;

use understory_dirty::{Channel, DirtySet as UnderstoryDirtySet};

use crate::sorted_merge::for_each_count_join;
use crate::{CornerId, FaceId, HalfEdge, HalfEdgeId, Id, Mesh, VertexId, attr};

const DIRTY_FACES_CHANNEL: Channel = Channel::new(0);
const DIRTY_VERTICES_CHANNEL: Channel = Channel::new(1);
const DIRTY_CORNERS_CHANNEL: Channel = Channel::new(2);

/// Conservative dirty summary for incremental systems.
///
/// This wraps [`understory_dirty`] primitives while exposing typed Exedra
/// domains. The primary consumption path is deterministic drain operations.
///
/// Most callers consume this via [`ChangeSet::dirty`] after
/// [`Txn::commit`](crate::Txn::commit).
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

    /// Returns `true` when the face channel has dirty IDs.
    #[must_use]
    pub fn has_dirty_faces(&self) -> bool {
        self.inner.has_dirty(DIRTY_FACES_CHANNEL)
    }

    /// Returns `true` when the vertex channel has dirty IDs.
    #[must_use]
    pub fn has_dirty_vertices(&self) -> bool {
        self.inner.has_dirty(DIRTY_VERTICES_CHANNEL)
    }

    /// Returns `true` when the corner channel has dirty IDs.
    #[must_use]
    pub fn has_dirty_corners(&self) -> bool {
        self.inner.has_dirty(DIRTY_CORNERS_CHANNEL)
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

    /// Drains dirty faces in deterministic ID order into `out`.
    pub fn drain_faces_into(&mut self, out: &mut Vec<FaceId>) {
        drain_sorted_into(&mut self.inner, DIRTY_FACES_CHANNEL, out);
    }

    /// Drains dirty vertices in deterministic ID order into `out`.
    pub fn drain_vertices_into(&mut self, out: &mut Vec<VertexId>) {
        drain_sorted_into(&mut self.inner, DIRTY_VERTICES_CHANNEL, out);
    }

    /// Drains dirty corners in deterministic ID order into `out`.
    pub fn drain_corners_into(&mut self, out: &mut Vec<CornerId>) {
        drain_sorted_into(&mut self.inner, DIRTY_CORNERS_CHANNEL, out);
    }
}

/// Deterministic summary of mesh changes produced by a committed transaction.
///
/// Returned by [`Txn::commit`] and by convenience wrappers such as
/// [`Mesh::delete_faces`].
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

/// Face-deletion behavior for isolated vertices.
///
/// Passed to [`Mesh::delete_faces`] or [`Txn::delete_faces`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum DeletePolicy {
    /// Remove isolated vertices after face deletion.
    #[default]
    CleanupIsolated,
    /// Keep isolated vertices with `out = HalfEdgeId::INVALID`.
    KeepIsolated,
}

/// Structured face-deletion error from [`Mesh::delete_faces`] and
/// [`Txn::delete_faces`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DeleteFacesError {
    /// Input face list must be sorted and deduplicated.
    NonCanonicalFaceSet,
    /// `FaceId::OUTSIDE` is not a deletable interior face.
    OutsideFaceNotAllowed,
    /// Face ID is stale/dead.
    FaceNotLive {
        /// Stale face index.
        face: u32,
    },
    /// Post-delete boundary continuation would be ambiguous at a vertex.
    BoundaryContinuationAmbiguous {
        /// Vertex where boundary fan continuity fails.
        vertex: u32,
        /// Number of outgoing boundary candidates after deletion.
        outgoing: usize,
        /// Number of incoming boundary edges after deletion.
        incoming: usize,
    },
}

impl fmt::Display for DeleteFacesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonCanonicalFaceSet => f.write_str("face set must be sorted and deduplicated"),
            Self::OutsideFaceNotAllowed => f.write_str("FaceId::OUTSIDE cannot be deleted"),
            Self::FaceNotLive { face } => write!(f, "face is not live: {face}"),
            Self::BoundaryContinuationAmbiguous {
                vertex,
                outgoing,
                incoming,
            } => write!(
                f,
                "boundary continuation ambiguous at vertex {vertex}: outgoing={outgoing}, incoming={incoming}"
            ),
        }
    }
}

impl core::error::Error for DeleteFacesError {}

/// Single-writer transaction over a mesh.
///
/// Mutations are applied eagerly to the underlying mesh. Dropping a transaction
/// does not roll back mesh changes; it only discards accumulated bookkeeping.
///
/// Acquire via [`Mesh::begin`], apply mutating operations, then finish with
/// [`Txn::commit`] (or [`Txn::abort`] to drop bookkeeping only).
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

    /// Deletes a canonical set of interior faces in one committed transaction.
    ///
    /// This is a convenience wrapper over [`Txn::delete_faces`] + [`Txn::commit`].
    ///
    /// Note: transactions in Exedra are eager. This returns only precondition
    /// errors before mutation begins. If internal boundary restitching cannot
    /// produce a valid continuation, this function panics with diagnostics.
    pub fn delete_faces(
        &mut self,
        faces: &[FaceId],
        policy: DeletePolicy,
    ) -> Result<ChangeSet, DeleteFacesError> {
        let mut txn = self.begin();
        txn.delete_faces(faces, policy)?;
        Ok(txn.commit())
    }
}

impl Txn<'_> {
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

    /// Returns explicit sharpness state for an undirected edge.
    #[must_use]
    pub fn edge_sharpness(&self, half_edge: HalfEdgeId) -> Option<bool> {
        self.mesh.edge_sharpness(half_edge)
    }

    /// Sets explicit sharpness state for an undirected edge.
    ///
    /// Returns `true` when `half_edge` is live and writable.
    pub fn set_edge_sharpness(&mut self, half_edge: HalfEdgeId, sharp: bool) -> bool {
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

    /// Deletes interior faces from the mesh.
    ///
    /// `faces` must be canonical (sorted, deduplicated), contain only live
    /// interior face IDs, and never include [`FaceId::OUTSIDE`].
    ///
    /// Note: transactions in Exedra are eager. This returns only precondition
    /// errors before mutation begins. If internal boundary restitching cannot
    /// produce a valid continuation, this function panics with diagnostics.
    pub fn delete_faces(
        &mut self,
        faces: &[FaceId],
        policy: DeletePolicy,
    ) -> Result<(), DeleteFacesError> {
        if !is_canonical_face_set(faces) {
            return Err(DeleteFacesError::NonCanonicalFaceSet);
        }
        for &face in faces {
            if face == FaceId::OUTSIDE {
                return Err(DeleteFacesError::OutsideFaceNotAllowed);
            }
            if self.mesh.faces.get(face.as_id()).is_none() {
                return Err(DeleteFacesError::FaceNotLive { face: face.index() });
            }
        }
        if faces.is_empty() {
            return Ok(());
        }

        let mut dirty_faces = Vec::<FaceId>::new();
        let mut dirty_vertices = Vec::<VertexId>::new();
        let mut boundary_replacements = Vec::<HalfEdgeId>::new();
        let mut deleted_half_edges = Vec::<HalfEdgeId>::new();

        for &face in faces {
            let loop_edges = self.mesh.face_loop(face).collect::<Vec<_>>();
            for half_edge in loop_edges {
                let twin = self
                    .mesh
                    .twin(half_edge)
                    .expect("valid mesh must provide a twin for each half-edge");
                let twin_face = self
                    .mesh
                    .face(twin)
                    .expect("valid mesh must provide an owning face for each half-edge");
                deleted_half_edges.push(half_edge);
                if twin_face == FaceId::OUTSIDE {
                    deleted_half_edges.push(twin);
                } else if !contains_face(faces, twin_face) {
                    boundary_replacements.push(twin);
                    dirty_faces.push(twin_face);
                }
                collect_half_edge_vertices(self.mesh, half_edge, &mut dirty_vertices);
            }
        }
        sort_dedup(&mut deleted_half_edges);
        sort_dedup(&mut boundary_replacements);
        sort_dedup(&mut dirty_faces);
        sort_dedup(&mut dirty_vertices);
        preflight_boundary_continuation(self.mesh, &deleted_half_edges, &boundary_replacements)?;

        for &face in faces {
            let removed = self.mesh.faces.remove(face.as_id());
            debug_assert!(removed.is_some(), "validated face should remove");
            self.record_deleted_face(face);
        }
        for half_edge in deleted_half_edges {
            let removed = self.mesh.half_edges.remove(half_edge.as_id());
            debug_assert!(removed.is_some(), "validated half-edge should remove");
            self.record_deleted_half_edge(half_edge);
            clear_deleted_corner_attrs(self.mesh, half_edge);
        }

        for twin in boundary_replacements {
            let from = self
                .mesh
                .from_vertex(twin)
                .expect("surviving interior half-edge must have an origin");
            let boundary = HalfEdgeId::from(self.mesh.half_edges.insert(HalfEdge {
                to: from,
                face: FaceId::OUTSIDE,
                next: HalfEdgeId::INVALID,
                twin,
            }));
            self.mesh
                .half_edges
                .get_mut(twin.as_id())
                .expect("surviving interior half-edge must be live")
                .twin = boundary;
            self.record_created_half_edge(boundary);
            collect_half_edge_vertices(self.mesh, boundary, &mut dirty_vertices);
        }
        sort_dedup(&mut dirty_vertices);

        stitch_outside_loops(self.mesh);

        for face in dirty_faces {
            self.mark_face_dirty(face);
            let corners = self.mesh.face_loop(face).collect::<Vec<_>>();
            for corner in corners {
                self.mark_corner_dirty(corner);
            }
        }
        let mut isolated = Vec::<VertexId>::new();
        let use_global_index =
            should_use_global_outgoing_index(dirty_vertices.len(), self.mesh.half_edges.len());
        let outgoing_index = use_global_index.then(|| build_outgoing_index(self.mesh));
        for &vertex in &dirty_vertices {
            let new_out = outgoing_index
                .as_deref()
                .and_then(|index| find_outgoing_half_edge(index, vertex))
                .or_else(|| find_outgoing_half_edge_linear_scan(self.mesh, vertex));
            let Some(record) = self.mesh.vertices.get_mut(vertex.as_id()) else {
                continue;
            };
            let previous = record.out;
            record.out = new_out.unwrap_or(HalfEdgeId::INVALID);
            if record.out != previous {
                self.mark_vertex_dirty(vertex);
            }
            if new_out.is_none() {
                isolated.push(vertex);
            }
        }
        for vertex in dirty_vertices {
            self.mark_vertex_dirty(vertex);
        }

        if policy == DeletePolicy::CleanupIsolated {
            for vertex in isolated {
                if self.mesh.vertices.remove(vertex.as_id()).is_some() {
                    self.record_deleted_vertex(vertex);
                }
            }
        }
        Ok(())
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

fn sort_dedup<T: Ord>(values: &mut Vec<T>) {
    values.sort_unstable();
    values.dedup();
}

fn drain_sorted_into<T>(dirty: &mut UnderstoryDirtySet<Id>, channel: Channel, out: &mut Vec<T>)
where
    T: From<Id> + Ord,
{
    out.clear();
    out.extend(dirty.drain(channel).map(T::from));
    out.sort_unstable();
}

fn is_canonical_face_set(faces: &[FaceId]) -> bool {
    faces.windows(2).all(|pair| pair[0] < pair[1])
}

fn contains_face(faces: &[FaceId], target: FaceId) -> bool {
    faces.binary_search(&target).is_ok()
}

fn collect_half_edge_vertices(mesh: &Mesh, half_edge: HalfEdgeId, out: &mut Vec<VertexId>) {
    if let Some((from, to)) = half_edge_vertices(mesh, half_edge) {
        out.push(from);
        out.push(to);
    }
}

fn half_edge_vertices(mesh: &Mesh, half_edge: HalfEdgeId) -> Option<(VertexId, VertexId)> {
    let to = mesh.to_vertex(half_edge)?;
    let twin = mesh.twin(half_edge)?;
    let from = mesh.to_vertex(twin)?;
    Some((from, to))
}

fn build_outgoing_index(mesh: &Mesh) -> Vec<(VertexId, HalfEdgeId)> {
    let mut pairs = mesh
        .half_edges
        .iter()
        .filter_map(|(id, _)| {
            let half_edge = HalfEdgeId::from(id);
            half_edge_vertices(mesh, half_edge).map(|(from, _)| (from, half_edge))
        })
        .collect::<Vec<_>>();
    pairs.sort_unstable_by_key(|(vertex, half_edge)| (*vertex, *half_edge));
    pairs.dedup_by_key(|(vertex, _)| *vertex);
    pairs
}

fn find_outgoing_half_edge_linear_scan(mesh: &Mesh, vertex: VertexId) -> Option<HalfEdgeId> {
    mesh.half_edges.iter().find_map(|(id, _)| {
        let half_edge = HalfEdgeId::from(id);
        (half_edge_vertices(mesh, half_edge).is_some_and(|(from, _)| from == vertex))
            .then_some(half_edge)
    })
}

fn find_outgoing_half_edge(
    outgoing_index: &[(VertexId, HalfEdgeId)],
    vertex: VertexId,
) -> Option<HalfEdgeId> {
    outgoing_index
        .binary_search_by_key(&vertex, |(candidate, _)| *candidate)
        .ok()
        .map(|position| outgoing_index[position].1)
}

fn should_use_global_outgoing_index(affected_vertices: usize, total_half_edges: usize) -> bool {
    if affected_vertices == 0 || total_half_edges == 0 {
        return false;
    }
    // Heuristic: switch to global index once the affected set is at least
    // about 1/8 of total half-edges; this avoids expensive repeated linear
    // scans near crossover cases.
    affected_vertices.saturating_mul(8) > total_half_edges
}

fn clear_deleted_corner_attrs(mesh: &mut Mesh, half_edge: HalfEdgeId) {
    if let Some(layer) = mesh.attrs_mut().sparse_mut(attr::CORNER_UV) {
        let _ = layer.remove(half_edge.as_id());
    }
    if let Some(layer) = mesh.attrs_mut().sparse_mut(attr::EDGE_SEAM) {
        let _ = layer.remove(half_edge.as_id());
    }
    if let Some(layer) = mesh.attrs_mut().sparse_mut(attr::EDGE_SHARPNESS) {
        let _ = layer.remove(half_edge.as_id());
    }
}

fn preflight_boundary_continuation(
    mesh: &Mesh,
    deleted_half_edges: &[HalfEdgeId],
    boundary_replacements: &[HalfEdgeId],
) -> Result<(), DeleteFacesError> {
    let mut outgoing = Vec::<VertexId>::new();
    let mut incoming = Vec::<VertexId>::new();

    for (id, edge) in mesh.half_edges.iter() {
        let half_edge = HalfEdgeId::from(id);
        if edge.face != FaceId::OUTSIDE {
            continue;
        }
        if deleted_half_edges.binary_search(&half_edge).is_ok() {
            continue;
        }
        let to = mesh
            .to_vertex(half_edge)
            .expect("boundary half-edge must have destination");
        let start = mesh
            .twin(half_edge)
            .and_then(|twin| mesh.to_vertex(twin))
            .expect("boundary half-edge must have twin with destination");
        outgoing.push(start);
        incoming.push(to);
    }

    for &twin in boundary_replacements {
        let start = mesh
            .to_vertex(twin)
            .expect("replacement twin must have destination");
        let to = mesh
            .from_vertex(twin)
            .expect("replacement twin must have origin");
        outgoing.push(start);
        incoming.push(to);
    }

    let outgoing_counts = count_vertices(outgoing);
    let incoming_counts = count_vertices(incoming);

    let mut mismatch = None;
    for_each_count_join(
        &outgoing_counts,
        &incoming_counts,
        |vertex, out_count, in_count| {
            if out_count != 1 || in_count != 1 {
                let _ = mismatch.get_or_insert(DeleteFacesError::BoundaryContinuationAmbiguous {
                    vertex: vertex.index(),
                    outgoing: out_count,
                    incoming: in_count,
                });
            }
        },
    );
    mismatch.map_or(Ok(()), Err)
}

fn count_vertices(mut values: Vec<VertexId>) -> Vec<(VertexId, usize)> {
    values.sort_unstable();
    let mut counts = Vec::<(VertexId, usize)>::new();
    for vertex in values {
        if let Some((last, count)) = counts.last_mut()
            && *last == vertex
        {
            *count = count.saturating_add(1);
            continue;
        }
        counts.push((vertex, 1));
    }
    counts
}

fn stitch_outside_loops(mesh: &mut Mesh) {
    let boundary = mesh
        .half_edges
        .iter()
        .filter_map(|(id, edge)| (edge.face == FaceId::OUTSIDE).then_some(HalfEdgeId::from(id)))
        .collect::<Vec<_>>();
    // TODO(exe-8w2z): This currently re-stitches all OUTSIDE loops globally.
    // We can scope to affected boundary components once delete kernels track
    // local boundary frontiers.

    // Build a deterministic index of boundary starts:
    // start(boundary_h) == to(twin(boundary_h)).
    let mut starts = boundary
        .iter()
        .copied()
        .map(|boundary_half_edge| {
            let twin = mesh
                .twin(boundary_half_edge)
                .expect("boundary half-edge must have twin");
            let start = mesh
                .to_vertex(twin)
                .expect("boundary twin must have destination vertex");
            (start, boundary_half_edge)
        })
        .collect::<Vec<_>>();
    starts.sort_unstable_by_key(|(start, boundary_half_edge)| (*start, *boundary_half_edge));

    for half_edge in &boundary {
        let to = mesh
            .to_vertex(*half_edge)
            .expect("boundary half-edge must have destination vertex");

        let range = equal_range_by_vertex(&starts, to);
        let candidates = range.end.saturating_sub(range.start);
        if candidates != 1 {
            panic!(
                "mesh topology corruption: OUTSIDE stitch failed at vertex {} ({} candidates)",
                to.index(),
                candidates
            );
        }
        let next = starts[range.start].1;
        mesh.half_edges
            .get_mut(half_edge.as_id())
            .expect("boundary half-edge must be live")
            .next = next;
    }
}

fn equal_range_by_vertex(
    starts: &[(VertexId, HalfEdgeId)],
    vertex: VertexId,
) -> core::ops::Range<usize> {
    let lower = starts.partition_point(|(candidate, _)| *candidate < vertex);
    let upper = starts.partition_point(|(candidate, _)| *candidate <= vertex);
    lower..upper
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use crate::{Face, HalfEdge, Id, MeshBuilder};

    use super::*;

    fn drained_faces(dirty: &mut DirtySet) -> Vec<FaceId> {
        let mut values = Vec::new();
        dirty.drain_faces_into(&mut values);
        values
    }

    fn drained_vertices(dirty: &mut DirtySet) -> Vec<VertexId> {
        let mut values = Vec::new();
        dirty.drain_vertices_into(&mut values);
        values
    }

    fn drained_corners(dirty: &mut DirtySet) -> Vec<CornerId> {
        let mut values = Vec::new();
        dirty.drain_corners_into(&mut values);
        values
    }

    fn count_boundary_loops(mesh: &Mesh) -> usize {
        let mut visited = vec![false; mesh.half_edges.slot_count()];
        let mut loops = 0_usize;
        for (id, half_edge) in mesh.half_edges.iter() {
            if half_edge.face != FaceId::OUTSIDE || visited[id.index() as usize] {
                continue;
            }
            loops += 1;
            let start = HalfEdgeId::from(id);
            let mut cursor = start;
            loop {
                visited[cursor.index() as usize] = true;
                cursor = mesh.next(cursor).expect("boundary next exists");
                if cursor == start {
                    break;
                }
            }
        }
        loops
    }

    fn closed_box_mesh() -> (Mesh, Vec<FaceId>) {
        let mut builder = MeshBuilder::new();
        for position in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ] {
            let _ = builder.push_vertex(position);
        }
        for loop_indices in [
            [0, 3, 2, 1], // bottom
            [4, 5, 6, 7], // top
            [0, 1, 5, 4], // front
            [1, 2, 6, 5], // right
            [2, 3, 7, 6], // back
            [3, 0, 4, 7], // left
        ] {
            builder.add_face(&loop_indices).expect("box face");
        }
        let built = builder.build().expect("box build");
        (built.mesh, built.face_ids)
    }

    #[test]
    fn txn_add_vertex_commits_created_and_dirty() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.begin();
        let vertex = txn.add_vertex([1.0, 2.0, 3.0]);
        let mut changes = txn.commit();

        assert_eq!(changes.created_vertices, vec![vertex]);
        assert_eq!(drained_vertices(&mut changes.dirty), vec![vertex]);
        assert!(drained_faces(&mut changes.dirty).is_empty());
        assert!(drained_corners(&mut changes.dirty).is_empty());
    }

    #[test]
    fn txn_set_vertex_position_marks_vertex_dirty() {
        let mut mesh = Mesh::new();
        let vertex = mesh.add_vertex([0.0, 0.0, 0.0]);
        let mut txn = mesh.begin();
        assert!(txn.set_vertex_position(vertex, [4.0, 5.0, 6.0]));
        let mut changes = txn.commit();

        assert_eq!(drained_vertices(&mut changes.dirty), vec![vertex]);
        assert_eq!(mesh.vertex_position(vertex), Some(&[4.0, 5.0, 6.0]));
    }

    #[test]
    fn txn_set_face_region_marks_face_dirty() {
        let built = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let mut mesh = built;
        let face = mesh.faces().next().expect("face should exist");

        let mut txn = mesh.begin();
        assert!(txn.set_face_region(face, 9));
        let mut changes = txn.commit();

        let region = mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .expect("face region layer should exist")
            .get(face.as_id())
            .copied();
        assert_eq!(region, Some(9));
        assert_eq!(drained_faces(&mut changes.dirty), vec![face]);
    }

    #[test]
    fn txn_set_corner_uv_writes_sparse_layer_and_marks_corner_dirty() {
        let built = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let mut mesh = built;
        let face = mesh.faces().next().expect("face should exist");
        let corner = mesh.face_loop(face).next().expect("corner should exist");

        let mut txn = mesh.begin();
        assert!(txn.set_corner_uv(corner, [0.25, 0.5]));
        assert_eq!(txn.corner_uv(corner), Some([0.25, 0.5]));
        let mut changes = txn.commit();
        assert_eq!(drained_corners(&mut changes.dirty), vec![corner]);
    }

    #[test]
    fn txn_set_edge_seam_round_trips_from_either_half_edge() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let shared = find_half_edge(&mesh, 0, 2).expect("shared half-edge should exist");
        let shared_twin = mesh.twin(shared).expect("shared edge should have twin");

        let mut txn = mesh.begin();
        assert_eq!(txn.edge_seam(shared), Some(false));
        assert!(txn.set_edge_seam(shared_twin, true));
        assert_eq!(txn.edge_seam(shared), Some(true));
        assert_eq!(txn.edge_seam(shared_twin), Some(true));
        let mut changes = txn.commit();

        let mut corners = drained_corners(&mut changes.dirty);
        corners.sort_unstable();
        let mut expected = vec![shared, shared_twin];
        expected.sort_unstable();
        assert_eq!(corners, expected);
    }

    #[test]
    fn txn_set_edge_sharpness_round_trips_from_either_half_edge() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let shared = find_half_edge(&mesh, 0, 2).expect("shared half-edge should exist");
        let shared_twin = mesh.twin(shared).expect("shared edge should have twin");

        let mut txn = mesh.begin();
        assert_eq!(txn.edge_sharpness(shared), Some(false));
        assert!(txn.set_edge_sharpness(shared_twin, true));
        assert_eq!(txn.edge_sharpness(shared), Some(true));
        assert_eq!(txn.edge_sharpness(shared_twin), Some(true));
        let mut changes = txn.commit();

        let mut corners = drained_corners(&mut changes.dirty);
        corners.sort_unstable();
        let mut expected = vec![shared, shared_twin];
        expected.sort_unstable();
        assert_eq!(corners, expected);
    }

    #[test]
    fn mesh_is_uv_discontinuous_checks_both_edge_endpoints() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let shared = find_half_edge(&mesh, 0, 2).expect("shared half-edge should exist");
        assert_eq!(mesh.is_uv_discontinuous(shared), Some(false));

        let h = shared;
        let t = mesh.twin(h).expect("twin should exist");
        let t_next = mesh.next(t).expect("next should exist");
        {
            let mut txn = mesh.begin();
            assert!(txn.set_corner_uv(h, [0.25, 0.0]));
            assert!(txn.set_corner_uv(t_next, [0.75, 0.0]));
            let _ = txn.commit();
        }
        assert_eq!(mesh.is_uv_discontinuous(shared), Some(true));
    }

    #[test]
    fn delete_faces_rejects_non_canonical_input() {
        let (mut mesh, faces) = closed_box_mesh();
        let err = mesh
            .delete_faces(&[faces[1], faces[0]], DeletePolicy::CleanupIsolated)
            .expect_err("non-canonical face set must fail");
        assert_eq!(err, DeleteFacesError::NonCanonicalFaceSet);
    }

    #[test]
    fn delete_single_box_face_creates_one_boundary_loop_and_valid_mesh() {
        let (mut mesh, faces) = closed_box_mesh();
        let deleted = faces[0];
        let changes = mesh
            .delete_faces(&[deleted], DeletePolicy::CleanupIsolated)
            .expect("delete should succeed");

        assert_eq!(changes.deleted_faces, vec![deleted]);
        assert_eq!(mesh.faces.len(), 5);
        assert_eq!(count_boundary_loops(&mesh), 1);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn delete_faces_keep_isolated_preserves_lonely_vertices() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let vertex_count = mesh.vertices.len();

        let changes = mesh
            .delete_faces(&[face], DeletePolicy::KeepIsolated)
            .expect("delete should succeed");
        assert_eq!(mesh.faces.len(), 0);
        assert_eq!(mesh.vertices.len(), vertex_count);
        assert!(changes.deleted_vertices.is_empty());
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn delete_two_adjacent_box_faces_merges_into_one_opening() {
        let (mut mesh, faces) = closed_box_mesh();
        let changes = mesh
            .delete_faces(&[faces[2], faces[3]], DeletePolicy::CleanupIsolated)
            .expect("delete should succeed");
        assert_eq!(changes.deleted_faces, vec![faces[2], faces[3]]);
        assert_eq!(mesh.faces.len(), 4);
        assert_eq!(count_boundary_loops(&mesh), 1);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn delete_two_non_adjacent_box_faces_creates_two_openings() {
        let (mut mesh, faces) = closed_box_mesh();
        let changes = mesh
            .delete_faces(&[faces[2], faces[4]], DeletePolicy::CleanupIsolated)
            .expect("delete should succeed");
        assert_eq!(changes.deleted_faces, vec![faces[2], faces[4]]);
        assert_eq!(mesh.faces.len(), 4);
        assert_eq!(count_boundary_loops(&mesh), 2);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn delete_faces_rejects_preflight_boundary_ambiguity_without_mutation() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 1.0],  // 0 top
                [0.0, 0.0, -1.0], // 1 bottom
                [1.0, 0.0, 0.0],  // 2
                [0.0, 1.0, 0.0],  // 3
                [-1.0, 0.0, 0.0], // 4
                [0.0, -1.0, 0.0], // 5
            ],
            &[
                [0, 2, 3],
                [0, 3, 4],
                [0, 4, 5],
                [0, 5, 2],
                [1, 3, 2],
                [1, 4, 3],
                [1, 5, 4],
                [1, 2, 5],
            ],
            &crate::mesh::BuildParams::default(),
        )
        .expect("octahedron build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let baseline_face_count = mesh.faces.len();
        let baseline_half_edge_count = mesh.half_edges.len();
        let baseline_vertex_count = mesh.vertices.len();
        let baseline_revision = mesh.revision();

        let err = mesh
            .delete_faces(&[faces[0], faces[2]], DeletePolicy::CleanupIsolated)
            .expect_err("preflight ambiguity should fail");
        assert!(matches!(
            err,
            DeleteFacesError::BoundaryContinuationAmbiguous { .. }
        ));
        assert_eq!(mesh.faces.len(), baseline_face_count);
        assert_eq!(mesh.half_edges.len(), baseline_half_edge_count);
        assert_eq!(mesh.vertices.len(), baseline_vertex_count);
        assert_eq!(mesh.revision(), baseline_revision);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn outgoing_index_strategy_prefers_localized_scans_for_small_edits() {
        assert!(!should_use_global_outgoing_index(1, 1_000));
        assert!(!should_use_global_outgoing_index(8, 10_000));
        assert!(!should_use_global_outgoing_index(0, 100));
    }

    #[test]
    fn outgoing_index_strategy_switches_to_global_for_large_affected_sets() {
        assert!(should_use_global_outgoing_index(130, 1_000));
        assert!(should_use_global_outgoing_index(1_300, 10_000));
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
        let mut changes = txn.commit();

        assert_eq!(changes.deleted_vertices, vec![v0, v2]);
        assert_eq!(changes.created_faces, vec![f1, f0]);
        assert_eq!(changes.created_half_edges, vec![h1, h0]);
        assert_eq!(drained_faces(&mut changes.dirty), vec![f0]);
        assert_eq!(drained_vertices(&mut changes.dirty), vec![v0, v2]);
        assert_eq!(drained_corners(&mut changes.dirty), vec![h1]);

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
        assert!(dirty.has_dirty_faces());
        assert!(dirty.has_dirty_vertices());
        assert!(dirty.has_dirty_corners());
        assert_eq!(drained_faces(&mut dirty), vec![face]);
        assert_eq!(drained_vertices(&mut dirty), vec![vertex]);
        assert_eq!(drained_corners(&mut dirty), vec![corner]);
    }

    #[test]
    fn dirty_set_drains_in_deterministic_order_and_consumes() {
        let mut dirty = DirtySet::new();
        let f2 = FaceId::from(Id::new(2, NonZeroU32::MIN));
        let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        dirty.mark_face(f2);
        dirty.mark_face(f0);
        dirty.mark_face(f1);

        assert_eq!(drained_faces(&mut dirty), vec![f0, f1, f2]);
        assert!(drained_faces(&mut dirty).is_empty());
        assert!(!dirty.has_dirty_faces());
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
        let before = mesh.revision();
        let mut txn = mesh.begin();
        let vertex = txn.add_vertex([0.0, 1.0, 2.0]);
        txn.abort();

        assert_eq!(mesh.vertex_position(vertex), Some(&[0.0, 1.0, 2.0]));
        assert_eq!(mesh.revision(), before);
    }

    #[test]
    fn mesh_revision_increments_once_per_commit() {
        let mut mesh = Mesh::new();
        assert_eq!(mesh.revision().value(), 0);

        let _ = mesh.begin().commit();
        assert_eq!(mesh.revision().value(), 1);

        let mut txn = mesh.begin();
        let _ = txn.add_vertex([1.0, 0.0, 0.0]);
        let _ = txn.commit();
        assert_eq!(mesh.revision().value(), 2);
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
        let mut changes = txn.commit();

        assert_eq!(changes.deleted_faces, vec![face]);
        assert_eq!(changes.deleted_half_edges, vec![edge]);
        assert_eq!(changes.deleted_vertices, vec![vertex]);
        assert_eq!(drained_faces(&mut changes.dirty), vec![face]);
        assert_eq!(drained_corners(&mut changes.dirty), vec![edge]);
        assert_eq!(drained_vertices(&mut changes.dirty), vec![vertex]);
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

    fn find_half_edge(mesh: &Mesh, from: u32, to: u32) -> Option<HalfEdgeId> {
        mesh.faces().find_map(|face| {
            mesh.face_loop(face).find(|&half_edge| {
                mesh.from_vertex(half_edge)
                    .is_some_and(|vertex| vertex.index() == from)
                    && mesh
                        .to_vertex(half_edge)
                        .is_some_and(|vertex| vertex.index() == to)
            })
        })
    }
}
