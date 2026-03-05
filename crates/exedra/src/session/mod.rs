// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit mesh transactions and deterministic change summaries.

use alloc::vec::Vec;
use core::fmt;

use understory_dirty::{Channel, DirtySet as UnderstoryDirtySet};

use crate::sorted_merge::for_each_count_join;
use crate::{CornerId, FaceId, HalfEdge, HalfEdgeId, Id, Mesh, VertexId, attr};
use propagation::{
    SplitEdgeUvSources, capture_edge_tags, clear_edge_tags, propagate_split_edge_corner_uvs,
    propagate_split_edge_edge_attrs, propagate_split_face_diagonal_uvs,
    split_face_diagonal_sharpness,
};

const DIRTY_FACES_CHANNEL: Channel = Channel::new(0);
const DIRTY_VERTICES_CHANNEL: Channel = Channel::new(1);
const DIRTY_CORNERS_CHANNEL: Channel = Channel::new(2);

#[derive(Copy, Clone, Debug)]
struct OutgoingAdj {
    from: VertexId,
    to: VertexId,
    half_edge: HalfEdgeId,
    face: FaceId,
}

mod attrs;
mod bookkeeping;
mod propagation;

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
/// [`EditSession::commit`](crate::EditSession::commit).
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
/// Returned by [`EditSession::commit`] and by convenience wrappers such as
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
/// Passed to [`Mesh::delete_faces`] or [`EditSession::delete_faces`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum DeletePolicy {
    /// Remove isolated vertices after face deletion.
    #[default]
    CleanupIsolated,
    /// Keep isolated vertices with `out = HalfEdgeId::INVALID`.
    KeepIsolated,
}

/// Structured face-deletion error from [`Mesh::delete_faces`] and
/// [`EditSession::delete_faces`].
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

/// Structured edge-deletion error from [`Mesh::delete_edges`] and
/// [`EditSession::delete_edges`].
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

/// Structured vertex-deletion error from [`Mesh::delete_vertices`] and
/// [`EditSession::delete_vertices`].
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

/// Structured edge-split error from [`EditSession::split_edge`].
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

/// Structured face-split error from [`EditSession::split_face`].
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

/// Structured face-creation error from [`EditSession::add_face`].
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

/// Single-writer transaction over a mesh.
///
/// Mutations are applied eagerly to the underlying mesh. Dropping a transaction
/// does not roll back mesh changes; it only discards accumulated bookkeeping.
///
/// Acquire via [`Mesh::begin`], apply mutating operations, then finish with
/// [`EditSession::commit`] (or [`EditSession::abort`] to drop bookkeeping only).
#[derive(Debug)]
pub struct EditSession<'a> {
    mesh: &'a mut Mesh,
    propagate_policy: PropagatePolicy,
    outgoing_index: Vec<OutgoingAdj>,
    outgoing_index_valid: bool,
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
    /// Call [`EditSession::commit`] to materialize a [`ChangeSet`], or [`EditSession::abort`]
    /// to explicitly discard bookkeeping without rollback.
    pub fn begin(&mut self) -> EditSession<'_> {
        EditSession {
            mesh: self,
            propagate_policy: PropagatePolicy::default(),
            outgoing_index: Vec::new(),
            outgoing_index_valid: false,
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
    /// This is a convenience wrapper over [`EditSession::delete_faces`] + [`EditSession::commit`].
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

    /// Deletes a canonical set of undirected edges in one committed transaction.
    ///
    /// This is a convenience wrapper over [`EditSession::delete_edges`] + [`EditSession::commit`].
    pub fn delete_edges(
        &mut self,
        edges: &[HalfEdgeId],
        policy: DeletePolicy,
    ) -> Result<ChangeSet, DeleteEdgesError> {
        let mut txn = self.begin();
        txn.delete_edges(edges, policy)?;
        Ok(txn.commit())
    }

    /// Deletes a canonical set of isolated vertices in one committed transaction.
    ///
    /// This is a convenience wrapper over [`EditSession::delete_vertices`] + [`EditSession::commit`].
    pub fn delete_vertices(
        &mut self,
        vertices: &[VertexId],
    ) -> Result<ChangeSet, DeleteVerticesError> {
        let mut txn = self.begin();
        txn.delete_vertices(vertices)?;
        Ok(txn.commit())
    }
}

