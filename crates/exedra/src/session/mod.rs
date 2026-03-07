// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit eager edit scopes and deterministic change summaries.
//!
//! This module owns edit-session hosting, bookkeeping, cache invalidation, and
//! shared mutation plumbing. Public kernel mutation entry points live in
//! [`crate::op`].

use alloc::vec::Vec;
use core::fmt;

use understory_dirty::{Channel, DirtySet as UnderstoryDirtySet};

use crate::sorted_merge::for_each_count_join;
use crate::{CornerId, FaceId, HalfEdgeId, Id, Mesh, VertexId, attr};

const DIRTY_FACES_CHANNEL: Channel = Channel::new(0);
const DIRTY_VERTICES_CHANNEL: Channel = Channel::new(1);
const DIRTY_CORNERS_CHANNEL: Channel = Channel::new(2);

#[derive(Copy, Clone, Debug)]
pub(crate) struct OutgoingAdj {
    from: VertexId,
    to: VertexId,
    half_edge: HalfEdgeId,
    face: FaceId,
}

mod attrs;
mod bookkeeping;
pub(crate) mod propagation;

/// Vertex-position propagation behavior for topology edits.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum PositionPropagation {
    /// Compute the new position as the endpoint midpoint.
    Midpoint,
    ///
    /// Edit kernels supply the weights/inputs for this strategy.
    WeightedMidpoint,
}

/// Corner-UV propagation behavior for topology edits.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum UvPropagation {
    /// Compute UVs as midpoint/interpolation from source corners.
    Midpoint,
    /// Copy UVs from one source side chosen by the edit primitive.
    CopyFromSide,
}

/// Corner normal-override propagation for topology edits.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum NormalOverridePropagation {
    /// Clear authored override so derived normals recompute.
    Clear,
    /// Copy override from one source side chosen by the edit primitive.
    CopyFromSide,
    /// Average source overrides when available.
    ///
    /// Edit kernels define source sampling and weighting details.
    Average,
}

/// Face-domain attribute propagation for topology edits.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum FaceAttrPropagation {
    /// Copy source face attributes.
    Copy,
    /// Copy attributes and allow the edit primitive to append metadata/tag.
    ///
    /// Edit kernels supply tag payload details.
    CopyAndTag,
}

/// Edge-domain attribute propagation for topology edits.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum EdgeAttrPropagation {
    /// Inherit source edge attributes.
    Inherit,
    /// Clear authored edge attributes on new topology.
    Clear,
    /// Apply subdivision-style decay to edge sharpness on split.
    ///
    /// Current behavior decays by `1.0` and clamps at `0.0`.
    DecayOnSplit,
}

/// Policy controlling attribute/value propagation across topology edits.
///
/// This is designed for edit primitives such as split/collapse/flip operations.
/// v0.1 defines the framework and defaults; individual edit kernels consume the
/// policy as they are implemented.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct PropagatePolicy {
    /// Position propagation mode.
    pub position: PositionPropagation,
    /// Corner UV propagation mode.
    pub uv: UvPropagation,
    /// Corner normal-override propagation mode.
    pub normal_override: NormalOverridePropagation,
    /// Face-domain propagation mode.
    pub face_attr: FaceAttrPropagation,
    /// Edge-domain propagation mode.
    pub edge_attr: EdgeAttrPropagation,
}

impl Default for PropagatePolicy {
    fn default() -> Self {
        Self {
            position: PositionPropagation::Midpoint,
            uv: UvPropagation::Midpoint,
            normal_override: NormalOverridePropagation::Clear,
            face_attr: FaceAttrPropagation::Copy,
            edge_attr: EdgeAttrPropagation::Inherit,
        }
    }
}

/// Conservative dirty summary for incremental systems.
///
/// This wraps [`understory_dirty`] primitives while exposing typed Exedra
/// domains. The primary consumption path is deterministic drain operations.
///
/// Most callers consume this via [`ChangeSet::dirty`] after
/// [`EditSession::finish`](crate::EditSession::finish).
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

/// Deterministic summary of mesh changes produced by a recorded edit scope.
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

/// Sink for optional edit-scope change recording.
///
/// [`EditSession`] always mutates the mesh eagerly. A sink controls whether the
/// session also records created/deleted IDs and dirty channels while edits are
/// applied. [`ChangeSetBuilder`] is the standard sink for callers that need an
/// Exedra [`ChangeSet`]; [`DiscardChanges`] is the default no-op sink used by
/// [`Mesh::edit`].
pub trait ChangeSink {
    /// Final output returned by [`EditSession::finish`].
    type Output;

    /// Records one dirty face.
    fn mark_face_dirty(&mut self, _face: FaceId) {}

    /// Records one dirty vertex.
    fn mark_vertex_dirty(&mut self, _vertex: VertexId) {}

    /// Records one dirty corner.
    fn mark_corner_dirty(&mut self, _corner: CornerId) {}

    /// Records one created vertex.
    fn record_created_vertex(&mut self, _vertex: VertexId) {}

    /// Records one created half-edge.
    fn record_created_half_edge(&mut self, _half_edge: HalfEdgeId) {}

    /// Records one created face.
    fn record_created_face(&mut self, _face: FaceId) {}

    /// Records one deleted vertex.
    fn record_deleted_vertex(&mut self, _vertex: VertexId) {}

    /// Records one deleted half-edge.
    fn record_deleted_half_edge(&mut self, _half_edge: HalfEdgeId) {}

    /// Records one deleted face.
    fn record_deleted_face(&mut self, _face: FaceId) {}

    /// Finalizes the sink and returns its output.
    fn finish(self) -> Self::Output;
}

/// Default no-op change sink used by [`Mesh::edit`].
#[derive(Copy, Clone, Debug, Default)]
pub struct DiscardChanges;

impl ChangeSink for DiscardChanges {
    type Output = ();

    fn finish(self) -> Self::Output {}
}

/// Builder sink that records a deterministic [`ChangeSet`].
#[derive(Clone, Debug, Default)]
pub struct ChangeSetBuilder {
    dirty: DirtySet,
    created_vertices: Vec<VertexId>,
    created_half_edges: Vec<HalfEdgeId>,
    created_faces: Vec<FaceId>,
    deleted_vertices: Vec<VertexId>,
    deleted_half_edges: Vec<HalfEdgeId>,
    deleted_faces: Vec<FaceId>,
}

impl ChangeSetBuilder {
    /// Creates an empty change-set builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ChangeSink for ChangeSetBuilder {
    type Output = ChangeSet;

    fn mark_face_dirty(&mut self, face: FaceId) {
        self.dirty.mark_face(face);
    }

    fn mark_vertex_dirty(&mut self, vertex: VertexId) {
        self.dirty.mark_vertex(vertex);
    }

    fn mark_corner_dirty(&mut self, corner: CornerId) {
        self.dirty.mark_corner(corner);
    }

    fn record_created_vertex(&mut self, vertex: VertexId) {
        self.created_vertices.push(vertex);
    }

    fn record_created_half_edge(&mut self, half_edge: HalfEdgeId) {
        self.created_half_edges.push(half_edge);
    }

    fn record_created_face(&mut self, face: FaceId) {
        self.created_faces.push(face);
    }

    fn record_deleted_vertex(&mut self, vertex: VertexId) {
        self.deleted_vertices.push(vertex);
        self.dirty.mark_vertex(vertex);
    }

    fn record_deleted_half_edge(&mut self, half_edge: HalfEdgeId) {
        self.deleted_half_edges.push(half_edge);
        self.dirty.mark_corner(half_edge);
    }

    fn record_deleted_face(&mut self, face: FaceId) {
        self.deleted_faces.push(face);
        self.dirty.mark_face(face);
    }

    fn finish(mut self) -> Self::Output {
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
}

/// Face-deletion behavior for isolated vertices.
///
/// Passed to [`crate::op::delete_faces`] and [`crate::op::delete_edges`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum DeletePolicy {
    /// Remove isolated vertices after face deletion.
    #[default]
    CleanupIsolated,
    /// Keep isolated vertices with `out = HalfEdgeId::INVALID`.
    KeepIsolated,
}