impl EditSession<'_> {
    fn ensure_outgoing_index(&mut self) -> &[OutgoingAdj] {
        if !self.outgoing_index_valid {
            self.outgoing_index = build_outgoing_index(self.mesh);
            self.outgoing_index_valid = true;
        }
        &self.outgoing_index
    }

    fn invalidate_outgoing_index(&mut self) {
        self.outgoing_index_valid = false;
    }

    fn vertex_has_incident_half_edge(&mut self, vertex: VertexId) -> bool {
        let index = self.ensure_outgoing_index();
        vertex_has_incident_half_edge_in_index(index, vertex)
    }

    fn has_undirected_edge(&mut self, a: VertexId, b: VertexId) -> bool {
        let index = self.ensure_outgoing_index();
        has_undirected_edge_in_index(index, a, b)
    }

    fn find_boundary_half_edge(&mut self, from: VertexId, to: VertexId) -> Option<HalfEdgeId> {
        let index = self.ensure_outgoing_index();
        find_boundary_half_edge_in_index(index, from, to)
    }

    /// Adds one interior polygon face loop from live vertex IDs.
    ///
    /// For each directed loop edge `from -> to`, behavior is:
    /// - reuse an existing matching OUTSIDE half-edge when present,
    /// - otherwise create a new interior half-edge and a new OUTSIDE twin.
    ///
    /// This supports edit operators that need to fill holes or create side-wall
    /// strips while preserving transaction bookkeeping and deterministic dirty
    /// summaries.
    pub fn add_face(&mut self, loop_vertices: &[VertexId]) -> Result<FaceId, AddFaceError> {
        if loop_vertices.len() < 3 {
            return Err(AddFaceError::LoopTooShort);
        }
        for &vertex in loop_vertices {
            if self.mesh.vertices.get(vertex.as_id()).is_none() {
                return Err(AddFaceError::VertexNotLive {
                    vertex: vertex.index(),
                });
            }
        }
        for pair in loop_vertices.windows(2) {
            if pair[0] == pair[1] {
                return Err(AddFaceError::ZeroLengthEdge {
                    from: pair[0].index(),
                    to: pair[1].index(),
                });
            }
        }
        if loop_vertices.first() == loop_vertices.last() {
            let first = loop_vertices[0];
            return Err(AddFaceError::ZeroLengthEdge {
                from: first.index(),
                to: first.index(),
            });
        }
        for i in 0..loop_vertices.len() {
            let a = loop_vertices[i];
            for &b in &loop_vertices[(i + 1)..] {
                if a == b {
                    return Err(AddFaceError::RepeatedVertex { vertex: a.index() });
                }
            }
        }

        let mut reuse_boundary = Vec::<Option<HalfEdgeId>>::with_capacity(loop_vertices.len());
        for i in 0..loop_vertices.len() {
            let from = loop_vertices[i];
            let to = loop_vertices[(i + 1) % loop_vertices.len()];
            let reuse = self.find_boundary_half_edge(from, to);
            if reuse.is_none() && self.has_undirected_edge(from, to) {
                return Err(AddFaceError::NonManifoldEdge {
                    a: u32::min(from.index(), to.index()),
                    b: u32::max(from.index(), to.index()),
                });
            }
            reuse_boundary.push(reuse);
        }

        let degree =
            u32::try_from(loop_vertices.len()).expect("face loop length should fit into u32");
        let face = FaceId::from(self.mesh.faces.insert(crate::Face {
            edge: HalfEdgeId::INVALID,
            degree,
        }));
        let mut loop_half_edges = Vec::<HalfEdgeId>::with_capacity(loop_vertices.len());
        for i in 0..loop_vertices.len() {
            let from = loop_vertices[i];
            let to = loop_vertices[(i + 1) % loop_vertices.len()];
            if let Some(boundary) = reuse_boundary[i] {
                self.mesh
                    .half_edges
                    .get_mut(boundary.as_id())
                    .expect("preflight-validated boundary half-edge must be live")
                    .face = face;
                loop_half_edges.push(boundary);
                self.mark_corner_dirty(boundary);
                continue;
            }

            let interior = HalfEdgeId::from(self.mesh.half_edges.insert(HalfEdge {
                to,
                face,
                next: HalfEdgeId::INVALID,
                twin: HalfEdgeId::INVALID,
            }));
            let boundary = HalfEdgeId::from(self.mesh.half_edges.insert(HalfEdge {
                to: from,
                face: FaceId::OUTSIDE,
                next: HalfEdgeId::INVALID,
                twin: interior,
            }));
            self.mesh
                .half_edges
                .get_mut(interior.as_id())
                .expect("new interior half-edge must be live")
                .twin = boundary;
            loop_half_edges.push(interior);
            self.record_created_half_edge(interior);
            self.record_created_half_edge(boundary);
            self.mark_corner_dirty(interior);
            self.mark_corner_dirty(boundary);
        }

        for i in 0..loop_half_edges.len() {
            let current = loop_half_edges[i];
            let next = loop_half_edges[(i + 1) % loop_half_edges.len()];
            self.mesh
                .half_edges
                .get_mut(current.as_id())
                .expect("new face loop half-edge must be live")
                .next = next;
        }
        self.mesh
            .faces
            .get_mut(face.as_id())
            .expect("new face must be live")
            .edge = loop_half_edges[0];

        stitch_outside_loops(self.mesh);
        self.mesh.attrs.sync_capacities(
            self.mesh.vertices.slot_count(),
            self.mesh.faces.slot_count(),
            self.mesh.half_edges.slot_count(),
        );

        self.record_created_face(face);
        self.mark_face_dirty(face);
        for &vertex in loop_vertices {
            self.mark_vertex_dirty(vertex);
        }
        for &corner in &loop_half_edges {
            self.mark_corner_dirty(corner);
            if let Some(twin) = self.mesh.twin(corner) {
                self.mark_corner_dirty(twin);
            }
        }

        self.invalidate_outgoing_index();
        Ok(face)
    }

    /// Splits an undirected edge by inserting a new midpoint vertex.
    ///
    /// Topology update:
    /// - original directed pair `(h, twin(h))` is replaced by
    ///   `(h -> child_h)` and `(twin(h) -> child_twin)`,
    /// - each adjacent face loop gains one corner,
    /// - midpoint vertex is inserted and used by the new child half-edges.
    ///
    /// Attribute/default propagation in v0.1:
    /// - vertex position follows [`PropagatePolicy::position`]
    ///   (`Midpoint`/`WeightedMidpoint` currently both use midpoint),
    /// - edge seam inherited to both child undirected edges when
    ///   policy is [`EdgeAttrPropagation::Inherit`] or
    ///   [`EdgeAttrPropagation::DecayOnSplit`], cleared for `Clear`,
    /// - edge sharpness propagation is policy-dependent:
    ///   - [`EdgeAttrPropagation::Inherit`]: preserve source value,
    ///   - [`EdgeAttrPropagation::DecayOnSplit`]: decay by `1.0` on split
    ///     (clamped to `0.0`),
    ///   - [`EdgeAttrPropagation::Clear`]: reset to `0.0`,
    /// - new corner UVs use midpoint interpolation when policy is
    ///   [`UvPropagation::Midpoint`] and side-copy for `CopyFromSide`,
    /// - face attributes are unchanged.
    pub fn split_edge(&mut self, half_edge: HalfEdgeId) -> Result<VertexId, SplitEdgeError> {
        let twin = self
            .mesh
            .twin(half_edge)
            .ok_or(SplitEdgeError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
        if self.mesh.half_edges.get(half_edge.as_id()).is_none()
            || self.mesh.half_edges.get(twin.as_id()).is_none()
        {
            return Err(SplitEdgeError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            });
        }

        let from = self
            .mesh
            .from_vertex(half_edge)
            .ok_or(SplitEdgeError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
        let to = self
            .mesh
            .to_vertex(half_edge)
            .ok_or(SplitEdgeError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
        let h_next = self
            .mesh
            .next(half_edge)
            .ok_or(SplitEdgeError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
        let t_next = self
            .mesh
            .next(twin)
            .ok_or(SplitEdgeError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;

        let from_pos = *self
            .mesh
            .vertex_position(from)
            .ok_or(SplitEdgeError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
        let to_pos = *self
            .mesh
            .vertex_position(to)
            .ok_or(SplitEdgeError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
        let midpoint = [
            0.5 * (from_pos[0] + to_pos[0]),
            0.5 * (from_pos[1] + to_pos[1]),
            0.5 * (from_pos[2] + to_pos[2]),
        ];

        let new_vertex = self.add_vertex(match self.propagate_policy.position {
            PositionPropagation::Midpoint | PositionPropagation::WeightedMidpoint => midpoint,
        });

        let h_face = self.mesh.face(half_edge).expect("validated edge face");
        let t_face = self.mesh.face(twin).expect("validated twin face");
        let old_canonical_edge = core::cmp::min(half_edge, twin);

        // Capture source corner UVs before rewiring. Corner UV is stored at the
        // destination corner vertex per-face.
        let old_uv_h = self.corner_uv(half_edge);
        let old_uv_t = self.corner_uv(twin);
        let uv_a_fh = corner_uv_for_face_to_vertex(self.mesh, h_face, from).unwrap_or([0.0, 0.0]);
        let uv_b_ft = corner_uv_for_face_to_vertex(self.mesh, t_face, to).unwrap_or([0.0, 0.0]);

        let child_h = HalfEdgeId::from(self.mesh.half_edges.insert(HalfEdge {
            to,
            face: h_face,
            next: h_next,
            twin,
        }));
        let child_t = HalfEdgeId::from(self.mesh.half_edges.insert(HalfEdge {
            to: from,
            face: t_face,
            next: t_next,
            twin: half_edge,
        }));

        self.mesh
            .half_edges
            .get_mut(half_edge.as_id())
            .expect("validated half-edge")
            .to = new_vertex;
        self.mesh
            .half_edges
            .get_mut(half_edge.as_id())
            .expect("validated half-edge")
            .next = child_h;
        self.mesh
            .half_edges
            .get_mut(half_edge.as_id())
            .expect("validated half-edge")
            .twin = child_t;

        self.mesh
            .half_edges
            .get_mut(twin.as_id())
            .expect("validated twin")
            .to = new_vertex;
        self.mesh
            .half_edges
            .get_mut(twin.as_id())
            .expect("validated twin")
            .next = child_t;
        self.mesh
            .half_edges
            .get_mut(twin.as_id())
            .expect("validated twin")
            .twin = child_h;
        self.mesh
            .vertices
            .get_mut(new_vertex.as_id())
            .expect("new vertex must be live")
            .out = child_h;
        self.mesh.attrs.sync_capacities(
            self.mesh.vertices.slot_count(),
            self.mesh.faces.slot_count(),
            self.mesh.half_edges.slot_count(),
        );

        self.record_created_half_edge(child_h);
        self.record_created_half_edge(child_t);
        self.mark_corner_dirty(half_edge);
        self.mark_corner_dirty(twin);
        self.mark_corner_dirty(child_h);
        self.mark_corner_dirty(child_t);
        self.mark_vertex_dirty(from);
        self.mark_vertex_dirty(to);
        self.mark_vertex_dirty(new_vertex);

        // Propagate edge-domain authored tags from original undirected edge.
        let edge_tags = capture_edge_tags(self.mesh, old_canonical_edge);
        clear_edge_tags(self.mesh, old_canonical_edge);
        let policy = self.propagate_policy;
        propagate_split_edge_edge_attrs(self, half_edge, child_h, edge_tags, &policy);

        // Propagate corner UVs for new corners.
        if self.mesh.attrs().sparse(attr::CORNER_UV).is_some() {
            let policy = self.propagate_policy;
            propagate_split_edge_corner_uvs(
                self,
                half_edge,
                twin,
                child_h,
                child_t,
                SplitEdgeUvSources {
                    old_uv_h,
                    old_uv_t,
                    uv_a_fh,
                    uv_b_ft,
                },
                &policy,
            );
        }

        // Update cached degree for incident interior faces.
        if h_face != FaceId::OUTSIDE {
            if let Some(face) = self.mesh.faces.get_mut(h_face.as_id()) {
                face.degree = face.degree.saturating_add(1);
            }
            self.mark_face_dirty(h_face);
        }
        if t_face != FaceId::OUTSIDE && t_face != h_face {
            if let Some(face) = self.mesh.faces.get_mut(t_face.as_id()) {
                face.degree = face.degree.saturating_add(1);
            }
            self.mark_face_dirty(t_face);
        }

        self.invalidate_outgoing_index();
        Ok(new_vertex)
    }

    /// Splits one interior face by inserting a diagonal between two corners.
    ///
    /// `corner_a` and `corner_b` must be distinct, non-adjacent corners on
    /// the same interior face loop.
    ///
    /// Propagation/default behavior in v0.1:
    /// - new face inherits source face-region value,
    /// - existing corner UVs are preserved,
    /// - diagonal corner UVs follow [`PropagatePolicy::uv`] while preserving
    ///   sparse missingness (missing sources stay missing),
    /// - diagonal edge sharpness defaults to smooth for
    ///   [`EdgeAttrPropagation::Inherit`] / [`EdgeAttrPropagation::Clear`],
    ///   and only derives from nearby authored sharpness under
    ///   [`EdgeAttrPropagation::DecayOnSplit`].
    pub fn split_face(
        &mut self,
        corner_a: CornerId,
        corner_b: CornerId,
    ) -> Result<FaceId, SplitFaceError> {
        if corner_a == corner_b {
            return Err(SplitFaceError::IdenticalCorners);
        }
        if self.mesh.half_edges.get(corner_a.as_id()).is_none() {
            return Err(SplitFaceError::CornerNotLive {
                corner: corner_a.index(),
            });
        }
        if self.mesh.half_edges.get(corner_b.as_id()).is_none() {
            return Err(SplitFaceError::CornerNotLive {
                corner: corner_b.index(),
            });
        }
        let face = self
            .mesh
            .face(corner_a)
            .ok_or(SplitFaceError::CornerNotLive {
                corner: corner_a.index(),
            })?;
        let face_b = self
            .mesh
            .face(corner_b)
            .ok_or(SplitFaceError::CornerNotLive {
                corner: corner_b.index(),
            })?;
        if face != face_b {
            return Err(SplitFaceError::CornersNotOnSameFace);
        }
        if face == FaceId::OUTSIDE {
            return Err(SplitFaceError::OutsideFaceNotAllowed);
        }

        let loop_edges = self.mesh.face_loop(face).collect::<Vec<_>>();
        let degree = loop_edges.len();
        let ia = loop_edges
            .iter()
            .position(|&corner| corner == corner_a)
            .ok_or(SplitFaceError::CornersNotOnSameFace)?;
        let ib = loop_edges
            .iter()
            .position(|&corner| corner == corner_b)
            .ok_or(SplitFaceError::CornersNotOnSameFace)?;
        let a_next_index = (ia + 1) % degree;
        let b_next_index = (ib + 1) % degree;
        if a_next_index == ib || b_next_index == ia {
            return Err(SplitFaceError::AdjacentCorners);
        }

        let corner_a_next = loop_edges[a_next_index];
        let corner_b_next = loop_edges[b_next_index];
        let vertex_a = self
            .mesh
            .to_vertex(corner_a)
            .expect("validated corner has destination vertex");
        let vertex_b = self
            .mesh
            .to_vertex(corner_b)
            .expect("validated corner has destination vertex");

        // Ordered path from `corner_a_next` through `corner_b` (inclusive) becomes new face.
        let mut new_face_path = Vec::<CornerId>::with_capacity(degree);
        let mut cursor = corner_a_next;
        for _ in 0..degree {
            new_face_path.push(cursor);
            if cursor == corner_b {
                break;
            }
            cursor = self
                .mesh
                .next(cursor)
                .expect("face loop should have valid next pointers");
        }
        debug_assert_eq!(
            new_face_path.last().copied(),
            Some(corner_b),
            "face loop path should reach target corner"
        );

        let diagonal_a_b = HalfEdgeId::from(self.mesh.half_edges.insert(HalfEdge {
            to: vertex_b,
            face,
            next: corner_b_next,
            twin: HalfEdgeId::INVALID,
        }));
        let new_face = FaceId::from(self.mesh.faces.insert(crate::Face {
            edge: corner_b,
            degree: 0,
        }));
        let diagonal_b_a = HalfEdgeId::from(self.mesh.half_edges.insert(HalfEdge {
            to: vertex_a,
            face: new_face,
            next: corner_a_next,
            twin: diagonal_a_b,
        }));
        self.mesh
            .half_edges
            .get_mut(diagonal_a_b.as_id())
            .expect("new diagonal half-edge must be live")
            .twin = diagonal_b_a;

        self.mesh
            .half_edges
            .get_mut(corner_a.as_id())
            .expect("validated corner must be live")
            .next = diagonal_a_b;
        self.mesh
            .half_edges
            .get_mut(corner_b.as_id())
            .expect("validated corner must be live")
            .next = diagonal_b_a;

        for &corner in &new_face_path {
            self.mesh
                .half_edges
                .get_mut(corner.as_id())
                .expect("validated loop corner must be live")
                .face = new_face;
        }
        let new_face_path_len =
            u32::try_from(new_face_path.len()).expect("face path length should fit u32");
        let face_degree = u32::try_from(degree).expect("face degree should fit u32");
        let new_face_degree = new_face_path_len + 1;
        let old_face_degree = face_degree - new_face_path_len + 1;
        {
            let source_face = self
                .mesh
                .faces
                .get_mut(face.as_id())
                .expect("source face must be live");
            source_face.edge = corner_a;
            source_face.degree = old_face_degree;
        }
        self.mesh
            .faces
            .get_mut(new_face.as_id())
            .expect("new face must be live")
            .degree = new_face_degree;

        if let Some(region_layer) = self.mesh.attrs_mut().dense_mut(attr::FACE_REGION) {
            let region = region_layer.get(face.as_id()).copied().unwrap_or(0);
            let _ = region_layer.set(new_face.as_id(), region);
        }

        if self.mesh.attrs().sparse(attr::CORNER_UV).is_some() {
            let uv_a = self.corner_uv(corner_a);
            let uv_b = self.corner_uv(corner_b);
            let policy = self.propagate_policy;
            propagate_split_face_diagonal_uvs(
                self,
                diagonal_a_b,
                diagonal_b_a,
                uv_a,
                uv_b,
                &policy,
            );
        }

        // v0.1 diagonal sharpness derives from the two existing directed
        // edges that leave the split endpoints (`corner_a`, `corner_b`).
        let source_sharp = self
            .edge_sharpness(corner_a)
            .unwrap_or(0.0)
            .max(self.edge_sharpness(corner_b).unwrap_or(0.0));
        let policy = self.propagate_policy;
        let diagonal_sharp = split_face_diagonal_sharpness(source_sharp, &policy);
        if diagonal_sharp > 0.0 {
            let _ = self.set_edge_sharpness(diagonal_a_b, diagonal_sharp);
        }

        self.mesh.attrs.sync_capacities(
            self.mesh.vertices.slot_count(),
            self.mesh.faces.slot_count(),
            self.mesh.half_edges.slot_count(),
        );

        self.record_created_face(new_face);
        self.record_created_half_edge(diagonal_a_b);
        self.record_created_half_edge(diagonal_b_a);
        self.mark_face_dirty(face);
        self.mark_face_dirty(new_face);
        self.mark_corner_dirty(corner_a);
        self.mark_corner_dirty(corner_b);
        self.mark_corner_dirty(diagonal_a_b);
        self.mark_corner_dirty(diagonal_b_a);
        // Conservative dirty marking: include the full original loop to keep
        // incremental consumers correct even when only face ownership changed.
        for corner in loop_edges {
            if let Some(to) = self.mesh.to_vertex(corner) {
                self.mark_vertex_dirty(to);
            }
            self.mark_corner_dirty(corner);
        }

        self.invalidate_outgoing_index();
        Ok(new_face)
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
        self.invalidate_outgoing_index();
        Ok(())
    }

    /// Deletes canonical undirected edges by deleting all incident interior faces.
    ///
    /// `edges` must be canonical (sorted, deduplicated) and contain only live
    /// half-edge IDs where each ID is the canonical representative
    /// (`min(edge, twin)`) of an undirected edge.
    ///
    /// Each selected edge contributes its interior incident face(s), and this
    /// kernel delegates topology mutation to [`EditSession::delete_faces`].
    pub fn delete_edges(
        &mut self,
        edges: &[HalfEdgeId],
        policy: DeletePolicy,
    ) -> Result<(), DeleteEdgesError> {
        let mut faces = Vec::<FaceId>::new();
        let mut previous = None;

        for &half_edge in edges {
            if let Some(prev) = previous
                && prev >= half_edge
            {
                return Err(DeleteEdgesError::NonCanonicalEdgeSet);
            }
            previous = Some(half_edge);

            let twin = self
                .mesh
                .twin(half_edge)
                .ok_or(DeleteEdgesError::HalfEdgeNotLive {
                    half_edge: half_edge.index(),
                })?;
            if core::cmp::min(half_edge, twin) != half_edge {
                return Err(DeleteEdgesError::NonCanonicalEdgeSet);
            }

            let face = self
                .mesh
                .face(half_edge)
                .ok_or(DeleteEdgesError::HalfEdgeNotLive {
                    half_edge: half_edge.index(),
                })?;
            let twin_face = self
                .mesh
                .face(twin)
                .ok_or(DeleteEdgesError::HalfEdgeNotLive {
                    half_edge: half_edge.index(),
                })?;
            if face != FaceId::OUTSIDE {
                faces.push(face);
            }
            if twin_face != FaceId::OUTSIDE {
                faces.push(twin_face);
            }
            if face == FaceId::OUTSIDE && twin_face == FaceId::OUTSIDE {
                return Err(DeleteEdgesError::EdgeHasNoInteriorFace {
                    half_edge: half_edge.index(),
                });
            }
        }

        sort_dedup(&mut faces);
        self.delete_faces(&faces, policy)
            .map_err(DeleteEdgesError::FaceDeleteFailed)
    }

    /// Deletes isolated vertices from the mesh.
    ///
    /// `vertices` must be canonical (sorted, deduplicated), contain only live
    /// vertex IDs, and each vertex must have no incident half-edges.
    pub fn delete_vertices(&mut self, vertices: &[VertexId]) -> Result<(), DeleteVerticesError> {
        if !is_canonical_vertex_set(vertices) {
            return Err(DeleteVerticesError::NonCanonicalVertexSet);
        }
        for &vertex in vertices {
            let Some(record) = self.mesh.vertices.get(vertex.as_id()) else {
                return Err(DeleteVerticesError::VertexNotLive {
                    vertex: vertex.index(),
                });
            };
            if record.out != HalfEdgeId::INVALID || self.vertex_has_incident_half_edge(vertex) {
                return Err(DeleteVerticesError::VertexNotIsolated {
                    vertex: vertex.index(),
                });
            }
        }
        for &vertex in vertices {
            let removed = self.mesh.vertices.remove(vertex.as_id());
            debug_assert!(removed.is_some(), "validated vertex should remove");
            self.record_deleted_vertex(vertex);
        }
        self.invalidate_outgoing_index();
        Ok(())
    }
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

fn is_canonical_vertex_set(vertices: &[VertexId]) -> bool {
    vertices.windows(2).all(|pair| pair[0] < pair[1])
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

fn find_outgoing_half_edge_linear_scan(mesh: &Mesh, vertex: VertexId) -> Option<HalfEdgeId> {
    mesh.half_edges.iter().find_map(|(id, _)| {
        let half_edge = HalfEdgeId::from(id);
        (half_edge_vertices(mesh, half_edge).is_some_and(|(from, _)| from == vertex))
            .then_some(half_edge)
    })
}

fn find_outgoing_half_edge(outgoing_index: &[OutgoingAdj], vertex: VertexId) -> Option<HalfEdgeId> {
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

fn should_use_global_outgoing_index(affected_vertices: usize, total_half_edges: usize) -> bool {
    if affected_vertices == 0 || total_half_edges == 0 {
        return false;
    }
    // Heuristic: switch to global index once the affected set is at least
    // about 1/8 of total half-edges; this avoids expensive repeated linear
    // scans near crossover cases.
    affected_vertices.saturating_mul(8) > total_half_edges
}

fn corner_uv_for_face_to_vertex(mesh: &Mesh, face: FaceId, vertex: VertexId) -> Option<[f32; 2]> {
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
    fn txn_propagate_policy_can_be_overridden() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.begin();
        assert_eq!(txn.propagate_policy(), PropagatePolicy::default());
        txn.set_propagate_policy(PropagatePolicy {
            position: PositionPropagation::WeightedMidpoint,
            uv: UvPropagation::CopyFromSide,
            normal_override: NormalOverridePropagation::Average,
            face_attr: FaceAttrPropagation::CopyAndTag,
            edge_attr: EdgeAttrPropagation::DecayOnSplit,
        });
        assert_eq!(
            txn.propagate_policy().position,
            PositionPropagation::WeightedMidpoint
        );
        assert_eq!(txn.propagate_policy().uv, UvPropagation::CopyFromSide);
        assert_eq!(
            txn.propagate_policy().normal_override,
            NormalOverridePropagation::Average
        );
        assert_eq!(
            txn.propagate_policy().face_attr,
            FaceAttrPropagation::CopyAndTag
        );
        assert_eq!(
            txn.propagate_policy().edge_attr,
            EdgeAttrPropagation::DecayOnSplit
        );
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
        assert_eq!(txn.edge_sharpness(shared), Some(0.0));
        assert!(txn.set_edge_sharpness(shared_twin, 2.5));
        assert_eq!(txn.edge_sharpness(shared), Some(2.5));
        assert_eq!(txn.edge_sharpness(shared_twin), Some(2.5));
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
        let mut txn = mesh.begin();
        let err = txn
            .split_edge(HalfEdgeId::new(99, NonZeroU32::MIN))
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

        let mut txn = mesh.begin();
        let inserted = txn.split_edge(shared).expect("split should succeed");
        let mut changes = txn.commit();

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
            let mut txn = mesh.begin();
            assert!(txn.set_edge_seam(shared, true));
            assert!(txn.set_edge_sharpness(shared, 2.5));
            let _ = txn.commit();
        }

        let mut txn = mesh.begin();
        let _ = txn.split_edge(shared).expect("split should succeed");
        let _ = txn.commit();
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
            let mut txn = mesh.begin();
            assert!(txn.set_edge_seam(shared, true));
            assert!(txn.set_edge_sharpness(shared, 2.5));
            let _ = txn.commit();
        }
        let twin = mesh.twin(shared).expect("shared edge should have twin");
        let mut txn = mesh.begin();
        txn.set_propagate_policy(PropagatePolicy {
            edge_attr: EdgeAttrPropagation::DecayOnSplit,
            ..PropagatePolicy::default()
        });
        let _ = txn.split_edge(shared).expect("split should succeed");
        let _ = txn.commit();
        let child_h = mesh.next(shared).expect("split child should exist");
        let child_t = mesh.next(twin).expect("split child should exist");
        assert_eq!(mesh.edge_sharpness(shared), Some(1.5));
        assert_eq!(mesh.edge_sharpness(child_h), Some(1.5));
        assert_eq!(mesh.edge_sharpness(twin), Some(1.5));
        assert_eq!(mesh.edge_sharpness(child_t), Some(1.5));
        assert!(mesh.validate_deep().is_empty());

        let (mut mesh, shared) = split_source_mesh();
        {
            let mut txn = mesh.begin();
            assert!(txn.set_edge_seam(shared, true));
            assert!(txn.set_edge_sharpness(shared, 2.5));
            let _ = txn.commit();
        }
        let twin = mesh.twin(shared).expect("shared edge should have twin");
        let mut txn = mesh.begin();
        txn.set_propagate_policy(PropagatePolicy {
            edge_attr: EdgeAttrPropagation::Clear,
            ..PropagatePolicy::default()
        });
        let _ = txn.split_edge(shared).expect("split should succeed");
        let _ = txn.commit();
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
            let mut txn = mesh.begin();
            assert!(txn.set_corner_uv(shared, [0.0, 0.25]));
            assert!(txn.set_corner_uv(twin, [1.0, 0.75]));
            let _ = txn.commit();
        }

        let mut txn = mesh.begin();
        let _ = txn.split_edge(shared).expect("split should succeed");
        let _ = txn.commit();
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
            let mut txn = mesh.begin();
            assert!(txn.set_corner_uv(shared, [0.0, 0.25]));
            assert!(txn.set_corner_uv(twin, [1.0, 0.75]));
            let _ = txn.commit();
        }
        let mut txn = mesh.begin();
        txn.set_propagate_policy(PropagatePolicy {
            uv: UvPropagation::CopyFromSide,
            ..PropagatePolicy::default()
        });
        let _ = txn.split_edge(shared).expect("split should succeed");
        let _ = txn.commit();
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

        let mut txn = mesh.begin();
        let inserted = txn.split_edge(boundary).expect("split should succeed");
        let _ = txn.commit();

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

        let mut txn = mesh.begin();
        let new_face = txn
            .split_face(corners[0], corners[2])
            .expect("split should succeed");
        let mut changes = txn.commit();

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
        let mut txn = mesh.begin();
        let err = txn
            .split_face(corners[0], corners[1])
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

        let mut txn = mesh.begin();
        let err = txn
            .split_face(corners[0], corners[0])
            .expect_err("identical corners must fail");
        assert_eq!(err, SplitFaceError::IdenticalCorners);

        let err = txn
            .split_face(corners[0], CornerId::new(999, NonZeroU32::MIN))
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
        let mut txn = open.begin();
        let err = txn
            .split_face(outside_corner_a, outside_corner_b)
            .expect_err("outside face should fail");
        assert_eq!(err, SplitFaceError::OutsideFaceNotAllowed);

        let (mut closed, _) = closed_box_mesh();
        let faces = closed.faces().collect::<Vec<_>>();
        let f0 = faces[0];
        let f1 = faces[1];
        let c0 = closed.face_loop(f0).next().expect("corner 0");
        let c1 = closed.face_loop(f1).next().expect("corner 1");
        let mut txn = closed.begin();
        let err = txn
            .split_face(c0, c1)
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
            let mut txn = mesh.begin();
            assert!(txn.set_corner_uv(corners[0], [0.0, 0.0]));
            assert!(txn.set_corner_uv(corners[2], [2.0, 1.0]));
            let _ = txn.commit();
        }

        let mut txn = mesh.begin();
        txn.set_propagate_policy(PropagatePolicy {
            uv: UvPropagation::CopyFromSide,
            ..txn.propagate_policy()
        });
        let new_face = txn
            .split_face(corners[0], corners[2])
            .expect("split should succeed");
        let changes = txn.commit();
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
            let mut txn = mesh.begin();
            assert!(txn.set_corner_uv(corners[0], [0.5, 0.25]));
            let _ = txn.commit();
        }

        let mut txn = mesh.begin();
        let _ = txn
            .split_face(corners[0], corners[2])
            .expect("split should succeed");
        let changes = txn.commit();
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
            let mut txn = mesh.begin();
            assert!(txn.set_edge_sharpness(corners[0], 3.0));
            assert!(txn.set_edge_sharpness(corners[2], 2.0));
            let _ = txn.commit();
        }

        let mut txn = mesh.begin();
        let _ = txn
            .split_face(corners[0], corners[2])
            .expect("split should succeed");
        let changes = txn.commit();
        let diagonal = changes.created_half_edges[0];
        assert_eq!(mesh.edge_sharpness(diagonal), Some(0.0));

        let (mut mesh, _) = closed_box_mesh();
        let face = mesh.faces().next().expect("face should exist");
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        {
            let mut txn = mesh.begin();
            assert!(txn.set_edge_sharpness(corners[0], 3.0));
            assert!(txn.set_edge_sharpness(corners[2], 2.0));
            let _ = txn.commit();
        }
        let mut txn = mesh.begin();
        txn.set_propagate_policy(PropagatePolicy {
            edge_attr: EdgeAttrPropagation::DecayOnSplit,
            ..PropagatePolicy::default()
        });
        let _ = txn
            .split_face(corners[0], corners[2])
            .expect("split should succeed");
        let changes = txn.commit();
        let diagonal = changes.created_half_edges[0];
        assert_eq!(mesh.edge_sharpness(diagonal), Some(2.0));
    }

    #[test]
    fn add_face_creates_triangle_with_boundary_twins() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.begin();
        let v0 = txn.add_vertex([0.0, 0.0, 0.0]);
        let v1 = txn.add_vertex([1.0, 0.0, 0.0]);
        let v2 = txn.add_vertex([0.0, 1.0, 0.0]);
        let face = txn
            .add_face(&[v0, v1, v2])
            .expect("triangle face creation should succeed");
        let changes = txn.commit();

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

        let mut txn = mesh.begin();
        let face = txn
            .add_face(&[vertices[0], vertices[2], vertices[1]])
            .expect("reversed triangle should consume existing boundary");
        let changes = txn.commit();

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
        let mut txn = mesh.begin();
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
            let mut txn = mesh.begin();
            let _ = txn
                .add_face(&[vertices[0], vertices[2], vertices[1]])
                .expect("second face should close boundary");
            let _ = txn.commit();
        }

        let mut txn = mesh.begin();
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

        let mut txn = mesh.begin();
        let _ = txn.add_face(&[v0, v1, v2]).expect("first face should add");
        assert!(txn.find_boundary_half_edge(v1, v0).is_some());

        let _ = txn.add_face(&[v1, v3, v2]).expect("second face should add");
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
        let _ = txn.split_edge(shared).expect("split should succeed");
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
        let mut txn = mesh.begin();
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
        let mut txn = mesh.begin();
        let err = txn
            .delete_edges(&[twin], DeletePolicy::CleanupIsolated)
            .expect_err("non-canonical orientation should fail");
        assert_eq!(err, DeleteEdgesError::NonCanonicalEdgeSet);
    }

    #[test]
    fn delete_edges_rejects_non_canonical_order() {
        let mut mesh = three_tri_strip_mesh();
        let edges = canonical_interior_edges(&mesh);
        let mut txn = mesh.begin();
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