/// Structured face-deletion error from [`crate::op::delete_faces`].
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

/// Structured edge-deletion error from [`crate::op::delete_edges`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DeleteEdgesError {
    /// Input edge list must be sorted, deduplicated, and canonicalized.
    NonCanonicalEdgeSet,
    /// Half-edge ID is stale/dead or has no live twin.
    HalfEdgeNotLive {
        /// Stale half-edge index.
        half_edge: u32,
    },
    /// Edge does not bound any interior face.
    EdgeHasNoInteriorFace {
        /// Half-edge index.
        half_edge: u32,
    },
    /// Incident-face deletion failed.
    FaceDeleteFailed(DeleteFacesError),
}

impl fmt::Display for DeleteEdgesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonCanonicalEdgeSet => f.write_str(
                "edge set must be sorted, deduplicated, and use canonical undirected edge IDs",
            ),
            Self::HalfEdgeNotLive { half_edge } => {
                write!(f, "half-edge is not live or has no live twin: {half_edge}")
            }
            Self::EdgeHasNoInteriorFace { half_edge } => {
                write!(f, "edge has no interior incident face: {half_edge}")
            }
            Self::FaceDeleteFailed(err) => write!(f, "failed to delete incident faces: {err}"),
        }
    }
}

impl core::error::Error for DeleteEdgesError {}

/// Structured vertex-deletion error from [`crate::op::delete_vertices`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DeleteVerticesError {
    /// Input vertex list must be sorted and deduplicated.
    NonCanonicalVertexSet,
    /// Vertex ID is stale/dead.
    VertexNotLive {
        /// Stale vertex index.
        vertex: u32,
    },
    /// Vertex still has incident topology.
    VertexNotIsolated {
        /// Vertex index.
        vertex: u32,
    },
}

impl fmt::Display for DeleteVerticesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonCanonicalVertexSet => {
                f.write_str("vertex set must be sorted and deduplicated")
            }
            Self::VertexNotLive { vertex } => write!(f, "vertex is not live: {vertex}"),
            Self::VertexNotIsolated { vertex } => {
                write!(f, "vertex is not isolated: {vertex}")
            }
        }
    }
}

impl core::error::Error for DeleteVerticesError {}

/// Structured edge-split error from [`crate::op::split_edge`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SplitEdgeError {
    /// Half-edge must be live and have a live twin.
    HalfEdgeNotLive {
        /// Stale or invalid half-edge index.
        half_edge: u32,
    },
}

impl fmt::Display for SplitEdgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HalfEdgeNotLive { half_edge } => {
                write!(f, "half-edge is not live or has no live twin: {half_edge}")
            }
        }
    }
}

impl core::error::Error for SplitEdgeError {}

/// Structured face-split error from [`crate::op::split_face`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SplitFaceError {
    /// Corner ID is stale or not live.
    CornerNotLive {
        /// Stale corner index.
        corner: u32,
    },
    /// Corners must belong to the same interior face.
    CornersNotOnSameFace,
    /// `FaceId::OUTSIDE` cannot be split.
    OutsideFaceNotAllowed,
    /// Corners must be distinct.
    IdenticalCorners,
    /// Adjacent corners cannot form a diagonal.
    AdjacentCorners,
}

impl fmt::Display for SplitFaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CornerNotLive { corner } => write!(f, "corner is not live: {corner}"),
            Self::CornersNotOnSameFace => f.write_str("corners must belong to the same face"),
            Self::OutsideFaceNotAllowed => f.write_str("FaceId::OUTSIDE cannot be split"),
            Self::IdenticalCorners => f.write_str("corners must be distinct"),
            Self::AdjacentCorners => f.write_str("adjacent corners cannot form a diagonal"),
        }
    }
}

impl core::error::Error for SplitFaceError {}

/// Structured face-creation error from [`crate::op::add_face`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AddFaceError {
    /// Face loop must have at least three vertices.
    LoopTooShort,
    /// Vertex ID is stale or not live.
    VertexNotLive {
        /// Stale vertex index.
        vertex: u32,
    },
    /// Consecutive loop vertices must differ.
    ZeroLengthEdge {
        /// Source vertex index.
        from: u32,
        /// Destination vertex index.
        to: u32,
    },
    /// Loop vertices must be unique.
    RepeatedVertex {
        /// Repeated vertex index.
        vertex: u32,
    },
    /// Adding the face would make an undirected edge non-manifold.
    NonManifoldEdge {
        /// Smaller endpoint index.
        a: u32,
        /// Larger endpoint index.
        b: u32,
    },
}

impl fmt::Display for AddFaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LoopTooShort => f.write_str("face loop must contain at least 3 vertices"),
            Self::VertexNotLive { vertex } => write!(f, "vertex is not live: {vertex}"),
            Self::ZeroLengthEdge { from, to } => {
                write!(f, "zero-length edge in face loop: {from}->{to}")
            }
            Self::RepeatedVertex { vertex } => {
                write!(f, "repeated vertex in face loop: {vertex}")
            }
            Self::NonManifoldEdge { a, b } => {
                write!(f, "adding face would create non-manifold edge: ({a}, {b})")
            }
        }
    }
}

impl core::error::Error for AddFaceError {}

/// Single-writer eager edit scope over a mesh.
///
/// [`EditSession`] owns eager mutation access, optional change recording, cache
/// invalidation, and internal topology helper plumbing.
///
/// For the public kernel operation catalog, use [`crate::op`] and apply
/// mutation functions through an active session. `EditSession` is not a second
/// public mutation catalog.
///
/// Mutations are applied eagerly to the underlying mesh. Dropping a session
/// does not roll back mesh changes; it only discards any accumulated sink
/// state.
///
/// Acquire via [`Mesh::edit`] or [`Mesh::edit_with`], apply mutating operations,
/// then close the scope with [`EditSession::finish`].
#[derive(Debug)]
pub struct EditSession<'a, S: ChangeSink = DiscardChanges> {
    mesh: &'a mut Mesh,
    outgoing_index: Vec<OutgoingAdj>,
    outgoing_index_valid: bool,
    sink: S,
}

impl Mesh {
    /// Begins a new eager edit scope without change recording.
    ///
    /// Mutations performed through the session update the mesh immediately.
    /// Call [`EditSession::finish`] to finalize the scope and bump
    /// [`Mesh::revision`].
    pub fn edit(&mut self) -> EditSession<'_> {
        EditSession {
            mesh: self,
            outgoing_index: Vec::new(),
            outgoing_index_valid: false,
            sink: DiscardChanges,
        }
    }

    /// Begins a new eager edit scope with explicit change recording.
    ///
    /// Use [`ChangeSetBuilder`] when you need a deterministic Exedra
    /// [`ChangeSet`] at the end of the scope.
    pub fn edit_with<S: ChangeSink>(&mut self, sink: S) -> EditSession<'_, S> {
        EditSession {
            mesh: self,
            outgoing_index: Vec::new(),
            outgoing_index_valid: false,
            sink,
        }
    }
}

impl<S: ChangeSink> EditSession<'_, S> {
    pub(crate) fn ensure_outgoing_index(&mut self) -> &[OutgoingAdj] {
        if !self.outgoing_index_valid {
            self.outgoing_index = build_outgoing_index(self.mesh);
            self.outgoing_index_valid = true;
        }
        &self.outgoing_index
    }

    pub(crate) fn invalidate_outgoing_index(&mut self) {
        self.outgoing_index_valid = false;
    }

    pub(crate) fn vertex_has_incident_half_edge(&mut self, vertex: VertexId) -> bool {
        let index = self.ensure_outgoing_index();
        vertex_has_incident_half_edge_in_index(index, vertex)
    }

    pub(crate) fn has_undirected_edge(&mut self, a: VertexId, b: VertexId) -> bool {
        let index = self.ensure_outgoing_index();
        has_undirected_edge_in_index(index, a, b)
    }

    pub(crate) fn find_boundary_half_edge(
        &mut self,
        from: VertexId,
        to: VertexId,
    ) -> Option<HalfEdgeId> {
        let index = self.ensure_outgoing_index();
        find_boundary_half_edge_in_index(index, from, to)
    }

    #[cfg(test)]
    fn assert_outgoing_index_consistent(&mut self) {
        let actual = self.ensure_outgoing_index().to_vec();
        let expected = build_outgoing_index(self.mesh);

        assert_eq!(
            actual.len(),
            expected.len(),
            "outgoing index entry count mismatch: actual={}, expected={}",
            actual.len(),
            expected.len()
        );
        for (position, (a, b)) in actual.iter().zip(expected.iter()).enumerate() {
            assert_eq!(
                a.from,
                b.from,
                "outgoing index mismatch at {position}: from actual={} expected={}",
                a.from.index(),
                b.from.index()
            );
            assert_eq!(
                a.to,
                b.to,
                "outgoing index mismatch at {position}: to actual={} expected={}",
                a.to.index(),
                b.to.index()
            );
            assert_eq!(
                a.half_edge,
                b.half_edge,
                "outgoing index mismatch at {position}: half-edge actual={} expected={}",
                a.half_edge.index(),
                b.half_edge.index()
            );
            assert_eq!(
                a.face,
                b.face,
                "outgoing index mismatch at {position}: face actual={} expected={}",
                a.face.index(),
                b.face.index()
            );
        }
    }
}

pub(crate) fn sort_dedup<T: Ord>(values: &mut Vec<T>) {
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

pub(crate) fn is_canonical_face_set(faces: &[FaceId]) -> bool {
    faces.windows(2).all(|pair| pair[0] < pair[1])
}

pub(crate) fn is_canonical_vertex_set(vertices: &[VertexId]) -> bool {
    vertices.windows(2).all(|pair| pair[0] < pair[1])
}

pub(crate) fn contains_face(faces: &[FaceId], target: FaceId) -> bool {
    faces.binary_search(&target).is_ok()
}

pub(crate) fn collect_half_edge_vertices(
    mesh: &Mesh,
    half_edge: HalfEdgeId,
    out: &mut Vec<VertexId>,
) {
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

fn vertex_has_incident_half_edge_in_index(
    outgoing_index: &[OutgoingAdj],
    vertex: VertexId,
) -> bool {
    !equal_range_in_outgoing(outgoing_index, vertex).is_empty()
}

fn has_undirected_edge_in_index(outgoing_index: &[OutgoingAdj], a: VertexId, b: VertexId) -> bool {
    let a_range = equal_range_in_outgoing(outgoing_index, a);
    for entry in &outgoing_index[a_range] {
        if entry.to == b {
            return true;
        }
    }
    let b_range = equal_range_in_outgoing(outgoing_index, b);
    for entry in &outgoing_index[b_range] {
        if entry.to == a {
            return true;
        }
    }
    false
}

fn find_boundary_half_edge_in_index(
    outgoing_index: &[OutgoingAdj],
    from: VertexId,
    to: VertexId,
) -> Option<HalfEdgeId> {
    let range = equal_range_in_outgoing(outgoing_index, from);
    outgoing_index[range].iter().find_map(|entry| {
        (entry.face == FaceId::OUTSIDE && entry.to == to).then_some(entry.half_edge)
    })
}

fn build_outgoing_index(mesh: &Mesh) -> Vec<OutgoingAdj> {
    let mut pairs = mesh
        .half_edges
        .iter()
        .filter_map(|(id, edge)| {
            let half_edge = HalfEdgeId::from(id);
            half_edge_vertices(mesh, half_edge).map(|(from, to)| OutgoingAdj {
                from,
                to,
                half_edge,
                face: edge.face,
            })
        })
        .collect::<Vec<_>>();
    pairs.sort_unstable_by_key(|entry| (entry.from, entry.half_edge));
    pairs
}

pub(crate) fn find_outgoing_half_edge_linear_scan(
    mesh: &Mesh,
    vertex: VertexId,
) -> Option<HalfEdgeId> {
    mesh.half_edges.iter().find_map(|(id, _)| {
        let half_edge = HalfEdgeId::from(id);
        (half_edge_vertices(mesh, half_edge).is_some_and(|(from, _)| from == vertex))
            .then_some(half_edge)
    })
}

pub(crate) fn find_outgoing_half_edge(
    outgoing_index: &[OutgoingAdj],
    vertex: VertexId,
) -> Option<HalfEdgeId> {
    let range = equal_range_in_outgoing(outgoing_index, vertex);
    outgoing_index.get(range.start).map(|entry| entry.half_edge)
}

fn equal_range_in_outgoing(
    outgoing_index: &[OutgoingAdj],
    vertex: VertexId,
) -> core::ops::Range<usize> {
    let lower = outgoing_index.partition_point(|entry| entry.from < vertex);
    let upper = outgoing_index.partition_point(|entry| entry.from <= vertex);
    lower..upper
}

pub(crate) fn should_use_global_outgoing_index(
    affected_vertices: usize,
    total_half_edges: usize,
) -> bool {
    if affected_vertices == 0 || total_half_edges == 0 {
        return false;
    }
    // Heuristic: switch to global index once the affected set is at least
    // about 1/8 of total half-edges; this avoids expensive repeated linear
    // scans near crossover cases.
    affected_vertices.saturating_mul(8) > total_half_edges
}

pub(crate) fn corner_uv_for_face_to_vertex(
    mesh: &Mesh,
    face: FaceId,
    vertex: VertexId,
) -> Option<[f32; 2]> {
    if face == FaceId::OUTSIDE {
        return None;
    }
    let corner = mesh
        .face_loop(face)
        .find(|&candidate| mesh.to_vertex(candidate) == Some(vertex))?;
    mesh.attrs()
        .sparse(attr::CORNER_UV)
        .and_then(|layer| layer.get(corner.as_id()).copied())
}

pub(crate) fn clear_deleted_corner_attrs(mesh: &mut Mesh, half_edge: HalfEdgeId) {
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

pub(crate) fn preflight_boundary_continuation(
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

pub(crate) fn stitch_outside_loops(mesh: &mut Mesh) {
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

pub(crate) fn stitch_outside_loops_for_vertices(mesh: &mut Mesh, affected_vertices: &[VertexId]) {
    if affected_vertices.is_empty() {
        return;
    }

    let mut affected = affected_vertices.to_vec();
    sort_dedup(&mut affected);

    let mut starts = Vec::<(VertexId, HalfEdgeId)>::new();
    let mut impacted = Vec::<HalfEdgeId>::new();
    for (id, edge) in mesh.half_edges.iter() {
        if edge.face != FaceId::OUTSIDE {
            continue;
        }
        let half_edge = HalfEdgeId::from(id);
        let to = mesh
            .to_vertex(half_edge)
            .expect("boundary half-edge must have destination vertex");
        let start = mesh
            .twin(half_edge)
            .and_then(|twin| mesh.to_vertex(twin))
            .expect("boundary half-edge must have twin with destination vertex");
        starts.push((start, half_edge));
        if affected.binary_search(&to).is_ok() {
            impacted.push(half_edge);
        }
    }

    if impacted.is_empty() {
        return;
    }

    starts.sort_unstable_by_key(|(start, boundary_half_edge)| (*start, *boundary_half_edge));
    impacted.sort_unstable();
    impacted.dedup();

    for half_edge in impacted {
        let to = mesh
            .to_vertex(half_edge)
            .expect("boundary half-edge must have destination vertex");
        let range = equal_range_by_vertex(&starts, to);
        let candidates = range.end.saturating_sub(range.start);
        if candidates != 1 {
            // Internal invariant: add_face preflight should prevent user-input
            // ambiguity here. If this trips, topology state is inconsistent.
            panic!(
                "mesh topology corruption: OUTSIDE local stitch failed at vertex {} ({} candidates)",
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

    use crate::{Face, HalfEdge, Id, MeshBuilder, op};

    use super::*;

    trait SessionOpExt {
        fn add_vertex(&mut self, position: [f32; 3]) -> VertexId;
        fn set_vertex_position(
            &mut self,
            vertex: VertexId,
            position: [f32; 3],
        ) -> Result<(), op::SetVertexPositionError>;
        fn set_face_region(
            &mut self,
            face: FaceId,
            region: u32,
        ) -> Result<(), op::SetFaceRegionError>;
        fn set_corner_uv(
            &mut self,
            corner: CornerId,
            uv: [f32; 2],
        ) -> Result<(), op::SetCornerUvError>;
        fn set_edge_seam(
            &mut self,
            half_edge: HalfEdgeId,
            seam: bool,
        ) -> Result<(), op::SetEdgeSeamError>;
        fn set_edge_sharpness(
            &mut self,
            half_edge: HalfEdgeId,
            sharpness: f32,
        ) -> Result<(), op::SetEdgeSharpnessError>;
        fn add_face(&mut self, loop_vertices: &[VertexId]) -> Result<FaceId, AddFaceError>;
        fn split_edge(
            &mut self,
            half_edge: HalfEdgeId,
            policy: &PropagatePolicy,
        ) -> Result<VertexId, SplitEdgeError>;
        fn split_face(
            &mut self,
            corner_a: CornerId,
            corner_b: CornerId,
            policy: &PropagatePolicy,
        ) -> Result<FaceId, SplitFaceError>;
        fn delete_faces(
            &mut self,
            faces: &[FaceId],
            policy: DeletePolicy,
        ) -> Result<(), DeleteFacesError>;
        fn delete_edges(
            &mut self,
            edges: &[HalfEdgeId],
            policy: DeletePolicy,
        ) -> Result<(), DeleteEdgesError>;
        fn delete_vertices(&mut self, vertices: &[VertexId]) -> Result<(), DeleteVerticesError>;
    }

    trait MeshOpExt {
        fn delete_faces(
            &mut self,
            faces: &[FaceId],
            policy: DeletePolicy,
        ) -> Result<ChangeSet, DeleteFacesError>;
        fn delete_edges(
            &mut self,
            edges: &[HalfEdgeId],
            policy: DeletePolicy,
        ) -> Result<ChangeSet, DeleteEdgesError>;
        fn delete_vertices(
            &mut self,
            vertices: &[VertexId],
        ) -> Result<ChangeSet, DeleteVerticesError>;
    }

    impl<S: ChangeSink> SessionOpExt for EditSession<'_, S> {
        fn add_vertex(&mut self, position: [f32; 3]) -> VertexId {
            op::add_vertex(self, position)
        }

        fn set_vertex_position(
            &mut self,
            vertex: VertexId,
            position: [f32; 3],
        ) -> Result<(), op::SetVertexPositionError> {
            op::set_vertex_position(self, vertex, position)
        }

        fn set_face_region(
            &mut self,
            face: FaceId,
            region: u32,
        ) -> Result<(), op::SetFaceRegionError> {
            op::set_face_region(self, face, region)
        }

        fn set_corner_uv(
            &mut self,
            corner: CornerId,
            uv: [f32; 2],
        ) -> Result<(), op::SetCornerUvError> {
            op::set_corner_uv(self, corner, uv)
        }

        fn set_edge_seam(
            &mut self,
            half_edge: HalfEdgeId,
            seam: bool,
        ) -> Result<(), op::SetEdgeSeamError> {
            op::set_edge_seam(self, half_edge, seam)
        }

        fn set_edge_sharpness(
            &mut self,
            half_edge: HalfEdgeId,
            sharpness: f32,
        ) -> Result<(), op::SetEdgeSharpnessError> {
            op::set_edge_sharpness(self, half_edge, sharpness)
        }

        fn add_face(&mut self, loop_vertices: &[VertexId]) -> Result<FaceId, AddFaceError> {
            op::add_face(self, loop_vertices)
        }

        fn split_edge(
            &mut self,
            half_edge: HalfEdgeId,
            policy: &PropagatePolicy,
        ) -> Result<VertexId, SplitEdgeError> {
            op::split_edge(self, half_edge, policy)
        }

        fn split_face(
            &mut self,
            corner_a: CornerId,
            corner_b: CornerId,
            policy: &PropagatePolicy,
        ) -> Result<FaceId, SplitFaceError> {
            op::split_face(self, corner_a, corner_b, policy)
        }

        fn delete_faces(
            &mut self,
            faces: &[FaceId],
            policy: DeletePolicy,
        ) -> Result<(), DeleteFacesError> {
            op::delete_faces(self, faces, policy)
        }

        fn delete_edges(
            &mut self,
            edges: &[HalfEdgeId],
            policy: DeletePolicy,
        ) -> Result<(), DeleteEdgesError> {
            op::delete_edges(self, edges, policy)
        }

        fn delete_vertices(&mut self, vertices: &[VertexId]) -> Result<(), DeleteVerticesError> {
            op::delete_vertices(self, vertices)
        }
    }

    impl MeshOpExt for Mesh {
        fn delete_faces(
            &mut self,
            faces: &[FaceId],
            policy: DeletePolicy,
        ) -> Result<ChangeSet, DeleteFacesError> {
            let mut txn = self.edit_with(ChangeSetBuilder::new());
            op::delete_faces(&mut txn, faces, policy)?;
            Ok(txn.finish())
        }

        fn delete_edges(
            &mut self,
            edges: &[HalfEdgeId],
            policy: DeletePolicy,
        ) -> Result<ChangeSet, DeleteEdgesError> {
            let mut txn = self.edit_with(ChangeSetBuilder::new());
            op::delete_edges(&mut txn, edges, policy)?;
            Ok(txn.finish())
        }

        fn delete_vertices(
            &mut self,
            vertices: &[VertexId],
        ) -> Result<ChangeSet, DeleteVerticesError> {
            let mut txn = self.edit_with(ChangeSetBuilder::new());
            op::delete_vertices(&mut txn, vertices)?;
            Ok(txn.finish())
        }
    }

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

    fn two_tri_strip_mesh() -> Mesh {
        Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [2, 1, 3]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("two-triangle strip should build")
    }

    fn three_tri_strip_mesh() -> Mesh {
        Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [2, 1, 3], [2, 3, 4]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("three-triangle strip should build")
    }

    fn canonical_interior_edges(mesh: &Mesh) -> Vec<HalfEdgeId> {
        let mut edges = mesh
            .half_edges
            .iter()
            .map(|(id, _)| HalfEdgeId::from(id))
            .filter(|&half_edge| {
                let Some(twin) = mesh.twin(half_edge) else {
                    return false;
                };
                if core::cmp::min(half_edge, twin) != half_edge {
                    return false;
                }
                let Some(face) = mesh.face(half_edge) else {
                    return false;
                };
                let Some(twin_face) = mesh.face(twin) else {
                    return false;
                };
                face != FaceId::OUTSIDE && twin_face != FaceId::OUTSIDE
            })
            .collect::<Vec<_>>();
        edges.sort_unstable();
        edges
    }

    fn canonical_boundary_edge(mesh: &Mesh) -> HalfEdgeId {
        mesh.half_edges
            .iter()
            .map(|(id, _)| HalfEdgeId::from(id))
            .find(|&half_edge| {
                let Some(twin) = mesh.twin(half_edge) else {
                    return false;
                };
                if core::cmp::min(half_edge, twin) != half_edge {
                    return false;
                }
                let Some(face) = mesh.face(half_edge) else {
                    return false;
                };
                let Some(twin_face) = mesh.face(twin) else {
                    return false;
                };
                (face == FaceId::OUTSIDE) != (twin_face == FaceId::OUTSIDE)
            })
            .expect("mesh should have canonical boundary edge")
    }

    #[test]
    fn propagate_policy_defaults_are_sensible() {
        let policy = PropagatePolicy::default();
        assert_eq!(policy.position, PositionPropagation::Midpoint);
        assert_eq!(policy.uv, UvPropagation::Midpoint);
        assert_eq!(policy.normal_override, NormalOverridePropagation::Clear);
        assert_eq!(policy.face_attr, FaceAttrPropagation::Copy);
        assert_eq!(policy.edge_attr, EdgeAttrPropagation::Inherit);
    }

    #[test]
    fn per_call_policies_can_mix_within_one_session() {
        let (mut mesh, shared) = split_source_mesh();
        let twin = mesh.twin(shared).expect("shared edge should have twin");
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_edge_sharpness(shared, 3.0).is_ok());
            let _ = txn.finish();
        }
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let decay = PropagatePolicy {
            edge_attr: EdgeAttrPropagation::DecayOnSplit,
            ..PropagatePolicy::default()
        };
        let _ = txn
            .split_edge(shared, &decay)
            .expect("split with decay policy should succeed");
        let child_h = txn
            .mesh()
            .next(shared)
            .expect("split child should exist after first split");
        let clear = PropagatePolicy {
            edge_attr: EdgeAttrPropagation::Clear,
            ..PropagatePolicy::default()
        };
        let _ = txn
            .split_edge(child_h, &clear)
            .expect("split with clear policy should succeed");
        let _ = txn.finish();

        let child_t = mesh.next(twin).expect("split child should exist");
        let child_h_twin = mesh.twin(child_h).expect("split child should have twin");
        assert_eq!(mesh.edge_sharpness(shared), Some(2.0));
        assert_eq!(mesh.edge_sharpness(child_t), Some(0.0));
        assert_eq!(mesh.edge_sharpness(child_h), Some(0.0));
        assert_eq!(mesh.edge_sharpness(child_h_twin), Some(0.0));
    }

    #[test]
    fn txn_add_vertex_commits_created_and_dirty() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let vertex = txn.add_vertex([1.0, 2.0, 3.0]);
        let mut changes = txn.finish();

        assert_eq!(changes.created_vertices, vec![vertex]);
        assert_eq!(drained_vertices(&mut changes.dirty), vec![vertex]);
        assert!(drained_faces(&mut changes.dirty).is_empty());
        assert!(drained_corners(&mut changes.dirty).is_empty());
    }

    #[test]
    fn txn_set_vertex_position_marks_vertex_dirty() {
        let mut mesh = Mesh::new();
        let vertex = mesh.add_vertex([0.0, 0.0, 0.0]);
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        assert!(txn.set_vertex_position(vertex, [4.0, 5.0, 6.0]).is_ok());
        let mut changes = txn.finish();

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

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        assert!(txn.set_face_region(face, 9).is_ok());
        let mut changes = txn.finish();

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

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        assert!(txn.set_corner_uv(corner, [0.25, 0.5]).is_ok());
        assert_eq!(txn.corner_uv(corner), Some([0.25, 0.5]));
        let mut changes = txn.finish();
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

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        assert_eq!(txn.edge_seam(shared), Some(false));
        assert!(txn.set_edge_seam(shared_twin, true).is_ok());
        assert_eq!(txn.edge_seam(shared), Some(true));
        assert_eq!(txn.edge_seam(shared_twin), Some(true));
        let mut changes = txn.finish();

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

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        assert_eq!(txn.edge_sharpness(shared), Some(0.0));
        assert!(txn.set_edge_sharpness(shared_twin, 2.5).is_ok());
        assert_eq!(txn.edge_sharpness(shared), Some(2.5));
        assert_eq!(txn.edge_sharpness(shared_twin), Some(2.5));
        let mut changes = txn.finish();

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
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_corner_uv(h, [0.25, 0.0]).is_ok());
            assert!(txn.set_corner_uv(t_next, [0.75, 0.0]).is_ok());
            let _ = txn.finish();
        }
        assert_eq!(mesh.is_uv_discontinuous(shared), Some(true));
    }

    fn split_source_mesh() -> (Mesh, HalfEdgeId) {
        let mesh = Mesh::from_indexed_triangles(
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
        (mesh, shared)
    }

    #[test]
    fn split_edge_rejects_stale_half_edge() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let err = txn
            .split_edge(
                HalfEdgeId::new(99, NonZeroU32::MIN),
                &PropagatePolicy::default(),
            )
            .expect_err("stale half-edge must fail");
        assert_eq!(err, SplitEdgeError::HalfEdgeNotLive { half_edge: 99 });
    }

    #[test]
    fn split_edge_updates_topology_midpoint_and_change_tracking() {
        let (mut mesh, shared) = split_source_mesh();
        let twin = mesh.twin(shared).expect("shared edge should have twin");
        let left_face = mesh.face(shared).expect("shared edge face should exist");
        let right_face = mesh.face(twin).expect("shared twin face should exist");
        let from = mesh
            .from_vertex(shared)
            .expect("shared edge origin should exist");
        let to = mesh
            .to_vertex(shared)
            .expect("shared edge destination should exist");
        let vertices_before = mesh.vertices.len();
        let half_edges_before = mesh.half_edges.len();

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let inserted = txn
            .split_edge(shared, &PropagatePolicy::default())
            .expect("split should succeed");
        let mut changes = txn.finish();

        assert_eq!(mesh.vertices.len(), vertices_before + 1);
        assert_eq!(mesh.half_edges.len(), half_edges_before + 2);
        assert_eq!(
            mesh.faces.get(left_face.as_id()).map(|face| face.degree),
            Some(4)
        );
        assert_eq!(
            mesh.faces.get(right_face.as_id()).map(|face| face.degree),
            Some(4)
        );
        assert_eq!(mesh.vertex_position(inserted), Some(&[0.5, 0.5, 0.0]));
        assert_eq!(mesh.to_vertex(shared), Some(inserted));
        assert_eq!(mesh.to_vertex(twin), Some(inserted));
        let new_out = mesh
            .vertex_out(inserted)
            .expect("newly inserted vertex should have outgoing half-edge");
        let child_h = mesh.next(shared).expect("split child should exist");
        let child_t = mesh.next(twin).expect("split child should exist");
        assert!(new_out == child_h || new_out == child_t);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
        for (id, _) in mesh.half_edges.iter() {
            let half_edge = HalfEdgeId::from(id);
            let twin_of_twin = mesh
                .twin(half_edge)
                .and_then(|candidate| mesh.twin(candidate))
                .expect("live half-edge should have valid twin pair");
            assert_eq!(twin_of_twin, half_edge);
        }

        assert_eq!(changes.created_vertices, vec![inserted]);
        assert_eq!(changes.created_half_edges.len(), 2);
        let mut dirty_vertices = drained_vertices(&mut changes.dirty);
        dirty_vertices.sort_unstable();
        let mut expected_vertices = vec![from, to, inserted];
        expected_vertices.sort_unstable();
        assert_eq!(dirty_vertices, expected_vertices);
        let mut dirty_faces = drained_faces(&mut changes.dirty);
        dirty_faces.sort_unstable();
        let mut expected_faces = vec![left_face, right_face];
        expected_faces.sort_unstable();
        assert_eq!(dirty_faces, expected_faces);
        assert_eq!(drained_corners(&mut changes.dirty).len(), 4);
    }

    #[test]
    fn split_edge_edge_attributes_follow_policy() {
        let (mut mesh, shared) = split_source_mesh();
        let twin = mesh.twin(shared).expect("shared edge should have twin");
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_edge_seam(shared, true).is_ok());
            assert!(txn.set_edge_sharpness(shared, 2.5).is_ok());
            let _ = txn.finish();
        }

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let _ = txn
            .split_edge(shared, &PropagatePolicy::default())
            .expect("split should succeed");
        let _ = txn.finish();
        let child_h = mesh.next(shared).expect("split child should exist");
        let child_t = mesh.next(twin).expect("split child should exist");
        assert_eq!(mesh.edge_seam(shared), Some(true));
        assert_eq!(mesh.edge_seam(child_h), Some(true));
        assert_eq!(mesh.edge_seam(twin), Some(true));
        assert_eq!(mesh.edge_seam(child_t), Some(true));
        assert_eq!(mesh.edge_sharpness(shared), Some(2.5));
        assert_eq!(mesh.edge_sharpness(child_h), Some(2.5));
        assert_eq!(mesh.edge_sharpness(twin), Some(2.5));
        assert_eq!(mesh.edge_sharpness(child_t), Some(2.5));
        assert!(mesh.validate_deep().is_empty());

        let (mut mesh, shared) = split_source_mesh();
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_edge_seam(shared, true).is_ok());
            assert!(txn.set_edge_sharpness(shared, 2.5).is_ok());
            let _ = txn.finish();
        }
        let twin = mesh.twin(shared).expect("shared edge should have twin");
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let policy = PropagatePolicy {
            edge_attr: EdgeAttrPropagation::DecayOnSplit,
            ..PropagatePolicy::default()
        };
        let _ = txn
            .split_edge(shared, &policy)
            .expect("split should succeed");
        let _ = txn.finish();
        let child_h = mesh.next(shared).expect("split child should exist");
        let child_t = mesh.next(twin).expect("split child should exist");
        assert_eq!(mesh.edge_sharpness(shared), Some(1.5));
        assert_eq!(mesh.edge_sharpness(child_h), Some(1.5));
        assert_eq!(mesh.edge_sharpness(twin), Some(1.5));
        assert_eq!(mesh.edge_sharpness(child_t), Some(1.5));
        assert!(mesh.validate_deep().is_empty());

        let (mut mesh, shared) = split_source_mesh();
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_edge_seam(shared, true).is_ok());
            assert!(txn.set_edge_sharpness(shared, 2.5).is_ok());
            let _ = txn.finish();
        }
        let twin = mesh.twin(shared).expect("shared edge should have twin");
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let policy = PropagatePolicy {
            edge_attr: EdgeAttrPropagation::Clear,
            ..PropagatePolicy::default()
        };
        let _ = txn
            .split_edge(shared, &policy)
            .expect("split should succeed");
        let _ = txn.finish();
        let child_h = mesh.next(shared).expect("split child should exist");
        let child_t = mesh.next(twin).expect("split child should exist");
        assert_eq!(mesh.edge_seam(shared), Some(false));
        assert_eq!(mesh.edge_seam(child_h), Some(false));
        assert_eq!(mesh.edge_seam(twin), Some(false));
        assert_eq!(mesh.edge_seam(child_t), Some(false));
        assert_eq!(mesh.edge_sharpness(shared), Some(0.0));
        assert_eq!(mesh.edge_sharpness(child_h), Some(0.0));
        assert_eq!(mesh.edge_sharpness(twin), Some(0.0));
        assert_eq!(mesh.edge_sharpness(child_t), Some(0.0));
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn split_edge_uvs_follow_policy() {
        let (mut mesh, shared) = split_source_mesh();
        let twin = mesh.twin(shared).expect("shared edge should have twin");
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_corner_uv(shared, [0.0, 0.25]).is_ok());
            assert!(txn.set_corner_uv(twin, [1.0, 0.75]).is_ok());
            let _ = txn.finish();
        }

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let _ = txn
            .split_edge(shared, &PropagatePolicy::default())
            .expect("split should succeed");
        let _ = txn.finish();
        let child_h = mesh.next(shared).expect("split child should exist");
        let child_t = mesh.next(twin).expect("split child should exist");
        let uv_layer = mesh
            .attrs()
            .sparse(attr::CORNER_UV)
            .expect("corner UV layer should exist");
        assert_eq!(uv_layer.get(shared.as_id()).copied(), Some([0.0, 0.125]));
        assert_eq!(uv_layer.get(twin.as_id()).copied(), Some([0.5, 0.375]));
        assert_eq!(uv_layer.get(child_h.as_id()).copied(), Some([0.0, 0.25]));
        assert_eq!(uv_layer.get(child_t.as_id()).copied(), Some([1.0, 0.75]));
        assert!(mesh.validate_deep().is_empty());

        let (mut mesh, shared) = split_source_mesh();
        let twin = mesh.twin(shared).expect("shared edge should have twin");
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_corner_uv(shared, [0.0, 0.25]).is_ok());
            assert!(txn.set_corner_uv(twin, [1.0, 0.75]).is_ok());
            let _ = txn.finish();
        }
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let policy = PropagatePolicy {
            uv: UvPropagation::CopyFromSide,
            ..PropagatePolicy::default()
        };
        let _ = txn
            .split_edge(shared, &policy)
            .expect("split should succeed");
        let _ = txn.finish();
        let child_h = mesh.next(shared).expect("split child should exist");
        let child_t = mesh.next(twin).expect("split child should exist");
        let uv_layer = mesh
            .attrs()
            .sparse(attr::CORNER_UV)
            .expect("corner UV layer should exist");
        assert_eq!(uv_layer.get(shared.as_id()).copied(), Some([0.0, 0.25]));
        assert_eq!(uv_layer.get(twin.as_id()).copied(), Some([1.0, 0.75]));
        assert_eq!(uv_layer.get(child_h.as_id()).copied(), Some([0.0, 0.25]));
        assert_eq!(uv_layer.get(child_t.as_id()).copied(), Some([1.0, 0.75]));
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn split_edge_handles_boundary_edges() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let boundary = find_half_edge(&mesh, 0, 1).expect("boundary edge should exist");
        let twin = mesh
            .twin(boundary)
            .expect("boundary edge should have outside twin");
        let interior_face = mesh.face(boundary).expect("boundary face should exist");
        assert_eq!(mesh.face(twin), Some(FaceId::OUTSIDE));

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let inserted = txn
            .split_edge(boundary, &PropagatePolicy::default())
            .expect("split should succeed");
        let _ = txn.finish();

        assert_eq!(
            mesh.faces
                .get(interior_face.as_id())
                .map(|face| face.degree),
            Some(4)
        );
        assert_eq!(mesh.face(twin), Some(FaceId::OUTSIDE));
        assert!(mesh.vertex_out(inserted).is_some());
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn split_face_quad_inserts_diagonal_and_splits_into_triangles() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        let half_edges_before = mesh.half_edges.len();
        let faces_before = mesh.faces.len();

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let new_face = txn
            .split_face(corners[0], corners[2], &PropagatePolicy::default())
            .expect("split should succeed");
        let mut changes = txn.finish();

        assert_eq!(mesh.faces.len(), faces_before + 1);
        assert_eq!(mesh.half_edges.len(), half_edges_before + 2);
        assert_eq!(
            mesh.faces.get(face.as_id()).map(|record| record.degree),
            Some(3)
        );
        assert_eq!(
            mesh.faces.get(new_face.as_id()).map(|record| record.degree),
            Some(3)
        );
        assert_eq!(changes.created_faces, vec![new_face]);
        assert_eq!(changes.created_half_edges.len(), 2);
        let mut dirty_faces = drained_faces(&mut changes.dirty);
        dirty_faces.sort_unstable();
        assert_eq!(dirty_faces, vec![face, new_face]);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn split_face_rejects_adjacent_corners() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let err = txn
            .split_face(corners[0], corners[1], &PropagatePolicy::default())
            .expect_err("adjacent corners must fail");
        assert_eq!(err, SplitFaceError::AdjacentCorners);
    }

    #[test]
    fn split_face_rejects_identical_and_stale_corners() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let corners = mesh.face_loop(face).collect::<Vec<_>>();

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let err = txn
            .split_face(corners[0], corners[0], &PropagatePolicy::default())
            .expect_err("identical corners must fail");
        assert_eq!(err, SplitFaceError::IdenticalCorners);

        let err = txn
            .split_face(
                corners[0],
                CornerId::new(999, NonZeroU32::MIN),
                &PropagatePolicy::default(),
            )
            .expect_err("stale corner must fail");
        assert_eq!(err, SplitFaceError::CornerNotLive { corner: 999 });
    }

    #[test]
    fn split_face_rejects_outside_and_cross_face_inputs() {
        let mut open = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let boundary_a = find_half_edge(&open, 0, 1).expect("boundary should exist");
        let boundary_b = find_half_edge(&open, 1, 2).expect("boundary should exist");
        let outside_corner_a = open.twin(boundary_a).expect("outside corner should exist");
        let outside_corner_b = open.twin(boundary_b).expect("outside corner should exist");
        let mut txn = open.edit_with(ChangeSetBuilder::new());
        let err = txn
            .split_face(
                outside_corner_a,
                outside_corner_b,
                &PropagatePolicy::default(),
            )
            .expect_err("outside face should fail");
        assert_eq!(err, SplitFaceError::OutsideFaceNotAllowed);

        let (mut closed, _) = closed_box_mesh();
        let faces = closed.faces().collect::<Vec<_>>();
        let f0 = faces[0];
        let f1 = faces[1];
        let c0 = closed.face_loop(f0).next().expect("corner 0");
        let c1 = closed.face_loop(f1).next().expect("corner 1");
        let mut txn = closed.edit_with(ChangeSetBuilder::new());
        let err = txn
            .split_face(c0, c1, &PropagatePolicy::default())
            .expect_err("cross-face split must fail");
        assert_eq!(err, SplitFaceError::CornersNotOnSameFace);
    }

    #[test]
    fn split_face_ngon_and_uv_propagation() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [2.0, 1.0, 0.0],
                [1.0, 2.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3, 4]],
        )
        .expect("pentagon build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_corner_uv(corners[0], [0.0, 0.0]).is_ok());
            assert!(txn.set_corner_uv(corners[2], [2.0, 1.0]).is_ok());
            let _ = txn.finish();
        }

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let policy = PropagatePolicy {
            uv: UvPropagation::CopyFromSide,
            ..PropagatePolicy::default()
        };
        let new_face = txn
            .split_face(corners[0], corners[2], &policy)
            .expect("split should succeed");
        let changes = txn.finish();
        assert_eq!(changes.created_faces, vec![new_face]);

        let new_edges = changes.created_half_edges;
        assert_eq!(new_edges.len(), 2);
        let uv_layer = mesh
            .attrs()
            .sparse(attr::CORNER_UV)
            .expect("corner uv layer should exist");
        let uv_values = new_edges
            .iter()
            .map(|edge| uv_layer.get(edge.as_id()).copied().unwrap_or([0.0, 0.0]))
            .collect::<Vec<_>>();
        assert!(uv_values.contains(&[0.0, 0.0]));
        assert!(uv_values.contains(&[2.0, 1.0]));
        assert_eq!(
            mesh.faces.get(face.as_id()).map(|record| record.degree),
            Some(4)
        );
        assert_eq!(
            mesh.faces.get(new_face.as_id()).map(|record| record.degree),
            Some(3)
        );
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn split_face_uv_midpoint_preserves_missingness() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_corner_uv(corners[0], [0.5, 0.25]).is_ok());
            let _ = txn.finish();
        }

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let _ = txn
            .split_face(corners[0], corners[2], &PropagatePolicy::default())
            .expect("split should succeed");
        let changes = txn.finish();
        let uv_layer = mesh
            .attrs()
            .sparse(attr::CORNER_UV)
            .expect("corner uv layer should exist");
        for edge in changes.created_half_edges {
            assert!(
                uv_layer.get(edge.as_id()).is_none(),
                "new diagonal corners should stay missing when midpoint inputs are incomplete"
            );
        }
    }

    #[test]
    fn split_face_diagonal_sharpness_is_policy_controlled() {
        let (mut mesh, _) = closed_box_mesh();
        let face = mesh.faces().next().expect("face should exist");
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_edge_sharpness(corners[0], 3.0).is_ok());
            assert!(txn.set_edge_sharpness(corners[2], 2.0).is_ok());
            let _ = txn.finish();
        }

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let _ = txn
            .split_face(corners[0], corners[2], &PropagatePolicy::default())
            .expect("split should succeed");
        let changes = txn.finish();
        let diagonal = changes.created_half_edges[0];
        assert_eq!(mesh.edge_sharpness(diagonal), Some(0.0));

        let (mut mesh, _) = closed_box_mesh();
        let face = mesh.faces().next().expect("face should exist");
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            assert!(txn.set_edge_sharpness(corners[0], 3.0).is_ok());
            assert!(txn.set_edge_sharpness(corners[2], 2.0).is_ok());
            let _ = txn.finish();
        }
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let policy = PropagatePolicy {
            edge_attr: EdgeAttrPropagation::DecayOnSplit,
            ..PropagatePolicy::default()
        };
        let _ = txn
            .split_face(corners[0], corners[2], &policy)
            .expect("split should succeed");
        let changes = txn.finish();
        let diagonal = changes.created_half_edges[0];
        assert_eq!(mesh.edge_sharpness(diagonal), Some(2.0));
    }

    #[test]
    fn add_face_creates_triangle_with_boundary_twins() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let v0 = txn.add_vertex([0.0, 0.0, 0.0]);
        let v1 = txn.add_vertex([1.0, 0.0, 0.0]);
        let v2 = txn.add_vertex([0.0, 1.0, 0.0]);
        let face = txn
            .add_face(&[v0, v1, v2])
            .expect("triangle face creation should succeed");
        let changes = txn.finish();

        assert_eq!(changes.created_faces, vec![face]);
        assert_eq!(changes.created_half_edges.len(), 6);
        assert_eq!(
            mesh.faces.get(face.as_id()).map(|record| record.degree),
            Some(3)
        );
        assert_eq!(
            mesh.half_edges
                .iter()
                .filter(|(_, edge)| edge.face == FaceId::OUTSIDE)
                .count(),
            3
        );
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn add_face_reuses_boundary_half_edges_when_possible() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("source mesh should build");
        let vertices = mesh.vertices().collect::<Vec<_>>();

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let face = txn
            .add_face(&[vertices[0], vertices[2], vertices[1]])
            .expect("reversed triangle should consume existing boundary");
        let changes = txn.finish();

        assert_eq!(changes.created_faces, vec![face]);
        assert!(changes.created_half_edges.is_empty());
        assert_eq!(mesh.faces.len(), 2);
        assert_eq!(
            mesh.half_edges
                .iter()
                .filter(|(_, edge)| edge.face == FaceId::OUTSIDE)
                .count(),
            0
        );
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn add_face_rejects_invalid_input() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let v0 = txn.add_vertex([0.0, 0.0, 0.0]);
        let v1 = txn.add_vertex([1.0, 0.0, 0.0]);
        let v2 = txn.add_vertex([0.0, 1.0, 0.0]);
        let stale = VertexId::from(Id::new(999, NonZeroU32::MIN));

        let err = txn
            .add_face(&[v0, v1, stale])
            .expect_err("stale vertex should fail");
        assert_eq!(err, AddFaceError::VertexNotLive { vertex: 999 });

        let err = txn
            .add_face(&[v0, v1, v2, v1])
            .expect_err("repeated vertex should fail");
        assert_eq!(err, AddFaceError::RepeatedVertex { vertex: v1.index() });
    }

    #[test]
    fn add_face_rejects_non_manifold_edge() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("source mesh should build");
        let vertices = mesh.vertices().collect::<Vec<_>>();
        {
            let mut txn = mesh.edit_with(ChangeSetBuilder::new());
            let _ = txn
                .add_face(&[vertices[0], vertices[2], vertices[1]])
                .expect("second face should close boundary");
            let _ = txn.finish();
        }

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let err = txn
            .add_face(&[vertices[0], vertices[1], vertices[2]])
            .expect_err("third face on same edge set should fail");
        assert!(matches!(err, AddFaceError::NonManifoldEdge { .. }));
    }

    #[test]
    fn outgoing_index_queries_update_after_topology_edits() {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([0.0, 1.0, 0.0]);
        let v3 = mesh.add_vertex([1.0, 1.0, 0.0]);

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let _ = txn.add_face(&[v0, v1, v2]).expect("first face should add");
        txn.assert_outgoing_index_consistent();
        assert!(txn.find_boundary_half_edge(v1, v0).is_some());

        let _ = txn.add_face(&[v1, v3, v2]).expect("second face should add");
        txn.assert_outgoing_index_consistent();
        assert!(txn.find_boundary_half_edge(v1, v2).is_none());

        let shared = find_half_edge(txn.mesh(), v1.index(), v2.index()).expect("shared edge");
        let from = txn
            .mesh()
            .from_vertex(shared)
            .expect("shared edge should have source");
        let to = txn
            .mesh()
            .to_vertex(shared)
            .expect("shared edge should have destination");
        assert!(txn.has_undirected_edge(from, to));
        let _ = txn
            .split_edge(shared, &PropagatePolicy::default())
            .expect("split should succeed");
        txn.assert_outgoing_index_consistent();
        assert!(!txn.has_undirected_edge(from, to));
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
    fn delete_edges_rejects_stale_half_edge() {
        let mut mesh = two_tri_strip_mesh();
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let stale = HalfEdgeId::from(Id::new(999, NonZeroU32::MIN));
        let err = txn
            .delete_edges(&[stale], DeletePolicy::CleanupIsolated)
            .expect_err("stale half-edge should fail");
        assert_eq!(err, DeleteEdgesError::HalfEdgeNotLive { half_edge: 999 });
    }

    #[test]
    fn delete_edges_rejects_non_canonical_orientation() {
        let mut mesh = two_tri_strip_mesh();
        let edge = canonical_interior_edges(&mesh)
            .first()
            .copied()
            .expect("strip should have interior edge");
        let twin = mesh.twin(edge).expect("interior edge should have twin");
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let err = txn
            .delete_edges(&[twin], DeletePolicy::CleanupIsolated)
            .expect_err("non-canonical orientation should fail");
        assert_eq!(err, DeleteEdgesError::NonCanonicalEdgeSet);
    }

    #[test]
    fn delete_edges_rejects_non_canonical_order() {
        let mut mesh = three_tri_strip_mesh();
        let edges = canonical_interior_edges(&mesh);
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let err = txn
            .delete_edges(&[edges[1], edges[0]], DeletePolicy::CleanupIsolated)
            .expect_err("unsorted edge set should fail");
        assert_eq!(err, DeleteEdgesError::NonCanonicalEdgeSet);
    }

    #[test]
    fn delete_edges_single_interior_edge_deletes_incident_faces() {
        let mut mesh = two_tri_strip_mesh();
        let edge = canonical_interior_edges(&mesh)
            .first()
            .copied()
            .expect("strip should have interior edge");
        let changes = mesh
            .delete_edges(&[edge], DeletePolicy::CleanupIsolated)
            .expect("interior edge delete should succeed");
        assert_eq!(mesh.faces().count(), 0);
        assert_eq!(mesh.half_edges.len(), 0);
        assert_eq!(mesh.vertices().count(), 0);
        assert_eq!(changes.deleted_faces.len(), 2);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn delete_edges_boundary_edge_deletes_single_face() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("triangle mesh should build");
        let edge = canonical_boundary_edge(&mesh);
        let changes = mesh
            .delete_edges(&[edge], DeletePolicy::KeepIsolated)
            .expect("boundary edge delete should succeed");
        assert_eq!(mesh.faces().count(), 0);
        assert_eq!(mesh.half_edges.len(), 0);
        assert_eq!(mesh.vertices().count(), 3);
        assert_eq!(changes.deleted_faces.len(), 1);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn delete_edges_adjacent_multi_edge_deletes_union_of_incident_faces() {
        let mut mesh = three_tri_strip_mesh();
        let edges = canonical_interior_edges(&mesh);
        assert_eq!(edges.len(), 2);
        let changes = mesh
            .delete_edges(&[edges[0], edges[1]], DeletePolicy::CleanupIsolated)
            .expect("multi-edge delete should succeed");
        assert_eq!(mesh.faces().count(), 0);
        assert_eq!(mesh.half_edges.len(), 0);
        assert_eq!(mesh.vertices().count(), 0);
        assert_eq!(changes.deleted_faces.len(), 3);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn delete_vertices_rejects_non_canonical_input() {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let err = mesh
            .delete_vertices(&[v1, v0])
            .expect_err("non-canonical vertex set must fail");
        assert_eq!(err, DeleteVerticesError::NonCanonicalVertexSet);
    }

    #[test]
    fn delete_vertices_rejects_stale_vertex() {
        let mut mesh = Mesh::new();
        let stale = VertexId::from(Id::new(999, NonZeroU32::MIN));
        let err = mesh
            .delete_vertices(&[stale])
            .expect_err("stale vertex must fail");
        assert_eq!(err, DeleteVerticesError::VertexNotLive { vertex: 999 });
    }

    #[test]
    fn delete_vertices_rejects_non_isolated_vertex() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &crate::mesh::BuildParams::default(),
        )
        .expect("triangle build should succeed");
        let vertex = mesh.vertices().next().expect("vertex should exist");
        let err = mesh
            .delete_vertices(&[vertex])
            .expect_err("non-isolated vertex must fail");
        assert_eq!(
            err,
            DeleteVerticesError::VertexNotIsolated {
                vertex: vertex.index()
            }
        );
    }

    #[test]
    fn delete_vertices_removes_isolated_vertices_and_tracks_changes() {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([2.0, 0.0, 0.0]);
        let changes = mesh
            .delete_vertices(&[v0, v2])
            .expect("isolated vertices should delete");
        assert_eq!(mesh.vertices().count(), 1);
        assert_eq!(mesh.vertices().next(), Some(v1));
        assert_eq!(changes.deleted_vertices, vec![v0, v2]);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn outgoing_index_consistent_after_delete_vertices() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let v0 = txn.add_vertex([0.0, 0.0, 0.0]);
        let v1 = txn.add_vertex([1.0, 0.0, 0.0]);
        let mut vertices = vec![v1, v0];
        vertices.sort_unstable();
        txn.delete_vertices(&vertices)
            .expect("delete_vertices should succeed");
        txn.assert_outgoing_index_consistent();
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
    fn outgoing_index_consistent_after_delete_faces() {
        let (mut mesh, faces) = closed_box_mesh();
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        txn.delete_faces(&[faces[0]], DeletePolicy::CleanupIsolated)
            .expect("delete_faces should succeed");
        txn.assert_outgoing_index_consistent();
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
    fn finish_sorts_and_dedups_change_lists() {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([2.0, 0.0, 0.0]);
        let f0 = FaceId::from(Id::new(3, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        let h0 = HalfEdgeId::from(Id::new(6, NonZeroU32::MIN));
        let h1 = HalfEdgeId::from(Id::new(2, NonZeroU32::MIN));

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
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
        let mut changes = txn.finish();

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
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let _ = txn.add_vertex([0.0, 0.0, 0.0]);
        assert_eq!(txn.mesh().vertices().count(), 1);
        let _ = txn.finish();
    }

    #[test]
    fn dropping_edit_scope_keeps_eager_mesh_mutations() {
        let mut mesh = Mesh::new();
        let before = mesh.revision();
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let vertex = txn.add_vertex([0.0, 1.0, 2.0]);
        drop(txn);

        assert_eq!(mesh.vertex_position(vertex), Some(&[0.0, 1.0, 2.0]));
        assert_eq!(mesh.revision(), before);
    }

    #[test]
    fn mesh_revision_increments_once_per_finished_edit_scope() {
        let mut mesh = Mesh::new();
        assert_eq!(mesh.revision().value(), 0);

        let _ = mesh.edit_with(ChangeSetBuilder::new()).finish();
        assert_eq!(mesh.revision().value(), 1);

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let _ = txn.add_vertex([1.0, 0.0, 0.0]);
        let _ = txn.finish();
        assert_eq!(mesh.revision().value(), 2);
    }

    #[test]
    fn txn_can_record_deleted_topology_ids() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        let face = FaceId::from(Id::new(5, NonZeroU32::MIN));
        let edge = HalfEdgeId::from(Id::new(6, NonZeroU32::MIN));
        let vertex = VertexId::from(Id::new(7, NonZeroU32::MIN));
        txn.record_deleted_face(face);
        txn.record_deleted_half_edge(edge);
        txn.record_deleted_vertex(vertex);
        let mut changes = txn.finish();

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
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());

        let face_a = FaceId::from(Id::new(9, NonZeroU32::MIN));
        let face_b = FaceId::from(Id::new(4, NonZeroU32::MIN));
        let edge_a = HalfEdgeId::from(Id::new(8, NonZeroU32::MIN));
        let edge_b = HalfEdgeId::from(Id::new(1, NonZeroU32::MIN));
        txn.record_created_face(face_a);
        txn.record_created_face(face_b);
        txn.record_created_half_edge(edge_a);
        txn.record_created_half_edge(edge_b);
        let changes = txn.finish();

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

        let mut txn = mesh.edit_with(ChangeSetBuilder::new());
        txn.record_created_face(face);
        txn.record_created_half_edge(half_edge);
        let changes = txn.finish();

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
