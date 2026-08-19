// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Mesh container and core topology traversal over half-edge records.
//!
//! Core invariants:
//! - Face loops are closed `next` cycles.
//! - Every half-edge has a twin; boundary twins use [`FaceId::OUTSIDE`].
//! - Boundary half-edges always have `face == FaceId::OUTSIDE`.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::num::NonZeroU32;

use crate::{
    Arena, CornerId, Face, FaceId, HalfEdge, HalfEdgeId, Id, Vertex, VertexId, attr,
    attributes::{Attributes, Domain},
};

/// Parameters for [`Mesh::from_indexed_triangles`].
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BuildParams {
    /// Optional weld tolerance for deduplicating input positions.
    ///
    /// When `None`, input positions are kept as-is. When `Some(t)`, vertices
    /// within distance `t` are deterministically welded to the earliest match.
    ///
    /// `t` must be finite and non-negative.
    pub weld_tolerance: Option<f32>,
}

/// Invalid face-loop reason reported by [`MeshBuilder::add_face`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FaceLoopErrorKind {
    /// Face loop has fewer than 3 vertices.
    TooShort,
    /// Face loop contains a repeated vertex index.
    RepeatedVertex,
    /// Face loop contains a zero-length edge (`v_i == v_{i+1}`).
    ZeroLengthEdge,
    /// Face loop references a vertex index outside builder vertex storage.
    IndexOutOfBounds {
        /// Out-of-range vertex index value.
        index: u32,
    },
}

impl fmt::Display for FaceLoopErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => f.write_str("face loop has fewer than 3 vertices"),
            Self::RepeatedVertex => f.write_str("face loop contains a repeated vertex"),
            Self::ZeroLengthEdge => f.write_str("face loop contains a zero-length edge"),
            Self::IndexOutOfBounds { index } => {
                write!(f, "face loop index out of bounds: {index}")
            }
        }
    }
}

impl core::error::Error for FaceLoopErrorKind {}

/// Invalid face-attribute reason reported by [`MeshBuilder::add_face_with_attrs`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FaceAttrErrorKind {
    /// Per-edge seam data length did not match the face degree.
    EdgeSeamLengthMismatch {
        /// Expected number of loop edges.
        expected: usize,
        /// Actual seam slice length.
        actual: usize,
    },
    /// Per-edge sharpness data length did not match the face degree.
    EdgeSharpnessLengthMismatch {
        /// Expected number of loop edges.
        expected: usize,
        /// Actual sharpness slice length.
        actual: usize,
    },
}

impl fmt::Display for FaceAttrErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EdgeSeamLengthMismatch { expected, actual } => {
                write!(
                    f,
                    "edge seam metadata length mismatch: expected {expected}, found {actual}"
                )
            }
            Self::EdgeSharpnessLengthMismatch { expected, actual } => {
                write!(
                    f,
                    "edge sharpness metadata length mismatch: expected {expected}, found {actual}"
                )
            }
        }
    }
}

impl core::error::Error for FaceAttrErrorKind {}

/// Construction error returned by [`Mesh::from_indexed_triangles`] and
/// [`Mesh::from_polygons`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BuildError {
    /// Triangle index referenced a position outside the input array.
    IndexOutOfBounds {
        /// Triangle index in the input list.
        triangle: usize,
        /// Out-of-range vertex index value.
        index: u32,
    },
    /// Triangle collapsed to fewer than three distinct vertices after remap/weld.
    DegenerateTriangle {
        /// Triangle index in the input list.
        triangle: usize,
    },
    /// More than two faces share an undirected edge, or two edges share same direction.
    NonManifoldEdge {
        /// Lower endpoint index of the undirected edge key.
        a: u32,
        /// Upper endpoint index of the undirected edge key.
        b: u32,
        /// Number of occurrences seen for this edge key.
        count: usize,
    },
    /// Boundary stitching failed (missing or ambiguous continuation).
    BoundaryStitchFailed {
        /// Boundary vertex where continuation lookup failed.
        vertex: u32,
        /// Number of continuation candidates found.
        candidates: usize,
    },
    /// Face-loop validation failed during polygon/ngon construction.
    InvalidFaceLoop {
        /// Face-loop index in builder input order.
        face: usize,
        /// Validation failure reason.
        kind: FaceLoopErrorKind,
    },
    /// Face attribute metadata failed validation during polygon/ngon construction.
    InvalidFaceAttrs {
        /// Face-loop index in builder input order.
        face: usize,
        /// Validation failure reason.
        kind: FaceAttrErrorKind,
    },
    /// Build parameter contained an invalid weld tolerance value.
    InvalidWeldTolerance,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexOutOfBounds { triangle, index } => {
                write!(f, "triangle {triangle} index out of bounds: {index}")
            }
            Self::DegenerateTriangle { triangle } => {
                write!(f, "triangle {triangle} is degenerate")
            }
            Self::NonManifoldEdge { a, b, count } => {
                write!(f, "non-manifold edge ({a}, {b}) with multiplicity {count}")
            }
            Self::BoundaryStitchFailed { vertex, candidates } => {
                write!(
                    f,
                    "boundary stitch failed at vertex {vertex} ({candidates} candidates)"
                )
            }
            Self::InvalidFaceLoop { face, kind } => {
                write!(f, "invalid face loop at face {face}: {kind}")
            }
            Self::InvalidFaceAttrs { face, kind } => {
                write!(f, "invalid face attributes at face {face}: {kind}")
            }
            Self::InvalidWeldTolerance => f.write_str("invalid weld tolerance"),
        }
    }
}

impl core::error::Error for BuildError {}

/// Structured validation error returned by [`Mesh::validate_fast`] and
/// [`Mesh::validate_deep`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ValidationError {
    /// A dense attribute layer length does not match domain capacity.
    DenseCapacityMismatch {
        /// Attribute domain.
        domain: Domain,
        /// Attribute name.
        name: &'static str,
        /// Expected dense length.
        expected: usize,
        /// Actual dense length.
        actual: usize,
    },
    /// A topology reference points to a stale/dead ID.
    InvalidReference {
        /// Owning element category.
        owner: &'static str,
        /// Owning element index.
        owner_index: u32,
        /// Referenced field name.
        field: &'static str,
        /// Referenced ID index.
        target_index: u32,
    },
    /// Twin involution is broken (`twin(twin(h)) != h`).
    TwinMismatch {
        /// Half-edge index.
        half_edge: u32,
        /// Twin index referenced by `half_edge`.
        twin: u32,
        /// Twin of twin index.
        twin_of_twin: u32,
    },
    /// Face loop did not close after cached degree steps.
    FaceLoopNotClosed {
        /// Face index.
        face: u32,
    },
    /// Cached degree does not match deep loop enumeration.
    FaceDegreeMismatch {
        /// Face index.
        face: u32,
        /// Cached degree value.
        cached: u32,
        /// Enumerated degree value.
        actual: usize,
    },
    /// A half-edge in a face loop points at a different face.
    FaceLoopForeignHalfEdge {
        /// Face index.
        face: u32,
        /// Half-edge index.
        half_edge: u32,
        /// Face index found on half-edge.
        found_face: u32,
    },
    /// Vertex-star walk did not terminate.
    VertexStarNotClosed {
        /// Vertex index.
        vertex: u32,
    },
    /// Boundary modeling invariant is broken.
    BoundaryInvariantBroken {
        /// Half-edge index.
        half_edge: u32,
    },
    /// Undirected edge does not have exactly two half-edges.
    EdgeMultiplicity {
        /// Lower endpoint index.
        a: u32,
        /// Upper endpoint index.
        b: u32,
        /// Number of half-edges found for this undirected edge.
        count: usize,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DenseCapacityMismatch {
                domain,
                name,
                expected,
                actual,
            } => write!(
                f,
                "dense capacity mismatch for {domain:?}.{name}: expected {expected}, got {actual}"
            ),
            Self::InvalidReference {
                owner,
                owner_index,
                field,
                target_index,
            } => write!(
                f,
                "invalid reference: {owner}[{owner_index}].{field} -> {target_index}"
            ),
            Self::TwinMismatch {
                half_edge,
                twin,
                twin_of_twin,
            } => write!(
                f,
                "twin mismatch at half-edge {half_edge}: twin {twin}, twin(twin) {twin_of_twin}"
            ),
            Self::FaceLoopNotClosed { face } => write!(f, "face loop not closed: {face}"),
            Self::FaceDegreeMismatch {
                face,
                cached,
                actual,
            } => write!(
                f,
                "face degree mismatch at {face}: cached {cached}, actual {actual}"
            ),
            Self::FaceLoopForeignHalfEdge {
                face,
                half_edge,
                found_face,
            } => write!(
                f,
                "foreign half-edge in face loop: face {face}, half-edge {half_edge}, found {found_face}"
            ),
            Self::VertexStarNotClosed { vertex } => {
                write!(f, "vertex star not closed: {vertex}")
            }
            Self::BoundaryInvariantBroken { half_edge } => {
                write!(f, "boundary invariant broken at half-edge {half_edge}")
            }
            Self::EdgeMultiplicity { a, b, count } => {
                write!(f, "edge multiplicity mismatch for ({a}, {b}): {count}")
            }
        }
    }
}

impl core::error::Error for ValidationError {}

/// Boundary-loop traversal error returned by [`Mesh::boundary_loop`] and
/// [`Mesh::boundary_loops`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BoundaryLoopError {
    /// Seed half-edge is stale or references stale topology.
    StaleHalfEdge,
    /// Seed half-edge is not on a boundary.
    NotBoundaryEdge,
    /// Boundary traversal encountered a non-OUTSIDE half-edge.
    NonBoundaryHalfEdge {
        /// Half-edge index that broke the boundary invariant.
        half_edge: u32,
    },
    /// Boundary traversal encountered a missing `next` pointer.
    MissingNext {
        /// Half-edge index missing a `next` pointer.
        half_edge: u32,
    },
    /// Boundary traversal revisited a non-start half-edge.
    RevisitedHalfEdge {
        /// Traversal seed index.
        start: u32,
        /// Revisited half-edge index.
        half_edge: u32,
    },
}

impl fmt::Display for BoundaryLoopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleHalfEdge => f.write_str("boundary loop seed is stale"),
            Self::NotBoundaryEdge => f.write_str("half-edge is not on a boundary loop"),
            Self::NonBoundaryHalfEdge { half_edge } => {
                write!(
                    f,
                    "boundary traversal encountered non-OUTSIDE half-edge {half_edge}"
                )
            }
            Self::MissingNext { half_edge } => {
                write!(
                    f,
                    "boundary traversal encountered missing next on {half_edge}"
                )
            }
            Self::RevisitedHalfEdge { start, half_edge } => write!(
                f,
                "boundary traversal from {start} revisited non-start half-edge {half_edge}"
            ),
        }
    }
}

impl core::error::Error for BoundaryLoopError {}

/// Selected-face boundary-loop extraction error returned by
/// [`Mesh::selected_face_boundary_loops`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SelectedFaceBoundaryError {
    /// Selection contains [`FaceId::OUTSIDE`].
    OutsideFaceInSelection,
    /// Selection contains a stale face ID.
    StaleFace {
        /// Stale face ID.
        face: FaceId,
    },
    /// A selected face loop could not provide stable boundary vertices.
    InvalidFaceLoop {
        /// Face whose loop data was invalid.
        face: FaceId,
    },
    /// Selected-patch boundary had more than one outgoing continuation.
    AmbiguousBoundaryVertex {
        /// Boundary vertex where traversal became ambiguous.
        vertex: VertexId,
        /// Number of candidate outgoing boundary edges.
        candidates: usize,
    },
    /// Selected-patch boundary traversal found an open chain instead of a loop.
    OpenBoundaryChain {
        /// Start vertex of the open chain.
        start: VertexId,
        /// End vertex where traversal stopped.
        end: VertexId,
    },
}

impl fmt::Display for SelectedFaceBoundaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutsideFaceInSelection => f.write_str("selected patch contains FaceId::OUTSIDE"),
            Self::StaleFace { face } => {
                write!(f, "selected patch contains stale face {}", face.index())
            }
            Self::InvalidFaceLoop { face } => {
                write!(
                    f,
                    "selected patch has invalid face loop on face {}",
                    face.index()
                )
            }
            Self::AmbiguousBoundaryVertex { vertex, candidates } => write!(
                f,
                "selected patch boundary is ambiguous at vertex {} ({candidates} candidates)",
                vertex.index()
            ),
            Self::OpenBoundaryChain { start, end } => write!(
                f,
                "selected patch boundary has open chain from {} to {}",
                start.index(),
                end.index()
            ),
        }
    }
}

impl core::error::Error for SelectedFaceBoundaryError {}

/// Selected-face patch topology query error returned by
/// [`Mesh::selected_face_patch_topology`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SelectedFacePatchError {
    /// Selection contains [`FaceId::OUTSIDE`].
    OutsideFaceInSelection,
    /// Selection contains a stale face ID.
    StaleFace {
        /// Stale face ID.
        face: FaceId,
    },
    /// A selected face loop could not provide stable topology.
    InvalidFaceLoop {
        /// Face whose loop data was invalid.
        face: FaceId,
    },
}

impl fmt::Display for SelectedFacePatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutsideFaceInSelection => f.write_str("selected patch contains FaceId::OUTSIDE"),
            Self::StaleFace { face } => {
                write!(f, "selected patch contains stale face {}", face.index())
            }
            Self::InvalidFaceLoop { face } => {
                write!(
                    f,
                    "selected patch has invalid face loop on face {}",
                    face.index()
                )
            }
        }
    }
}

impl core::error::Error for SelectedFacePatchError {}

/// One selected-face patch edge reference.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SelectedFacePatchEdge {
    /// Face that owns the selected patch edge.
    pub face: FaceId,
    /// Half-edge on the selected face loop.
    pub edge: HalfEdgeId,
    /// Source vertex of `edge`.
    pub from: VertexId,
    /// Destination vertex of `edge`.
    pub to: VertexId,
    /// Stable edge index within the owning face loop.
    pub edge_index: usize,
}

/// One undirected edge shared by multiple selected faces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedFacePatchSharedEdge {
    /// Canonical undirected edge key.
    pub key: (VertexId, VertexId),
    /// Selected face incidences touching `key`.
    pub incidences: Vec<SelectedFacePatchEdge>,
}

/// Deterministic topology classification for a selected face patch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedFacePatchTopology {
    /// Boundary edges on the selected face side.
    pub boundary_edges: Vec<SelectedFacePatchEdge>,
    /// Interior shared edges inside the selected patch.
    pub interior_edges: Vec<SelectedFacePatchSharedEdge>,
    /// Unique vertices touched by the selected patch in deterministic order.
    pub incident_vertices: Vec<VertexId>,
    /// True when every selected boundary edge borders [`FaceId::OUTSIDE`].
    pub boundary_lies_on_mesh_boundary: bool,
}

/// Connected face-region query error returned by [`Mesh::connected_face_region`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ConnectedFaceRegionError {
    /// Seed face cannot be [`FaceId::OUTSIDE`].
    OutsideSeedFace,
    /// Seed face is stale.
    StaleSeedFace {
        /// Stale face ID.
        face: FaceId,
    },
    /// Required dense `face.region` attribute layer is missing.
    MissingFaceRegionAttribute,
    /// A face in the connected component had no `face.region` value.
    MissingFaceRegionValue {
        /// Face missing a region value.
        face: FaceId,
    },
    /// Region traversal encountered broken adjacency on a face loop.
    BrokenAdjacency {
        /// Face whose loop traversal encountered invalid adjacency.
        face: FaceId,
        /// Half-edge that failed adjacency lookup.
        half_edge: HalfEdgeId,
    },
}

impl fmt::Display for ConnectedFaceRegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutsideSeedFace => {
                f.write_str("connected face-region seed cannot be FaceId::OUTSIDE")
            }
            Self::StaleSeedFace { face } => {
                write!(f, "connected face-region seed is stale: {}", face.index())
            }
            Self::MissingFaceRegionAttribute => {
                f.write_str("missing required dense face.region layer")
            }
            Self::MissingFaceRegionValue { face } => write!(
                f,
                "connected face-region face is missing a region value: {}",
                face.index()
            ),
            Self::BrokenAdjacency { face, half_edge } => write!(
                f,
                "connected face-region traversal encountered broken adjacency on face {} at half-edge {}",
                face.index(),
                half_edge.index()
            ),
        }
    }
}

impl core::error::Error for ConnectedFaceRegionError {}

/// Strategy for enumerating one face's triangles.
///
/// A triangle index is meaningful only together with the strategy that
/// produced it; APIs that store triangle indices (for example the boolean
/// broad phase) record the strategy alongside them. See ADR-0008.
#[derive(Copy, Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum FaceTriangulation {
    /// Deterministic fan from the loop's first corner.
    ///
    /// Fast and allocation-light, but can produce overlapping or inverted
    /// triangles for non-convex faces.
    #[default]
    Fan,
    /// Plane-projected robust triangulation via `exedra_triangulate`.
    ///
    /// Handles concave faces; falls back to the fan deterministically when
    /// the projected polygon is not simple (enumeration never fails).
    Robust,
}

/// Monotonic mesh revision counter.
///
/// Revision increments exactly once per finished edit scope
/// ([`EditSession::finish`](crate::EditSession::finish)).
#[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MeshRevision(u64);

impl MeshRevision {
    /// Returns the raw revision value.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Domain remapping produced by [`Mesh::compact`].
///
/// The remap translates IDs from the source mesh into IDs in the compacted
/// mesh. Deleted/tombstoned source IDs return `None`. [`FaceId::OUTSIDE`] is a
/// sentinel and maps to itself.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Remap {
    vertices: Vec<Option<(NonZeroU32, VertexId)>>,
    half_edges: Vec<Option<(NonZeroU32, HalfEdgeId)>>,
    faces: Vec<Option<(NonZeroU32, FaceId)>>,
}

impl Remap {
    /// Returns the compacted vertex ID for a live source vertex ID.
    #[must_use]
    pub fn vertex(&self, vertex: VertexId) -> Option<VertexId> {
        lookup_remap(&self.vertices, vertex.as_id())
    }

    /// Returns the compacted half-edge ID for a live source half-edge ID.
    #[must_use]
    pub fn half_edge(&self, half_edge: HalfEdgeId) -> Option<HalfEdgeId> {
        lookup_remap(&self.half_edges, half_edge.as_id())
    }

    /// Returns the compacted corner ID for a live source corner ID.
    #[must_use]
    pub fn corner(&self, corner: CornerId) -> Option<CornerId> {
        self.half_edge(corner)
    }

    /// Returns the compacted face ID for a live source face ID.
    ///
    /// [`FaceId::OUTSIDE`] maps to itself because it is a stable sentinel, not
    /// a stored face arena entry.
    #[must_use]
    pub fn face(&self, face: FaceId) -> Option<FaceId> {
        if face == FaceId::OUTSIDE {
            return Some(FaceId::OUTSIDE);
        }
        lookup_remap(&self.faces, face.as_id())
    }
}

fn lookup_remap<T: Copy>(entries: &[Option<(NonZeroU32, T)>], id: Id) -> Option<T> {
    entries
        .get(id.index() as usize)
        .and_then(|entry| *entry)
        .and_then(|(generation, mapped)| (generation == id.generation()).then_some(mapped))
}

/// Result payload from [`MeshBuilder::build`].
///
/// Use this when you need both the built mesh and deterministic provenance
/// back to builder-local indices.
#[derive(Clone, Debug)]
pub struct MeshBuildResult {
    /// Built mesh.
    pub mesh: Mesh,
    /// Builder-local vertex index `i` maps to `vertex_ids[i]`.
    pub vertex_ids: Vec<VertexId>,
    /// Builder-local face index `i` maps to `face_ids[i]`.
    pub face_ids: Vec<FaceId>,
    /// Directed loop edge provenance by face: `face_edge_ids[face][edge_index]`.
    pub face_edge_ids: Vec<Vec<HalfEdgeId>>,
}

/// Optional metadata attached while inserting one face into a [`MeshBuilder`].
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct FaceBuildAttrs<'a> {
    /// Face-region value written into [`attr::FACE_REGION`] for the built face.
    pub region: Option<u32>,
    /// Per-edge seam values written onto canonical undirected edges.
    pub edge_seams: Option<&'a [bool]>,
    /// Per-edge sharpness values written onto canonical undirected edges.
    pub edge_sharpness: Option<&'a [f32]>,
}

/// Incremental polygon/ngon mesh builder using builder-local indices.
///
/// Build flow:
/// 1. [`MeshBuilder::push_vertex`]
/// 2. [`MeshBuilder::add_face`]
/// 3. [`MeshBuilder::build`]
#[derive(Clone, Debug, Default)]
pub struct MeshBuilder {
    vertices: Vec<[f32; 3]>,
    faces: Vec<Vec<u32>>,
    face_attrs: Vec<OwnedFaceBuildAttrs>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct OwnedFaceBuildAttrs {
    region: Option<u32>,
    edge_seams: Option<Vec<bool>>,
    edge_sharpness: Option<Vec<f32>>,
}

/// Half-edge mesh storage with explicit OUTSIDE boundary semantics.
///
/// This is the main topology container in Exedra. Construct with [`Mesh::new`]
/// or builders like [`Mesh::from_indexed_triangles`]/[`Mesh::from_polygons`],
/// then traverse/edit using methods on this type or via [`EditSession`](crate::EditSession).
#[derive(Clone, Debug)]
pub struct Mesh {
    pub(crate) vertices: Arena<Vertex>,
    pub(crate) half_edges: Arena<HalfEdge>,
    pub(crate) faces: Arena<Face>,
    pub(crate) attrs: Attributes,
    pub(crate) revision: u64,
}

impl Default for Mesh {
    fn default() -> Self {
        Self::new()
    }
}

impl Mesh {
    /// Creates an empty mesh.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vertices: Arena::new(),
            half_edges: Arena::new(),
            faces: Arena::new(),
            attrs: Attributes::new(),
            revision: 0,
        }
    }

    /// Returns the current monotonic mesh revision.
    ///
    /// Revision is part of mesh state and naturally carries through topology
    /// transformations such as compaction.
    #[must_use]
    pub const fn revision(&self) -> MeshRevision {
        MeshRevision(self.revision)
    }

    /// Returns a tombstone-free copy of this mesh and a source-to-copy ID remap.
    ///
    /// Compaction is explicit: the source mesh is unchanged, and all live
    /// topology and attributes are copied into fresh arenas with contiguous slot
    /// indices. Deleted source IDs and stale generations are not mapped. The
    /// [`FaceId::OUTSIDE`] sentinel maps to itself.
    #[must_use]
    pub fn compact(&self) -> (Self, Remap) {
        let mut compacted = Self {
            vertices: Arena::new(),
            half_edges: Arena::new(),
            faces: Arena::new(),
            attrs: Attributes::new(),
            revision: self.revision,
        };
        let mut remap = Remap {
            vertices: vec![None; self.vertices.slot_count()],
            half_edges: vec![None; self.half_edges.slot_count()],
            faces: vec![None; self.faces.slot_count()],
        };
        let mut vertex_attr_map = vec![None; self.vertices.slot_count()];
        let mut face_attr_map = vec![None; self.faces.slot_count()];
        let mut half_edge_attr_map = vec![None; self.half_edges.slot_count()];

        for (old_id, _) in self.vertices.iter() {
            let new_id = VertexId::from(compacted.vertices.insert(Vertex {
                out: HalfEdgeId::INVALID,
            }));
            remap.vertices[old_id.index() as usize] = Some((old_id.generation(), new_id));
            vertex_attr_map[old_id.index() as usize] = Some(new_id.as_id());
        }

        for (old_id, face) in self.faces.iter() {
            let new_id = FaceId::from(compacted.faces.insert(Face {
                edge: HalfEdgeId::INVALID,
                degree: face.degree,
            }));
            remap.faces[old_id.index() as usize] = Some((old_id.generation(), new_id));
            face_attr_map[old_id.index() as usize] = Some(new_id.as_id());
        }

        for (old_id, _) in self.half_edges.iter() {
            let new_id = HalfEdgeId::from(compacted.half_edges.insert(HalfEdge {
                to: VertexId::INVALID,
                face: FaceId::OUTSIDE,
                next: HalfEdgeId::INVALID,
                twin: HalfEdgeId::INVALID,
            }));
            remap.half_edges[old_id.index() as usize] = Some((old_id.generation(), new_id));
            half_edge_attr_map[old_id.index() as usize] = Some(new_id.as_id());
        }

        for (old_id, vertex) in self.vertices.iter() {
            let new_id = remap
                .vertex(VertexId::from(old_id))
                .expect("live source vertex must have a compacted ID");
            let new_out = if vertex.out == HalfEdgeId::INVALID {
                HalfEdgeId::INVALID
            } else {
                remap
                    .half_edge(vertex.out)
                    .expect("live source vertex must reference live outgoing edge")
            };
            compacted
                .vertices
                .get_mut(new_id.as_id())
                .expect("compacted vertex must exist")
                .out = new_out;
        }

        for (old_id, face) in self.faces.iter() {
            let new_id = remap
                .face(FaceId::from(old_id))
                .expect("live source face must have a compacted ID");
            let new_edge = remap
                .half_edge(face.edge)
                .expect("live source face must reference live loop edge");
            compacted
                .faces
                .get_mut(new_id.as_id())
                .expect("compacted face must exist")
                .edge = new_edge;
        }

        for (old_id, half_edge) in self.half_edges.iter() {
            let new_id = remap
                .half_edge(HalfEdgeId::from(old_id))
                .expect("live source half-edge must have a compacted ID");
            let new_half_edge = HalfEdge {
                to: remap
                    .vertex(half_edge.to)
                    .expect("live source half-edge must reference live destination vertex"),
                face: remap
                    .face(half_edge.face)
                    .expect("live source half-edge must reference live face or OUTSIDE"),
                next: remap
                    .half_edge(half_edge.next)
                    .expect("live source half-edge must reference live next edge"),
                twin: remap
                    .half_edge(half_edge.twin)
                    .expect("live source half-edge must reference live twin edge"),
            };
            *compacted
                .half_edges
                .get_mut(new_id.as_id())
                .expect("compacted half-edge must exist") = new_half_edge;
        }

        compacted.attrs =
            self.attrs
                .compacted(&vertex_attr_map, &face_attr_map, &half_edge_attr_map);

        (compacted, remap)
    }

    /// Returns immutable attribute storage.
    #[must_use]
    pub const fn attrs(&self) -> &Attributes {
        &self.attrs
    }

    #[must_use]
    pub(crate) fn attrs_mut(&mut self) -> &mut Attributes {
        &mut self.attrs
    }

    /// Adds a new vertex with a required position value.
    ///
    /// The vertex position is written into the required dense
    /// [`attr::VERTEX_POSITION`] layer.
    pub(crate) fn add_vertex(&mut self, position: [f32; 3]) -> VertexId {
        let id = VertexId::from(self.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        self.sync_attr_capacities();
        let set_ok = self
            .attrs
            .dense_mut(attr::VERTEX_POSITION)
            .expect("builtin position layer must exist")
            .set(id.as_id(), position);
        debug_assert!(set_ok, "new vertex index must be in dense position layer");
        id
    }

    /// Returns the required position for a vertex.
    #[must_use]
    pub fn vertex_position(&self, vertex: VertexId) -> Option<&[f32; 3]> {
        self.attrs
            .dense(attr::VERTEX_POSITION)
            .and_then(|layer| layer.get(vertex.as_id()))
    }

    /// Sets the required position for a vertex.
    ///
    /// Returns `true` if the vertex slot exists in the position layer.
    pub(crate) fn set_vertex_position(&mut self, vertex: VertexId, position: [f32; 3]) -> bool {
        self.attrs
            .dense_mut(attr::VERTEX_POSITION)
            .is_some_and(|layer| layer.set(vertex.as_id(), position))
    }

    /// Returns the explicit vertex sharpness override, when present.
    ///
    /// `None` means either the vertex is stale or no override is authored, so
    /// downstream subdivision classifiers should derive the class from incident
    /// edge sharpness.
    #[must_use]
    pub fn vertex_sharpness(&self, vertex: VertexId) -> Option<f32> {
        let _ = self.vertices.get(vertex.as_id())?;
        self.attrs
            .sparse(attr::VERTEX_SHARPNESS)
            .and_then(|layer| layer.get(vertex.as_id()).copied())
    }

    /// Sets the explicit vertex sharpness override.
    ///
    /// Returns `true` when `vertex` is live and writable.
    pub(crate) fn set_vertex_sharpness(&mut self, vertex: VertexId, sharpness: f32) -> bool {
        if self.vertices.get(vertex.as_id()).is_none() {
            return false;
        }
        if self.attrs.sparse(attr::VERTEX_SHARPNESS).is_none() {
            let _ = self.attrs.define_sparse(attr::VERTEX_SHARPNESS);
        }
        self.attrs
            .sparse_mut(attr::VERTEX_SHARPNESS)
            .is_some_and(|layer| {
                layer.set(vertex.as_id(), sharpness);
                true
            })
    }

    /// Returns one outgoing half-edge for the given vertex.
    #[must_use]
    pub fn vertex_out(&self, vertex: VertexId) -> Option<HalfEdgeId> {
        self.vertices
            .get(vertex.as_id())
            .map(|record| record.out)
            .filter(|out| *out != HalfEdgeId::INVALID)
    }

    /// Iterates live vertices in deterministic arena slot order.
    pub fn vertices(&self) -> impl Iterator<Item = VertexId> + '_ {
        self.vertices.iter().map(|(id, _)| VertexId::from(id))
    }

    /// Iterates live interior faces in deterministic arena slot order.
    pub fn faces(&self) -> impl Iterator<Item = FaceId> + '_ {
        self.faces.iter().map(|(id, _)| FaceId::from(id))
    }

    /// Returns one loop half-edge for an interior face.
    ///
    /// Returns `None` for [`FaceId::OUTSIDE`].
    #[must_use]
    pub fn face_edge(&self, face: FaceId) -> Option<HalfEdgeId> {
        if face == FaceId::OUTSIDE {
            return None;
        }
        self.faces.get(face.as_id()).map(|record| record.edge)
    }

    /// Returns the twin half-edge.
    #[must_use]
    pub fn twin(&self, half_edge: HalfEdgeId) -> Option<HalfEdgeId> {
        self.half_edges
            .get(half_edge.as_id())
            .map(|record| record.twin)
    }

    /// Returns a deterministic canonical representative for an undirected edge.
    ///
    /// Canonicalization uses the smaller stable half-edge ID between `h` and
    /// `twin(h)`, so callers can access one per-edge value regardless of which
    /// directed half-edge they start from.
    #[must_use]
    pub fn canonical_edge(&self, half_edge: HalfEdgeId) -> Option<HalfEdgeId> {
        let twin = self.twin(half_edge)?;
        Some(core::cmp::min(half_edge, twin))
    }

    /// Returns the explicit seam tag for an edge.
    ///
    /// Returns `None` when `half_edge` is stale.
    #[must_use]
    pub fn edge_seam(&self, half_edge: HalfEdgeId) -> Option<bool> {
        let canonical = self.canonical_edge(half_edge)?;
        Some(
            self.attrs
                .sparse(attr::EDGE_SEAM)
                .and_then(|layer| layer.get(canonical.as_id()).copied())
                .unwrap_or(false),
        )
    }

    /// Sets the explicit seam tag for an edge.
    ///
    /// Returns `true` when `half_edge` is live and writable.
    pub(crate) fn set_edge_seam(&mut self, half_edge: HalfEdgeId, seam: bool) -> bool {
        let Some(canonical) = self.canonical_edge(half_edge) else {
            return false;
        };
        if self.attrs.sparse(attr::EDGE_SEAM).is_none() {
            let _ = self.attrs.define_sparse(attr::EDGE_SEAM);
        }
        self.attrs.sparse_mut(attr::EDGE_SEAM).is_some_and(|layer| {
            layer.set(canonical.as_id(), seam);
            true
        })
    }

    /// Returns the explicit sharpness value for an edge.
    ///
    /// Returns `None` when `half_edge` is stale.
    #[must_use]
    pub fn edge_sharpness(&self, half_edge: HalfEdgeId) -> Option<f32> {
        let canonical = self.canonical_edge(half_edge)?;
        Some(
            self.attrs
                .sparse(attr::EDGE_SHARPNESS)
                .and_then(|layer| layer.get(canonical.as_id()).copied())
                .unwrap_or(0.0),
        )
    }

    /// Sets the explicit sharpness value for an edge.
    ///
    /// Returns `true` when `half_edge` is live and writable.
    pub(crate) fn set_edge_sharpness(&mut self, half_edge: HalfEdgeId, sharp: f32) -> bool {
        let Some(canonical) = self.canonical_edge(half_edge) else {
            return false;
        };
        if self.attrs.sparse(attr::EDGE_SHARPNESS).is_none() {
            let _ = self.attrs.define_sparse(attr::EDGE_SHARPNESS);
        }
        self.attrs
            .sparse_mut(attr::EDGE_SHARPNESS)
            .is_some_and(|layer| {
                layer.set(canonical.as_id(), sharp);
                true
            })
    }

    /// Returns the next half-edge in the owning face loop.
    #[must_use]
    pub fn next(&self, half_edge: HalfEdgeId) -> Option<HalfEdgeId> {
        self.half_edges
            .get(half_edge.as_id())
            .map(|record| record.next)
    }

    /// Returns the owning face for a half-edge.
    #[must_use]
    pub fn face(&self, half_edge: HalfEdgeId) -> Option<FaceId> {
        self.half_edges
            .get(half_edge.as_id())
            .map(|record| record.face)
    }

    /// Returns one full boundary loop containing `seed_edge`.
    ///
    /// The seed may be either the OUTSIDE half-edge itself or its interior
    /// twin. Returned half-edges are the OUTSIDE loop half-edges in traversal
    /// order, starting from the normalized OUTSIDE seed.
    pub fn boundary_loop(
        &self,
        seed_edge: HalfEdgeId,
    ) -> Result<Vec<HalfEdgeId>, BoundaryLoopError> {
        let start = self.normalize_boundary_seed(seed_edge)?;
        let mut visited = BTreeSet::new();
        let mut loop_edges = Vec::new();
        let mut current = start;

        loop {
            if !visited.insert(current) {
                if current == start {
                    break;
                }
                return Err(BoundaryLoopError::RevisitedHalfEdge {
                    start: start.index(),
                    half_edge: current.index(),
                });
            }
            if self.face(current) != Some(FaceId::OUTSIDE) {
                return Err(BoundaryLoopError::NonBoundaryHalfEdge {
                    half_edge: current.index(),
                });
            }
            loop_edges.push(current);
            let next = self.next(current).ok_or(BoundaryLoopError::MissingNext {
                half_edge: current.index(),
            })?;
            if next == start {
                break;
            }
            current = next;
        }

        Ok(loop_edges)
    }

    /// Returns all boundary loops in deterministic seed order.
    pub fn boundary_loops(&self) -> Result<Vec<Vec<HalfEdgeId>>, BoundaryLoopError> {
        let mut visited = vec![false; self.half_edges.slot_count()];
        let mut loops = Vec::new();

        for (id, half_edge) in self.half_edges.iter() {
            if half_edge.face != FaceId::OUTSIDE || visited[id.index() as usize] {
                continue;
            }
            let boundary_loop = self.boundary_loop(HalfEdgeId::from(id))?;
            for edge in &boundary_loop {
                visited[edge.index() as usize] = true;
            }
            loops.push(boundary_loop);
        }

        Ok(loops)
    }

    /// Returns deterministic topology classification for a selected face patch.
    ///
    /// Input faces may be unsorted and may contain duplicates. Returned edge
    /// and vertex collections are canonicalized into deterministic order.
    pub fn selected_face_patch_topology(
        &self,
        faces: &[FaceId],
    ) -> Result<SelectedFacePatchTopology, SelectedFacePatchError> {
        let mut selected = BTreeSet::<FaceId>::new();
        for &face in faces {
            if face == FaceId::OUTSIDE {
                return Err(SelectedFacePatchError::OutsideFaceInSelection);
            }
            if self.face_edge(face).is_none() {
                return Err(SelectedFacePatchError::StaleFace { face });
            }
            let _ = selected.insert(face);
        }

        let mut incident_vertices = BTreeSet::<VertexId>::new();
        let mut edge_membership =
            BTreeMap::<(VertexId, VertexId), Vec<SelectedFacePatchEdge>>::new();

        for &face in &selected {
            let corners = self.face_loop(face).collect::<Vec<_>>();
            if corners.len() < 3 {
                return Err(SelectedFacePatchError::InvalidFaceLoop { face });
            }
            let mut vertices = Vec::with_capacity(corners.len());
            for &corner in &corners {
                let vertex = self
                    .to_vertex(corner)
                    .ok_or(SelectedFacePatchError::InvalidFaceLoop { face })?;
                let _ = incident_vertices.insert(vertex);
                vertices.push(vertex);
            }

            for edge_index in 0..vertices.len() {
                let edge = corners[(edge_index + 1) % corners.len()];
                let from = vertices[edge_index];
                let to = vertices[(edge_index + 1) % vertices.len()];
                let twin = self
                    .twin(edge)
                    .ok_or(SelectedFacePatchError::InvalidFaceLoop { face })?;
                let _ = self
                    .face(twin)
                    .ok_or(SelectedFacePatchError::InvalidFaceLoop { face })?;

                edge_membership
                    .entry(canonical_vertex_pair(from, to))
                    .or_default()
                    .push(SelectedFacePatchEdge {
                        face,
                        edge,
                        from,
                        to,
                        edge_index,
                    });
            }
        }

        let mut boundary_edges = Vec::new();
        let mut interior_edges = Vec::new();
        for (key, mut incidences) in edge_membership {
            incidences.sort_by_key(|edge| (edge.face.index(), edge.edge_index, edge.edge.index()));
            if incidences.len() == 1 {
                boundary_edges.push(incidences[0]);
            } else {
                interior_edges.push(SelectedFacePatchSharedEdge { key, incidences });
            }
        }

        boundary_edges.sort_by_key(|edge| (edge.face.index(), edge.edge_index, edge.edge.index()));
        interior_edges.sort_by_key(|edge| edge.key);

        let boundary_lies_on_mesh_boundary = !boundary_edges.is_empty()
            && boundary_edges.iter().all(|boundary| {
                self.twin(boundary.edge)
                    .and_then(|twin| self.face(twin))
                    .is_some_and(|face| face == FaceId::OUTSIDE)
            });

        Ok(SelectedFacePatchTopology {
            boundary_edges,
            interior_edges,
            incident_vertices: incident_vertices.into_iter().collect(),
            boundary_lies_on_mesh_boundary,
        })
    }

    /// Returns boundary loops around a selected face patch.
    ///
    /// Returned half-edges are the selected-face-side half-edges in traversal
    /// order. Input faces may be unsorted and may contain duplicates; the
    /// extraction order is still deterministic.
    pub fn selected_face_boundary_loops(
        &self,
        faces: &[FaceId],
    ) -> Result<Vec<Vec<HalfEdgeId>>, SelectedFaceBoundaryError> {
        let boundary_edges = self
            .selected_face_patch_topology(faces)
            .map_err(map_selected_face_patch_error)?
            .boundary_edges;

        let mut outgoing = BTreeMap::<VertexId, Vec<usize>>::new();
        for (index, edge) in boundary_edges.iter().enumerate() {
            outgoing.entry(edge.from).or_default().push(index);
        }
        for candidates in outgoing.values_mut() {
            candidates.sort_by_key(|&index| {
                let edge = boundary_edges[index];
                (edge.face.index(), edge.edge_index, edge.edge.index())
            });
        }

        let mut visited = vec![false; boundary_edges.len()];
        let mut loops = Vec::<Vec<HalfEdgeId>>::new();
        for seed_index in 0..boundary_edges.len() {
            if visited[seed_index] {
                continue;
            }
            let seed = boundary_edges[seed_index];
            let mut current = seed;
            let mut current_index = seed_index;
            let mut loop_edges = Vec::<HalfEdgeId>::new();

            loop {
                visited[current_index] = true;
                loop_edges.push(current.edge);

                if current.to == seed.from {
                    break;
                }

                let candidates = outgoing
                    .get(&current.to)
                    .map(|indices| {
                        indices
                            .iter()
                            .copied()
                            .filter(|&index| !visited[index])
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                match candidates.as_slice() {
                    [next_index] => {
                        current_index = *next_index;
                        current = boundary_edges[*next_index];
                    }
                    [] => {
                        return Err(SelectedFaceBoundaryError::OpenBoundaryChain {
                            start: seed.from,
                            end: current.to,
                        });
                    }
                    many => {
                        return Err(SelectedFaceBoundaryError::AmbiguousBoundaryVertex {
                            vertex: current.to,
                            candidates: many.len(),
                        });
                    }
                }
            }

            loops.push(loop_edges);
        }

        loops.sort_by_key(|boundary_loop| {
            boundary_loop
                .first()
                .copied()
                .expect("selected-face boundary loops are never empty")
        });
        Ok(loops)
    }

    /// Returns the connected interior faces that share the seed face's region.
    ///
    /// Returned faces are sorted in deterministic [`FaceId`] order.
    pub fn connected_face_region(
        &self,
        seed_face: FaceId,
    ) -> Result<Vec<FaceId>, ConnectedFaceRegionError> {
        if seed_face == FaceId::OUTSIDE {
            return Err(ConnectedFaceRegionError::OutsideSeedFace);
        }
        if self.face_edge(seed_face).is_none() {
            return Err(ConnectedFaceRegionError::StaleSeedFace { face: seed_face });
        }

        let region_layer = self
            .attrs()
            .dense(attr::FACE_REGION)
            .ok_or(ConnectedFaceRegionError::MissingFaceRegionAttribute)?;
        let seed_region = region_layer
            .get(seed_face.as_id())
            .copied()
            .ok_or(ConnectedFaceRegionError::MissingFaceRegionValue { face: seed_face })?;

        let mut visited = BTreeSet::new();
        let mut queue = VecDeque::new();
        let mut faces = Vec::new();

        visited.insert(seed_face);
        queue.push_back(seed_face);

        while let Some(face) = queue.pop_front() {
            faces.push(face);
            for half_edge in self.face_loop(face) {
                let twin = self
                    .twin(half_edge)
                    .ok_or(ConnectedFaceRegionError::BrokenAdjacency { face, half_edge })?;
                let adjacent = self
                    .face(twin)
                    .ok_or(ConnectedFaceRegionError::BrokenAdjacency { face, half_edge })?;
                if adjacent == FaceId::OUTSIDE || visited.contains(&adjacent) {
                    continue;
                }
                let adjacent_region = region_layer
                    .get(adjacent.as_id())
                    .copied()
                    .ok_or(ConnectedFaceRegionError::MissingFaceRegionValue { face: adjacent })?;
                if adjacent_region == seed_region {
                    let inserted = visited.insert(adjacent);
                    debug_assert!(inserted, "visited faces must remain unique");
                    queue.push_back(adjacent);
                }
            }
        }

        faces.sort_unstable();
        Ok(faces)
    }

    /// Returns the destination vertex for a half-edge.
    #[must_use]
    pub fn to_vertex(&self, half_edge: HalfEdgeId) -> Option<VertexId> {
        self.half_edges
            .get(half_edge.as_id())
            .map(|record| record.to)
    }

    /// Returns the previous half-edge in the owning face loop.
    ///
    /// This is derived by walking `next` links (v0.1 decision: no stored prev).
    #[must_use]
    pub fn prev(&self, half_edge: HalfEdgeId) -> Option<HalfEdgeId> {
        let face = self.face(half_edge)?;
        let face_degree = if face == FaceId::OUTSIDE {
            None
        } else {
            self.faces
                .get(face.as_id())
                .and_then(|record| usize::try_from(record.degree).ok())
        };
        let max_steps = self.half_edges.slot_count().max(1);
        let mut cursor = self.next(half_edge).unwrap_or_else(|| {
            panic!(
                "mesh topology corruption: prev() missing next for half-edge {}",
                half_edge.index()
            )
        });
        for _ in 0..max_steps {
            let next = self.next(cursor).unwrap_or_else(|| {
                panic!(
                    "mesh topology corruption: prev() encountered missing next from half-edge {}",
                    cursor.index()
                )
            });
            if next == half_edge {
                return Some(cursor);
            }
            cursor = next;
        }
        panic!(
            "mesh topology corruption: prev() exceeded {} steps for half-edge {} on face {} (degree {:?})",
            max_steps,
            half_edge.index(),
            face.index(),
            face_degree
        );
    }

    /// Returns the origin vertex for a half-edge.
    ///
    /// This call is `O(face_degree)` because it derives origin via [`Mesh::prev`].
    #[must_use]
    pub fn from_vertex(&self, half_edge: HalfEdgeId) -> Option<VertexId> {
        let prev = self.prev(half_edge)?;
        self.to_vertex(prev)
    }

    /// Returns true when corner UV values are discontinuous across an edge.
    ///
    /// This compares effective corner UVs (missing values treated as
    /// `[0.0, 0.0]`) at both edge endpoints.
    #[must_use]
    pub fn is_uv_discontinuous(&self, half_edge: HalfEdgeId) -> Option<bool> {
        let twin = self.twin(half_edge)?;
        let next = self.next(half_edge)?;
        let twin_next = self.next(twin)?;
        let layer = self.attrs.sparse(attr::CORNER_UV);
        let uv = |corner: HalfEdgeId| {
            layer
                .and_then(|sparse| sparse.get(corner.as_id()).copied())
                .unwrap_or([0.0, 0.0])
        };

        let endpoint_a_diff = uv(half_edge).map(f32::to_bits) != uv(twin_next).map(f32::to_bits);
        let endpoint_b_diff = uv(twin).map(f32::to_bits) != uv(next).map(f32::to_bits);
        Some(endpoint_a_diff || endpoint_b_diff)
    }

    /// Iterates half-edges on an interior face loop.
    #[must_use]
    pub fn face_loop(&self, face: FaceId) -> FaceLoopIter<'_> {
        let start = self.face_edge(face);
        let max_steps = self.half_edges.slot_count().max(1);
        FaceLoopIter {
            mesh: self,
            face,
            start,
            current: start,
            steps: 0,
            max_steps,
            exhausted: start.is_none(),
        }
    }

    /// Iterates outgoing half-edges in a vertex star.
    ///
    /// Iteration is deterministic in half-edge slot order.
    ///
    /// Current v0.1 implementation scans all half-edges and filters by
    /// [`Mesh::from_vertex`], so it is `O(total_half_edges * face_degree)` in
    /// the worst case.
    pub fn vertex_star(&self, vertex: VertexId) -> impl Iterator<Item = HalfEdgeId> + '_ {
        self.half_edges.iter().filter_map(move |(id, _)| {
            let typed = HalfEdgeId::from(id);
            (self.from_vertex(typed) == Some(vertex)).then_some(typed)
        })
    }

    /// Triangulates one interior face using deterministic fan triangulation.
    ///
    /// Fan triangulation emits triangles:
    /// `(c0, c1, c2)`, `(c0, c2, c3)`, ..., `(c0, c{n-2}, c{n-1})`
    /// where `c0..c{n-1}` are loop corners from [`Mesh::face_loop`].
    ///
    /// This strategy is deterministic and simple, but can generate poor
    /// quality triangles for non-convex polygons.
    #[must_use]
    pub fn triangulate_face_fan(&self, face: FaceId) -> Vec<[CornerId; 3]> {
        let corners = self.face_loop(face).collect::<Vec<_>>();
        if corners.len() < 3 {
            return Vec::new();
        }

        let c0 = corners[0];
        let mut triangles = Vec::with_capacity(corners.len().saturating_sub(2));
        for i in 1..corners.len() - 1 {
            triangles.push([c0, corners[i], corners[i + 1]]);
        }
        triangles
    }

    /// Enumerates one face's triangles under an explicit strategy.
    ///
    /// This is the single kernel-owned triangle enumeration: render
    /// extraction and the boolean broad phase both consume it, so a
    /// triangle index is meaningful only together with the strategy it was
    /// produced under (see ADR-0008).
    ///
    /// [`FaceTriangulation::Fan`] output is byte-identical to
    /// [`Mesh::triangulate_face_fan`]. [`FaceTriangulation::Robust`]
    /// projects the face loop onto its Newell best-fit plane and runs the
    /// shared deterministic triangulator; when the projected polygon is not
    /// simple it falls back to the fan deterministically (enumeration never
    /// fails — see [`Mesh::face_triangles_counted`] to observe fallbacks).
    #[must_use]
    pub fn face_triangles(&self, face: FaceId, strategy: FaceTriangulation) -> Vec<[CornerId; 3]> {
        self.face_triangles_counted(face, strategy).0
    }

    /// Like [`Mesh::face_triangles`], additionally reporting whether the
    /// robust strategy fell back to the fan for this face.
    #[must_use]
    pub fn face_triangles_counted(
        &self,
        face: FaceId,
        strategy: FaceTriangulation,
    ) -> (Vec<[CornerId; 3]>, bool) {
        match strategy {
            FaceTriangulation::Fan => (self.triangulate_face_fan(face), false),
            FaceTriangulation::Robust => {
                if let Some(triangles) = self.triangulate_face_robust(face) {
                    (triangles, false)
                } else {
                    (self.triangulate_face_fan(face), true)
                }
            }
        }
    }

    /// Robust triangulation of one face via plane projection; `None` when
    /// the face is degenerate or its projection is not a simple polygon.
    fn triangulate_face_robust(&self, face: FaceId) -> Option<Vec<[CornerId; 3]>> {
        let corners = self.face_loop(face).collect::<Vec<_>>();
        if corners.len() < 3 {
            return Some(Vec::new());
        }
        let mut points = Vec::with_capacity(corners.len());
        for &corner in &corners {
            let vertex = self.to_vertex(corner)?;
            let p = self.vertex_position(vertex)?;
            // f32 -> f64 promotion is exact; robustness downstream comes
            // from the triangulator's exact-sign predicates.
            points.push([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
        }

        // Newell normal: component k is twice the signed area of the
        // polygon projected along axis k.
        let mut normal = [0.0_f64; 3];
        for i in 0..points.len() {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
            normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
            normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        // Dominant axis: largest |component|, ties resolved by the lowest
        // axis index (deterministic, trig-free).
        let mut axis = 0;
        for k in 1..3 {
            if normal[k].abs() > normal[axis].abs() {
                axis = k;
            }
        }
        if normal[axis] == 0.0 || !normal[axis].is_finite() {
            return None;
        }
        // Project onto the plane axes in cyclic order; a negative dominant
        // component means the cyclic projection winds clockwise, so swap
        // the axes to hand the triangulator a counter-clockwise polygon.
        // The mirrored output triangles then wind consistently with the
        // face loop either way.
        let (u, v) = if normal[axis] > 0.0 {
            ((axis + 1) % 3, (axis + 2) % 3)
        } else {
            ((axis + 2) % 3, (axis + 1) % 3)
        };
        let projected: Vec<[f64; 2]> = points.iter().map(|p| [p[u], p[v]]).collect();
        let input = exedra_triangulate::PolygonInput {
            outer: &projected,
            holes: &[],
        };
        let result =
            exedra_triangulate::triangulate(&input, &exedra_triangulate::TriParams::default())
                .ok()?;
        Some(
            result
                .triangles
                .iter()
                .map(|t| {
                    [
                        corners[t[0] as usize],
                        corners[t[1] as usize],
                        corners[t[2] as usize],
                    ]
                })
                .collect(),
        )
    }

    fn normalize_boundary_seed(
        &self,
        seed_edge: HalfEdgeId,
    ) -> Result<HalfEdgeId, BoundaryLoopError> {
        let twin = self
            .twin(seed_edge)
            .ok_or(BoundaryLoopError::StaleHalfEdge)?;
        let seed_face = self
            .face(seed_edge)
            .ok_or(BoundaryLoopError::StaleHalfEdge)?;
        let twin_face = self.face(twin).ok_or(BoundaryLoopError::StaleHalfEdge)?;

        if seed_face == FaceId::OUTSIDE {
            Ok(seed_edge)
        } else if twin_face == FaceId::OUTSIDE {
            Ok(twin)
        } else {
            Err(BoundaryLoopError::NotBoundaryEdge)
        }
    }

    /// Builds a mesh from indexed triangles.
    ///
    /// `positions` contains input vertex coordinates. `indices` contains one
    /// triangle per entry, indexing into `positions`.
    ///
    /// On success this produces a valid half-edge mesh with explicit OUTSIDE
    /// boundary modeling:
    /// - every interior edge has a twin,
    /// - every open boundary edge gets a twin on [`FaceId::OUTSIDE`],
    /// - boundary `next` links form OUTSIDE loops.
    ///
    /// Construction is deterministic: identical inputs and params produce
    /// identical topology and ID assignment.
    ///
    /// Returns:
    /// - [`BuildError::InvalidWeldTolerance`] for invalid weld params,
    /// - [`BuildError::IndexOutOfBounds`] for invalid triangle indices,
    /// - [`BuildError::DegenerateTriangle`] for collapsed triangles,
    /// - [`BuildError::NonManifoldEdge`] for non-manifold edge usage,
    /// - [`BuildError::BoundaryStitchFailed`] when boundary loop linking is
    ///   ambiguous or incomplete.
    ///
    /// Boundary stitching currently performs linear candidate scans per boundary
    /// half-edge, so stitching cost is quadratic in boundary edge count.
    pub fn from_indexed_triangles(
        positions: &[[f32; 3]],
        indices: &[[u32; 3]],
        params: &BuildParams,
    ) -> Result<Self, BuildError> {
        if params
            .weld_tolerance
            .is_some_and(|t| !t.is_finite() || t < 0.0)
        {
            return Err(BuildError::InvalidWeldTolerance);
        }
        let (canonical_positions, remap) = weld_positions(positions, params.weld_tolerance);

        let mut mesh = Self::new();
        let mut vertex_ids = Vec::with_capacity(canonical_positions.len());
        for position in &canonical_positions {
            vertex_ids.push(mesh.add_vertex(*position));
        }

        let mut edge_records: Vec<(u32, u32, HalfEdgeId)> = Vec::with_capacity(indices.len() * 3);
        let mut endpoints: Vec<(u32, u32)> = Vec::with_capacity(indices.len() * 6);

        for (triangle, tri) in indices.iter().enumerate() {
            let i0 = remap_index(tri[0], &remap, triangle)?;
            let i1 = remap_index(tri[1], &remap, triangle)?;
            let i2 = remap_index(tri[2], &remap, triangle)?;
            if i0 == i1 || i1 == i2 || i2 == i0 {
                return Err(BuildError::DegenerateTriangle { triangle });
            }

            let v0 = *vertex_ids
                .get(i0 as usize)
                .ok_or(BuildError::IndexOutOfBounds {
                    triangle,
                    index: i0,
                })?;
            let v1 = *vertex_ids
                .get(i1 as usize)
                .ok_or(BuildError::IndexOutOfBounds {
                    triangle,
                    index: i1,
                })?;
            let v2 = *vertex_ids
                .get(i2 as usize)
                .ok_or(BuildError::IndexOutOfBounds {
                    triangle,
                    index: i2,
                })?;

            let face = FaceId::from(mesh.faces.insert(Face {
                edge: HalfEdgeId::INVALID,
                degree: 3,
            }));
            let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
                to: v1,
                face,
                next: HalfEdgeId::INVALID,
                twin: HalfEdgeId::INVALID,
            }));
            let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
                to: v2,
                face,
                next: HalfEdgeId::INVALID,
                twin: HalfEdgeId::INVALID,
            }));
            let h2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
                to: v0,
                face,
                next: HalfEdgeId::INVALID,
                twin: HalfEdgeId::INVALID,
            }));

            mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
            mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h2;
            mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").next = h0;
            mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;

            edge_records.push((i0, i1, h0));
            edge_records.push((i1, i2, h1));
            edge_records.push((i2, i0, h2));
            set_endpoint(&mut endpoints, h0, i0, i1);
            set_endpoint(&mut endpoints, h1, i1, i2);
            set_endpoint(&mut endpoints, h2, i2, i0);
        }

        let mut grouped: Vec<(u32, u32, u32, u32, HalfEdgeId)> = edge_records
            .iter()
            .map(|(from, to, hid)| (u32::min(*from, *to), u32::max(*from, *to), *from, *to, *hid))
            .collect();
        grouped.sort_by_key(|(a, b, from, to, hid)| (*a, *b, *from, *to, hid.index()));

        let mut boundary_interior: Vec<HalfEdgeId> = Vec::new();
        let mut cursor = 0_usize;
        while cursor < grouped.len() {
            let (a, b, _, _, _) = grouped[cursor];
            let mut end = cursor + 1;
            while end < grouped.len() && grouped[end].0 == a && grouped[end].1 == b {
                end += 1;
            }

            let group = &grouped[cursor..end];
            if group.len() == 1 {
                boundary_interior.push(group[0].4);
            } else if group.len() == 2 {
                let (_, _, from0, to0, h0) = group[0];
                let (_, _, from1, to1, h1) = group[1];
                if from0 == from1 || to0 == to1 {
                    return Err(BuildError::NonManifoldEdge {
                        a,
                        b,
                        count: group.len(),
                    });
                }
                mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").twin = h1;
                mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").twin = h0;
            } else {
                return Err(BuildError::NonManifoldEdge {
                    a,
                    b,
                    count: group.len(),
                });
            }
            cursor = end;
        }

        boundary_interior.sort_by_key(|id| id.index());
        let mut boundary_ids = Vec::with_capacity(boundary_interior.len());
        for interior in &boundary_interior {
            let (from, to) = get_endpoint(&endpoints, *interior);
            let boundary_to = *vertex_ids
                .get(from as usize)
                .expect("endpoint index must map to an existing vertex id");
            let boundary = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
                to: boundary_to,
                face: FaceId::OUTSIDE,
                next: HalfEdgeId::INVALID,
                twin: *interior,
            }));
            mesh.half_edges
                .get_mut(interior.as_id())
                .expect("interior edge live")
                .twin = boundary;
            set_endpoint(&mut endpoints, boundary, to, from);
            boundary_ids.push(boundary);
        }

        for boundary in &boundary_ids {
            let (_, to) = get_endpoint(&endpoints, *boundary);
            let candidates = boundary_ids
                .iter()
                .copied()
                .filter(|candidate| {
                    let (start, _) = get_endpoint(&endpoints, *candidate);
                    start == to
                })
                .collect::<Vec<_>>();
            if candidates.len() != 1 {
                return Err(BuildError::BoundaryStitchFailed {
                    vertex: to,
                    candidates: candidates.len(),
                });
            }
            mesh.half_edges
                .get_mut(boundary.as_id())
                .expect("boundary edge live")
                .next = candidates[0];
        }

        for (id, _) in mesh.half_edges.iter() {
            let half_edge = HalfEdgeId::from(id);
            let (from, _) = get_endpoint(&endpoints, half_edge);
            let vertex = *vertex_ids
                .get(from as usize)
                .expect("endpoint index must map to an existing vertex id");
            let out = &mut mesh
                .vertices
                .get_mut(vertex.as_id())
                .expect("vertex live")
                .out;
            if *out == HalfEdgeId::INVALID {
                *out = half_edge;
            }
        }

        mesh.sync_attr_capacities();
        Ok(mesh)
    }

    /// Builds a mesh from arbitrary polygon face loops (no triangulation).
    ///
    /// `positions` provides builder-local vertex positions.
    /// `face_loops` contains polygon loops as builder-local vertex indices.
    #[must_use = "construction can fail with structured BuildError"]
    pub fn from_polygons(
        positions: &[[f32; 3]],
        face_loops: &[&[u32]],
    ) -> Result<Self, BuildError> {
        let mut builder = MeshBuilder::new();
        for position in positions {
            let _ = builder.push_vertex(*position);
        }
        for loop_indices in face_loops {
            builder.add_face(loop_indices)?;
        }
        Ok(builder.build()?.mesh)
    }

    /// Runs cheap core invariant checks.
    #[must_use]
    pub fn validate_fast(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        let expected_vertex_cap = self.vertices.slot_count();
        let expected_face_cap = self.faces.slot_count();
        let expected_half_edge_cap = self.half_edges.slot_count();
        if self.attrs.domain_capacity(Domain::Vertex) != expected_vertex_cap {
            errors.push(ValidationError::DenseCapacityMismatch {
                domain: Domain::Vertex,
                name: "<domain-capacity>",
                expected: expected_vertex_cap,
                actual: self.attrs.domain_capacity(Domain::Vertex),
            });
        }
        if self.attrs.domain_capacity(Domain::Face) != expected_face_cap {
            errors.push(ValidationError::DenseCapacityMismatch {
                domain: Domain::Face,
                name: "<domain-capacity>",
                expected: expected_face_cap,
                actual: self.attrs.domain_capacity(Domain::Face),
            });
        }
        if self.attrs.domain_capacity(Domain::HalfEdge) != expected_half_edge_cap {
            errors.push(ValidationError::DenseCapacityMismatch {
                domain: Domain::HalfEdge,
                name: "<domain-capacity>",
                expected: expected_half_edge_cap,
                actual: self.attrs.domain_capacity(Domain::HalfEdge),
            });
        }
        for (domain, name, expected, actual) in self.attrs.dense_capacity_mismatches() {
            errors.push(ValidationError::DenseCapacityMismatch {
                domain,
                name,
                expected,
                actual,
            });
        }

        for (id, vertex) in self.vertices.iter() {
            if vertex.out != HalfEdgeId::INVALID
                && self.half_edges.get(vertex.out.as_id()).is_none()
            {
                errors.push(ValidationError::InvalidReference {
                    owner: "vertex",
                    owner_index: id.index(),
                    field: "out",
                    target_index: vertex.out.index(),
                });
            }
        }

        for (id, half_edge) in self.half_edges.iter() {
            let h = HalfEdgeId::from(id);
            if self.vertices.get(half_edge.to.as_id()).is_none() {
                errors.push(ValidationError::InvalidReference {
                    owner: "half_edge",
                    owner_index: id.index(),
                    field: "to",
                    target_index: half_edge.to.index(),
                });
            }
            if self.half_edges.get(half_edge.next.as_id()).is_none() {
                errors.push(ValidationError::InvalidReference {
                    owner: "half_edge",
                    owner_index: id.index(),
                    field: "next",
                    target_index: half_edge.next.index(),
                });
            }
            if self.half_edges.get(half_edge.twin.as_id()).is_none() {
                errors.push(ValidationError::InvalidReference {
                    owner: "half_edge",
                    owner_index: id.index(),
                    field: "twin",
                    target_index: half_edge.twin.index(),
                });
            }
            if half_edge.face != FaceId::OUTSIDE && self.faces.get(half_edge.face.as_id()).is_none()
            {
                errors.push(ValidationError::InvalidReference {
                    owner: "half_edge",
                    owner_index: id.index(),
                    field: "face",
                    target_index: half_edge.face.index(),
                });
            }

            if let Some(twin_record) = self.half_edges.get(half_edge.twin.as_id()) {
                let twin_of_twin = twin_record.twin;
                if twin_of_twin != h {
                    errors.push(ValidationError::TwinMismatch {
                        half_edge: h.index(),
                        twin: half_edge.twin.index(),
                        twin_of_twin: twin_of_twin.index(),
                    });
                }
            }
        }

        for (id, face) in self.faces.iter() {
            if self.half_edges.get(face.edge.as_id()).is_none() {
                errors.push(ValidationError::InvalidReference {
                    owner: "face",
                    owner_index: id.index(),
                    field: "edge",
                    target_index: face.edge.index(),
                });
                continue;
            }
            let mut cursor = face.edge;
            let mut ok = true;
            for _ in 0..face.degree {
                let Some(next) = self.half_edges.get(cursor.as_id()).map(|h| h.next) else {
                    ok = false;
                    break;
                };
                cursor = next;
            }
            if !ok || cursor != face.edge {
                errors.push(ValidationError::FaceLoopNotClosed { face: id.index() });
            }
        }

        errors
    }

    /// Runs deeper graph-walk validation checks (v0.1 partial coverage).
    #[must_use]
    pub fn validate_deep(&self) -> Vec<ValidationError> {
        let mut errors = self.validate_fast();
        let max_steps = self.half_edges.slot_count().max(1);

        for (face_id, face_record) in self.faces.iter() {
            let face = FaceId::from(face_id);
            let mut cursor = face_record.edge;
            let mut count = 0_usize;
            while let Some(record) = self.half_edges.get(cursor.as_id()) {
                if record.face != face {
                    errors.push(ValidationError::FaceLoopForeignHalfEdge {
                        face: face.index(),
                        half_edge: cursor.index(),
                        found_face: record.face.index(),
                    });
                    break;
                }
                count += 1;
                cursor = record.next;
                if cursor == face_record.edge {
                    break;
                }
                if count > max_steps {
                    errors.push(ValidationError::FaceLoopNotClosed { face: face.index() });
                    break;
                }
            }
            if count != face_record.degree as usize {
                errors.push(ValidationError::FaceDegreeMismatch {
                    face: face.index(),
                    cached: face_record.degree,
                    actual: count,
                });
            }
        }

        for (vertex_id, vertex_record) in self.vertices.iter() {
            if vertex_record.out == HalfEdgeId::INVALID {
                continue;
            }
            let mut cursor = vertex_record.out;
            let start = cursor;
            let mut closed = false;
            for _ in 0..max_steps {
                let Some(h) = self.half_edges.get(cursor.as_id()) else {
                    break;
                };
                let Some(twin) = self.half_edges.get(h.twin.as_id()) else {
                    break;
                };
                cursor = twin.next;
                if cursor == start {
                    closed = true;
                    break;
                }
            }
            if !closed {
                errors.push(ValidationError::VertexStarNotClosed {
                    vertex: vertex_id.index(),
                });
            }
        }

        let mut edge_keys = Vec::<(u32, u32)>::new();
        for (id, h) in self.half_edges.iter() {
            let twin = self.half_edges.get(h.twin.as_id());
            if h.face == FaceId::OUTSIDE && twin.is_some_and(|t| t.face == FaceId::OUTSIDE) {
                errors.push(ValidationError::BoundaryInvariantBroken {
                    half_edge: id.index(),
                });
            }

            let Some(twin) = twin else {
                continue;
            };
            let a = twin.to.index();
            let b = h.to.index();
            edge_keys.push((u32::min(a, b), u32::max(a, b)));
        }
        edge_keys.sort_unstable();
        let mut cursor = 0_usize;
        while cursor < edge_keys.len() {
            let (a, b) = edge_keys[cursor];
            let mut end = cursor + 1;
            while end < edge_keys.len() && edge_keys[end] == (a, b) {
                end += 1;
            }
            let count = end - cursor;
            if count != 2 {
                errors.push(ValidationError::EdgeMultiplicity { a, b, count });
            }
            cursor = end;
        }

        errors
    }

    fn sync_attr_capacities(&mut self) {
        self.attrs.sync_capacities(
            self.vertices.slot_count(),
            self.faces.slot_count(),
            self.half_edges.slot_count(),
        );
    }
}

impl MeshBuilder {
    /// Creates an empty polygon/ngon builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            vertices: Vec::new(),
            faces: Vec::new(),
            face_attrs: Vec::new(),
        }
    }

    /// Pushes a builder-local vertex and returns its local index.
    pub fn push_vertex(&mut self, position: [f32; 3]) -> u32 {
        let index = index_to_u32(self.vertices.len());
        self.vertices.push(position);
        index
    }

    /// Adds one polygon face loop (builder-local vertex indices).
    pub fn add_face(&mut self, loop_indices: &[u32]) -> Result<(), BuildError> {
        self.add_face_with_attrs(loop_indices, &FaceBuildAttrs::default())
    }

    /// Adds one polygon face loop plus optional build-time attributes.
    pub fn add_face_with_attrs(
        &mut self,
        loop_indices: &[u32],
        attrs: &FaceBuildAttrs<'_>,
    ) -> Result<(), BuildError> {
        let face = self.faces.len();
        if loop_indices.len() < 3 {
            return Err(BuildError::InvalidFaceLoop {
                face,
                kind: FaceLoopErrorKind::TooShort,
            });
        }
        for &index in loop_indices {
            if usize::try_from(index)
                .ok()
                .is_none_or(|i| i >= self.vertices.len())
            {
                return Err(BuildError::InvalidFaceLoop {
                    face,
                    kind: FaceLoopErrorKind::IndexOutOfBounds { index },
                });
            }
        }
        for edge in loop_indices.windows(2) {
            if edge[0] == edge[1] {
                return Err(BuildError::InvalidFaceLoop {
                    face,
                    kind: FaceLoopErrorKind::ZeroLengthEdge,
                });
            }
        }
        if loop_indices.first() == loop_indices.last() {
            return Err(BuildError::InvalidFaceLoop {
                face,
                kind: FaceLoopErrorKind::ZeroLengthEdge,
            });
        }
        for (i, &a) in loop_indices.iter().enumerate() {
            for &b in &loop_indices[(i + 1)..] {
                if a == b {
                    return Err(BuildError::InvalidFaceLoop {
                        face,
                        kind: FaceLoopErrorKind::RepeatedVertex,
                    });
                }
            }
        }

        self.faces.push(loop_indices.to_vec());
        self.face_attrs.push(OwnedFaceBuildAttrs::from_borrowed(
            face,
            loop_indices.len(),
            attrs,
        )?);
        Ok(())
    }

    /// Builds the final mesh and provenance mapping.
    pub fn build(&self) -> Result<MeshBuildResult, BuildError> {
        let mut mesh = Mesh::new();
        let mut vertex_ids = Vec::with_capacity(self.vertices.len());
        for position in &self.vertices {
            vertex_ids.push(mesh.add_vertex(*position));
        }

        let mut face_ids = Vec::with_capacity(self.faces.len());
        let mut face_edge_ids = Vec::with_capacity(self.faces.len());
        let mut edge_records: Vec<(u32, u32, HalfEdgeId)> = Vec::new();
        let mut endpoints: Vec<(u32, u32)> = Vec::new();

        for (face_index, loop_indices) in self.faces.iter().enumerate() {
            let degree = index_to_u32(loop_indices.len());
            let face = FaceId::from(mesh.faces.insert(Face {
                edge: HalfEdgeId::INVALID,
                degree,
            }));
            let mut loop_half_edges = Vec::with_capacity(loop_indices.len());

            for i in 0..loop_indices.len() {
                let from = loop_indices[i];
                let to = loop_indices[(i + 1) % loop_indices.len()];
                debug_assert_ne!(
                    from, to,
                    "MeshBuilder::add_face validates zero-length edges"
                );
                let to_vertex =
                    *vertex_ids
                        .get(to as usize)
                        .ok_or(BuildError::InvalidFaceLoop {
                            // Defensive-only check; add_face validates this.
                            face: face_index,
                            kind: FaceLoopErrorKind::IndexOutOfBounds { index: to },
                        })?;
                let hid = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
                    to: to_vertex,
                    face,
                    next: HalfEdgeId::INVALID,
                    twin: HalfEdgeId::INVALID,
                }));
                loop_half_edges.push(hid);
                edge_records.push((from, to, hid));
                set_endpoint(&mut endpoints, hid, from, to);
            }

            for i in 0..loop_half_edges.len() {
                let current = loop_half_edges[i];
                let next = loop_half_edges[(i + 1) % loop_half_edges.len()];
                mesh.half_edges
                    .get_mut(current.as_id())
                    .expect("newly created half-edge must be live")
                    .next = next;
            }
            mesh.faces
                .get_mut(face.as_id())
                .expect("newly created face must be live")
                .edge = loop_half_edges[0];
            face_ids.push(face);
            face_edge_ids.push(loop_half_edges);
        }

        let mut grouped: Vec<(u32, u32, u32, u32, HalfEdgeId)> = edge_records
            .iter()
            .map(|(from, to, hid)| (u32::min(*from, *to), u32::max(*from, *to), *from, *to, *hid))
            .collect();
        grouped.sort_by_key(|(a, b, from, to, hid)| (*a, *b, *from, *to, hid.index()));

        let mut boundary_interior = Vec::<HalfEdgeId>::new();
        let mut cursor = 0_usize;
        while cursor < grouped.len() {
            let (a, b, _, _, _) = grouped[cursor];
            let mut end = cursor + 1;
            while end < grouped.len() && grouped[end].0 == a && grouped[end].1 == b {
                end += 1;
            }
            let group = &grouped[cursor..end];
            if group.len() == 1 {
                boundary_interior.push(group[0].4);
            } else if group.len() == 2 {
                let (_, _, from0, to0, h0) = group[0];
                let (_, _, from1, to1, h1) = group[1];
                if from0 == from1 || to0 == to1 {
                    return Err(BuildError::NonManifoldEdge {
                        a,
                        b,
                        count: group.len(),
                    });
                }
                mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").twin = h1;
                mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").twin = h0;
            } else {
                return Err(BuildError::NonManifoldEdge {
                    a,
                    b,
                    count: group.len(),
                });
            }
            cursor = end;
        }

        boundary_interior.sort_by_key(|id| id.index());
        let mut boundary_ids = Vec::with_capacity(boundary_interior.len());
        for interior in &boundary_interior {
            let (from, to) = get_endpoint(&endpoints, *interior);
            let boundary_to = *vertex_ids
                .get(from as usize)
                .expect("endpoint index must map to an existing vertex id");
            let boundary = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
                to: boundary_to,
                face: FaceId::OUTSIDE,
                next: HalfEdgeId::INVALID,
                twin: *interior,
            }));
            mesh.half_edges
                .get_mut(interior.as_id())
                .expect("interior edge live")
                .twin = boundary;
            set_endpoint(&mut endpoints, boundary, to, from);
            boundary_ids.push(boundary);
        }

        for boundary in &boundary_ids {
            let (_, to) = get_endpoint(&endpoints, *boundary);
            let candidates = boundary_ids
                .iter()
                .copied()
                .filter(|candidate| {
                    let (start, _) = get_endpoint(&endpoints, *candidate);
                    start == to
                })
                .collect::<Vec<_>>();
            if candidates.len() != 1 {
                return Err(BuildError::BoundaryStitchFailed {
                    vertex: to,
                    candidates: candidates.len(),
                });
            }
            mesh.half_edges
                .get_mut(boundary.as_id())
                .expect("boundary edge live")
                .next = candidates[0];
        }

        for (id, _) in mesh.half_edges.iter() {
            let half_edge = HalfEdgeId::from(id);
            let (from, _) = get_endpoint(&endpoints, half_edge);
            let vertex = *vertex_ids
                .get(from as usize)
                .expect("endpoint index must map to an existing vertex id");
            let out = &mut mesh
                .vertices
                .get_mut(vertex.as_id())
                .expect("vertex live")
                .out;
            if *out == HalfEdgeId::INVALID {
                *out = half_edge;
            }
        }

        mesh.sync_attr_capacities();
        for (face_index, attrs) in self.face_attrs.iter().enumerate() {
            let face_id = face_ids[face_index];
            if let Some(region) = attrs.region {
                let layer = mesh
                    .attrs_mut()
                    .dense_mut(attr::FACE_REGION)
                    .expect("FACE_REGION must be registered");
                assert!(
                    layer.set(face_id.as_id(), region),
                    "face slot must exist after build"
                );
            }
            if let Some(edge_seams) = &attrs.edge_seams {
                for (half_edge, seam) in face_edge_ids[face_index].iter().zip(edge_seams) {
                    let _ = mesh.set_edge_seam(*half_edge, *seam);
                }
            }
            if let Some(edge_sharpness) = &attrs.edge_sharpness {
                for (half_edge, sharpness) in face_edge_ids[face_index].iter().zip(edge_sharpness) {
                    let _ = mesh.set_edge_sharpness(*half_edge, *sharpness);
                }
            }
        }
        Ok(MeshBuildResult {
            mesh,
            vertex_ids,
            face_ids,
            face_edge_ids,
        })
    }
}

impl OwnedFaceBuildAttrs {
    fn from_borrowed(
        face: usize,
        edge_count: usize,
        attrs: &FaceBuildAttrs<'_>,
    ) -> Result<Self, BuildError> {
        if let Some(edge_seams) = attrs.edge_seams
            && edge_seams.len() != edge_count
        {
            return Err(BuildError::InvalidFaceAttrs {
                face,
                kind: FaceAttrErrorKind::EdgeSeamLengthMismatch {
                    expected: edge_count,
                    actual: edge_seams.len(),
                },
            });
        }
        if let Some(edge_sharpness) = attrs.edge_sharpness
            && edge_sharpness.len() != edge_count
        {
            return Err(BuildError::InvalidFaceAttrs {
                face,
                kind: FaceAttrErrorKind::EdgeSharpnessLengthMismatch {
                    expected: edge_count,
                    actual: edge_sharpness.len(),
                },
            });
        }

        Ok(Self {
            region: attrs.region,
            edge_seams: attrs.edge_seams.map(|values| values.to_vec()),
            edge_sharpness: attrs.edge_sharpness.map(|values| values.to_vec()),
        })
    }
}

/// Iterator returned by [`Mesh::face_loop`].
///
/// Yields half-edges in deterministic loop order for one interior face.
#[derive(Debug)]
pub struct FaceLoopIter<'a> {
    mesh: &'a Mesh,
    face: FaceId,
    start: Option<HalfEdgeId>,
    current: Option<HalfEdgeId>,
    steps: usize,
    max_steps: usize,
    exhausted: bool,
}

impl Iterator for FaceLoopIter<'_> {
    type Item = HalfEdgeId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.exhausted {
            return None;
        }
        if self.steps >= self.max_steps {
            let start = self.start.map(HalfEdgeId::index).unwrap_or(u32::MAX);
            panic!(
                "mesh topology corruption: face_loop exceeded {} steps for face {} (start half-edge {})",
                self.max_steps,
                self.face.index(),
                start
            );
        }
        let current = self.current?;
        let next = self.mesh.next(current).unwrap_or_else(|| {
            panic!(
                "mesh topology corruption: face_loop encountered missing next from half-edge {} on face {}",
                current.index(),
                self.face.index()
            )
        });
        if Some(next) == self.start {
            self.exhausted = true;
        } else {
            self.current = Some(next);
        }
        self.steps += 1;
        Some(current)
    }
}

fn remap_index(index: u32, remap: &[u32], triangle: usize) -> Result<u32, BuildError> {
    remap
        .get(index as usize)
        .copied()
        .ok_or(BuildError::IndexOutOfBounds { triangle, index })
}

fn weld_positions(positions: &[[f32; 3]], tolerance: Option<f32>) -> (Vec<[f32; 3]>, Vec<u32>) {
    let Some(tol) = tolerance else {
        let mut remap = Vec::with_capacity(positions.len());
        for index in 0..positions.len() {
            remap.push(index_to_u32(index));
        }
        return (positions.to_vec(), remap);
    };

    let mut canonical = Vec::<[f32; 3]>::new();
    let mut remap = Vec::<u32>::with_capacity(positions.len());
    let tol_sq = tol * tol;
    for position in positions {
        let mut found = None;
        for (idx, candidate) in canonical.iter().enumerate() {
            let dx = position[0] - candidate[0];
            let dy = position[1] - candidate[1];
            let dz = position[2] - candidate[2];
            if dx * dx + dy * dy + dz * dz <= tol_sq {
                found = Some(index_to_u32(idx));
                break;
            }
        }
        if let Some(index) = found {
            remap.push(index);
        } else {
            let index = index_to_u32(canonical.len());
            canonical.push(*position);
            remap.push(index);
        }
    }
    (canonical, remap)
}

fn set_endpoint(endpoints: &mut Vec<(u32, u32)>, hid: HalfEdgeId, from: u32, to: u32) {
    let index = hid.index() as usize;
    if index >= endpoints.len() {
        endpoints.resize(index + 1, (u32::MAX, u32::MAX));
    }
    endpoints[index] = (from, to);
}

fn get_endpoint(endpoints: &[(u32, u32)], hid: HalfEdgeId) -> (u32, u32) {
    endpoints[hid.index() as usize]
}

fn index_to_u32(index: usize) -> u32 {
    u32::try_from(index).expect("index overflowed u32")
}

fn map_selected_face_patch_error(error: SelectedFacePatchError) -> SelectedFaceBoundaryError {
    match error {
        SelectedFacePatchError::OutsideFaceInSelection => {
            SelectedFaceBoundaryError::OutsideFaceInSelection
        }
        SelectedFacePatchError::StaleFace { face } => SelectedFaceBoundaryError::StaleFace { face },
        SelectedFacePatchError::InvalidFaceLoop { face } => {
            SelectedFaceBoundaryError::InvalidFaceLoop { face }
        }
    }
}

fn canonical_vertex_pair(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use super::FaceTriangulation;
    use crate::CornerId;

    use super::{
        BoundaryLoopError, BuildError, BuildParams, ConnectedFaceRegionError, FaceAttrErrorKind,
        FaceBuildAttrs, FaceLoopErrorKind, Mesh, MeshBuilder, SelectedFaceBoundaryError,
        SelectedFacePatchError, ValidationError,
    };
    use crate::attributes::{AttrKey, Domain};
    use crate::{
        ChangeSetBuilder, DeletePolicy, Face, FaceId, HalfEdge, HalfEdgeId, Id, VertexId, op,
        op::set_face_region,
    };

    fn triangle_mesh() -> Mesh {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([0.0, 1.0, 0.0]);
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 3,
        }));
        let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h2;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").next = h0;
        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").twin = b0;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").twin = b1;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").twin = b2;

        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").next = b2;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").next = b0;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").next = b1;
        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").twin = h0;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").twin = h1;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").twin = h2;

        mesh.vertices.get_mut(v0.as_id()).expect("v0 live").out = h0;
        mesh.vertices.get_mut(v1.as_id()).expect("v1 live").out = h1;
        mesh.vertices.get_mut(v2.as_id()).expect("v2 live").out = h2;
        mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;

        mesh
    }

    fn quad_mesh_open_boundary() -> (Mesh, FaceId) {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([1.0, 1.0, 0.0]);
        let v3 = mesh.add_vertex([0.0, 1.0, 0.0]);
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 4,
        }));

        let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v3,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h3 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        let b0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b3 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v3,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h2;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").next = h3;
        mesh.half_edges.get_mut(h3.as_id()).expect("h3 live").next = h0;
        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").twin = b0;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").twin = b1;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").twin = b2;
        mesh.half_edges.get_mut(h3.as_id()).expect("h3 live").twin = b3;

        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").next = b3;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").next = b0;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").next = b1;
        mesh.half_edges.get_mut(b3.as_id()).expect("b3 live").next = b2;
        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").twin = h0;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").twin = h1;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").twin = h2;
        mesh.half_edges.get_mut(b3.as_id()).expect("b3 live").twin = h3;

        mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;
        mesh.vertices.get_mut(v0.as_id()).expect("v0 live").out = h0;
        mesh.vertices.get_mut(v1.as_id()).expect("v1 live").out = h1;
        mesh.vertices.get_mut(v2.as_id()).expect("v2 live").out = h2;
        mesh.vertices.get_mut(v3.as_id()).expect("v3 live").out = h3;
        (mesh, face)
    }

    fn pentagon_mesh_open_boundary() -> (Mesh, FaceId) {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([1.5, 0.8, 0.0]);
        let v3 = mesh.add_vertex([0.5, 1.4, 0.0]);
        let v4 = mesh.add_vertex([-0.2, 0.7, 0.0]);
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 5,
        }));

        let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v3,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h3 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v4,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h4 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        let b0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b3 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v3,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b4 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v4,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h2;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").next = h3;
        mesh.half_edges.get_mut(h3.as_id()).expect("h3 live").next = h4;
        mesh.half_edges.get_mut(h4.as_id()).expect("h4 live").next = h0;
        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").twin = b0;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").twin = b1;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").twin = b2;
        mesh.half_edges.get_mut(h3.as_id()).expect("h3 live").twin = b3;
        mesh.half_edges.get_mut(h4.as_id()).expect("h4 live").twin = b4;

        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").next = b4;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").next = b0;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").next = b1;
        mesh.half_edges.get_mut(b3.as_id()).expect("b3 live").next = b2;
        mesh.half_edges.get_mut(b4.as_id()).expect("b4 live").next = b3;
        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").twin = h0;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").twin = h1;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").twin = h2;
        mesh.half_edges.get_mut(b3.as_id()).expect("b3 live").twin = h3;
        mesh.half_edges.get_mut(b4.as_id()).expect("b4 live").twin = h4;

        mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;
        mesh.vertices.get_mut(v0.as_id()).expect("v0 live").out = h0;
        mesh.vertices.get_mut(v1.as_id()).expect("v1 live").out = h1;
        mesh.vertices.get_mut(v2.as_id()).expect("v2 live").out = h2;
        mesh.vertices.get_mut(v3.as_id()).expect("v3 live").out = h3;
        mesh.vertices.get_mut(v4.as_id()).expect("v4 live").out = h4;
        (mesh, face)
    }

    #[test]
    fn compact_remaps_live_topology_and_attributes() {
        let mut builder = MeshBuilder::new();
        for position in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [3.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
        ] {
            let _ = builder.push_vertex(position);
        }
        builder.add_face(&[0, 1, 2]).expect("first face");
        builder.add_face(&[3, 4, 5]).expect("second face");
        let result = builder.build().expect("mesh should build");

        let mut mesh = result.mesh;
        let deleted_vertex = mesh.add_vertex([9.0, 9.0, 9.0]);
        assert!(mesh.vertices.remove(deleted_vertex.as_id()).is_some());
        let deleted_face = result.face_ids[0];
        let kept_face = result.face_ids[1];
        let deleted_corner = result.face_edge_ids[0][0];
        let kept_corner = result.face_edge_ids[1][0];
        let kept_vertex = result.vertex_ids[3];
        {
            let mut edit = mesh.edit_with(ChangeSetBuilder::new());
            op::set_vertex_sharpness(&mut edit, kept_vertex, f32::INFINITY)
                .expect("set kept vertex sharpness");
            set_face_region(&mut edit, kept_face, 23).expect("set kept face region");
            op::set_corner_uv(&mut edit, kept_corner, [0.25, 0.75]).expect("set kept corner uv");
            op::set_edge_seam(&mut edit, kept_corner, true).expect("set kept edge seam");
            op::set_edge_sharpness(&mut edit, kept_corner, 2.5).expect("set kept edge sharpness");
            op::delete_faces(&mut edit, &[deleted_face], DeletePolicy::CleanupIsolated)
                .expect("delete first face");
            let _ = edit.finish();
        }
        assert!(mesh.validate_deep().is_empty());
        assert!(mesh.vertices.slot_count() > mesh.vertices.len());
        assert!(mesh.half_edges.slot_count() > mesh.half_edges.len());
        assert!(mesh.faces.slot_count() > mesh.faces.len());

        let revision = mesh.revision();
        let (compacted, remap) = mesh.compact();

        assert_eq!(compacted.revision(), revision);
        assert!(compacted.validate_deep().is_empty());
        assert_eq!(compacted.vertices.slot_count(), compacted.vertices.len());
        assert_eq!(
            compacted.half_edges.slot_count(),
            compacted.half_edges.len()
        );
        assert_eq!(compacted.faces.slot_count(), compacted.faces.len());

        assert_eq!(remap.face(FaceId::OUTSIDE), Some(FaceId::OUTSIDE));
        assert_eq!(remap.vertex(deleted_vertex), None);
        assert_eq!(remap.face(deleted_face), None);
        assert_eq!(remap.half_edge(deleted_corner), None);

        let new_face = remap.face(kept_face).expect("kept face should map");
        let new_corner = remap
            .corner(kept_corner)
            .expect("kept corner should map through half-edge domain");
        let new_vertex = remap.vertex(kept_vertex).expect("kept vertex should map");

        assert_eq!(
            compacted
                .attrs()
                .dense(crate::attr::FACE_REGION)
                .expect("face region layer")
                .get(new_face.as_id())
                .copied(),
            Some(23)
        );
        assert_eq!(
            compacted
                .attrs()
                .sparse(crate::attr::CORNER_UV)
                .expect("corner uv layer")
                .get(new_corner.as_id())
                .copied(),
            Some([0.25, 0.75])
        );
        assert_eq!(compacted.edge_seam(new_corner), Some(true));
        assert_eq!(compacted.edge_sharpness(new_corner), Some(2.5));
        assert_eq!(
            compacted.vertex_position(new_vertex).copied(),
            Some([3.0, 0.0, 0.0])
        );
        assert_eq!(compacted.vertex_sharpness(new_vertex), Some(f32::INFINITY));

        let (compacted_again, remap_again) = mesh.compact();
        assert_eq!(remap, remap_again);
        assert_eq!(compacted_again.vertices.len(), compacted.vertices.len());
        assert_eq!(compacted_again.half_edges.len(), compacted.half_edges.len());
        assert_eq!(compacted_again.faces.len(), compacted.faces.len());
    }

    #[test]
    fn compact_remap_rejects_stale_generation_after_slot_reuse() {
        let mut mesh = Mesh::new();
        let stale = mesh.add_vertex([0.0, 0.0, 0.0]);
        assert!(mesh.vertices.remove(stale.as_id()).is_some());
        let live = mesh.add_vertex([1.0, 0.0, 0.0]);
        assert_eq!(live.index(), stale.index());
        assert_ne!(live.generation(), stale.generation());

        let (compacted, remap) = mesh.compact();

        assert!(compacted.validate_fast().is_empty());
        assert_eq!(compacted.vertices.slot_count(), 1);
        assert_eq!(remap.vertex(stale), None);
        assert_eq!(remap.vertex(live), Some(VertexId::new(0, NonZeroU32::MIN)));
    }

    #[test]
    fn traversal_accessors_work_on_triangle() {
        let mesh = triangle_mesh();
        let face = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let loop_ids: Vec<_> = mesh.face_loop(face).collect();
        assert_eq!(loop_ids.len(), 3);
        let h0 = loop_ids[0];
        let h1 = mesh.next(h0).expect("next exists");
        assert_eq!(mesh.prev(h1), Some(h0));
        let twin = mesh.twin(h0).expect("twin exists");
        assert_eq!(mesh.face(twin), Some(FaceId::OUTSIDE));
        assert_ne!(mesh.from_vertex(h0), mesh.to_vertex(h0));
    }

    #[test]
    fn face_loop_walks_quad() {
        let (mesh, face) = quad_mesh_open_boundary();
        let loop_ids: Vec<_> = mesh.face_loop(face).collect();
        assert_eq!(loop_ids.len(), 4);
    }

    #[test]
    fn vertex_star_includes_boundary_half_edges_for_open_mesh() {
        let (mesh, _) = quad_mesh_open_boundary();
        let v0 = VertexId::from(Id::new(0, NonZeroU32::MIN));
        let star: Vec<_> = mesh.vertex_star(v0).collect();
        assert_eq!(star.len(), 2);
        assert!(
            star.iter()
                .any(|h| mesh.face(*h).expect("live half-edge") == FaceId::OUTSIDE)
        );
    }

    #[test]
    fn vertices_iterates_live_vertices_in_slot_order() {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([2.0, 0.0, 0.0]);
        let removed = mesh.vertices.remove(v1.as_id());
        assert!(removed.is_some());
        let v3 = mesh.add_vertex([3.0, 0.0, 0.0]);

        let vertices: Vec<_> = mesh.vertices().collect();
        assert_eq!(vertices, vec![v0, v3, v2]);
    }

    #[test]
    fn faces_iterates_live_faces_in_slot_order() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 1.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder.push_vertex([2.0, 0.0, 0.0]);
        builder.push_vertex([2.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("face should be valid");
        builder
            .add_face(&[1, 4, 5, 2])
            .expect("face should be valid");
        let mut mesh = builder.build().expect("mesh should build").mesh;
        let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        let removed = mesh.faces.remove(f0.as_id());
        assert!(removed.is_some());
        let f2 = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 3,
        }));

        let faces: Vec<_> = mesh.faces().collect();
        assert_eq!(faces, vec![f2, f1]);
    }

    #[test]
    fn mesh_is_cloneable() {
        let (mesh, face) = quad_mesh_open_boundary();
        let clone = mesh.clone();
        let original_loop: Vec<_> = mesh.face_loop(face).collect();
        let clone_loop: Vec<_> = clone.face_loop(face).collect();
        assert_eq!(original_loop, clone_loop);
    }

    #[test]
    fn prev_walk_works_for_ngons() {
        let (mesh, face) = pentagon_mesh_open_boundary();
        let loop_ids: Vec<_> = mesh.face_loop(face).collect();
        assert_eq!(loop_ids.len(), 5);
        for (index, current) in loop_ids.iter().copied().enumerate() {
            let expected_prev = loop_ids[(index + loop_ids.len() - 1) % loop_ids.len()];
            assert_eq!(mesh.prev(current), Some(expected_prev));
        }
    }

    #[test]
    #[should_panic(expected = "prev() exceeded")]
    fn prev_panics_when_face_loop_does_not_reach_target() {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([0.0, 1.0, 0.0]);
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 3,
        }));
        let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        // Corrupt topology: h0 points to h1, but h1 points to itself.
        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h1;
        mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;
        mesh.vertices.get_mut(v0.as_id()).expect("v0 live").out = h0;
        mesh.vertices.get_mut(v1.as_id()).expect("v1 live").out = h1;
        mesh.vertices.get_mut(v2.as_id()).expect("v2 live").out = h1;

        let _ = mesh.prev(h0);
    }

    #[test]
    #[should_panic(expected = "face_loop exceeded")]
    fn face_loop_panics_when_loop_does_not_close() {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([0.0, 1.0, 0.0]);
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 3,
        }));
        let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;

        // Corrupt topology: loop never returns to h0.
        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h2;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").next = h1;

        let _: Vec<_> = mesh.face_loop(face).collect();
    }

    #[test]
    fn required_vertex_positions_can_get_and_set() {
        let mut mesh = Mesh::new();
        let vertex = mesh.add_vertex([1.0, 2.0, 3.0]);
        assert_eq!(mesh.vertex_position(vertex), Some(&[1.0, 2.0, 3.0]));
        assert!(mesh.set_vertex_position(vertex, [4.0, 5.0, 6.0]));
        assert_eq!(mesh.vertex_position(vertex), Some(&[4.0, 5.0, 6.0]));
    }

    #[test]
    fn mesh_default_matches_new_for_builtin_attributes() {
        let mut mesh = Mesh::default();
        let vertex = mesh.add_vertex([1.0, 2.0, 3.0]);
        assert_eq!(mesh.vertex_position(vertex), Some(&[1.0, 2.0, 3.0]));
    }

    #[test]
    fn from_indexed_triangles_single_triangle_has_boundary_loop() {
        let mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("single triangle should build");

        let boundary = mesh
            .half_edges
            .iter()
            .filter(|(_, h)| h.face == FaceId::OUTSIDE)
            .count();
        assert_eq!(boundary, 3);
    }

    #[test]
    fn from_indexed_triangles_shared_edge_links_twins() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [2, 1, 3]],
            &BuildParams::default(),
        )
        .expect("two triangles should build");

        let interior_pairs = mesh
            .half_edges
            .iter()
            .filter(|(id, h)| {
                h.face != FaceId::OUTSIDE
                    && mesh.half_edges.get(h.twin.as_id()).is_some_and(|twin| {
                        twin.face != FaceId::OUTSIDE && id.index() < h.twin.index()
                    })
            })
            .count();
        assert_eq!(interior_pairs, 1);
    }

    #[test]
    fn from_indexed_triangles_open_quad_has_four_boundary_edges() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("open quad should build");

        let boundary = mesh
            .half_edges
            .iter()
            .filter(|(_, h)| h.face == FaceId::OUTSIDE)
            .count();
        assert_eq!(boundary, 4);
    }

    #[test]
    fn from_indexed_triangles_closed_tetra_has_no_boundary() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.5, 0.8, 0.0],
                [0.5, 0.3, 0.9],
            ],
            &[[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]],
            &BuildParams::default(),
        )
        .expect("tetrahedron should build");

        let boundary = mesh
            .half_edges
            .iter()
            .filter(|(_, h)| h.face == FaceId::OUTSIDE)
            .count();
        assert_eq!(boundary, 0);
    }

    #[test]
    fn from_indexed_triangles_weld_tolerance_merges_nearby_vertices() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0005, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 3], [2, 3, 1]],
            &BuildParams {
                weld_tolerance: Some(0.001),
            },
        )
        .expect("welded mesh should build");

        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.faces.len(), 2);
    }

    #[test]
    fn from_indexed_triangles_reports_index_out_of_bounds() {
        let result = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 3]],
            &BuildParams::default(),
        );
        assert!(matches!(
            result,
            Err(BuildError::IndexOutOfBounds {
                triangle: 0,
                index: 3
            })
        ));
    }

    #[test]
    fn from_indexed_triangles_reports_degenerate_triangle_directly() {
        let result = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            &[[0, 0, 1]],
            &BuildParams::default(),
        );
        assert!(matches!(
            result,
            Err(BuildError::DegenerateTriangle { triangle: 0 })
        ));
    }

    #[test]
    fn from_indexed_triangles_reports_degenerate_triangle_after_weld() {
        let result = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [0.0005, 0.0, 0.0], [1.0, 0.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams {
                weld_tolerance: Some(0.001),
            },
        );
        assert!(matches!(
            result,
            Err(BuildError::DegenerateTriangle { triangle: 0 })
        ));
    }

    #[test]
    fn from_indexed_triangles_reports_non_manifold_edge() {
        let result = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],  // 0
                [1.0, 0.0, 0.0],  // 1
                [0.0, 1.0, 0.0],  // 2
                [0.0, -1.0, 0.0], // 3
                [1.0, 1.0, 0.0],  // 4
            ],
            &[[0, 1, 2], [1, 0, 3], [0, 1, 4]],
            &BuildParams::default(),
        );
        assert!(matches!(
            result,
            Err(BuildError::NonManifoldEdge {
                a: 0,
                b: 1,
                count: 3
            })
        ));
    }

    #[test]
    fn from_indexed_triangles_rejects_negative_weld_tolerance() {
        let result = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams {
                weld_tolerance: Some(-0.001),
            },
        );
        assert!(matches!(result, Err(BuildError::InvalidWeldTolerance)));
    }

    #[test]
    fn from_indexed_triangles_reports_boundary_stitch_ambiguity() {
        let result = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [-1.0, 0.0, 0.0],
                [0.0, -1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 3, 4]],
            &BuildParams::default(),
        );
        assert!(matches!(
            result,
            Err(BuildError::BoundaryStitchFailed {
                vertex: 0,
                candidates: 2,
            })
        ));
    }

    #[test]
    fn from_indexed_triangles_is_deterministic() {
        let positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let triangles = [[0, 1, 2], [0, 2, 3]];
        let params = BuildParams::default();

        let a = Mesh::from_indexed_triangles(&positions, &triangles, &params).expect("build A");
        let b = Mesh::from_indexed_triangles(&positions, &triangles, &params).expect("build B");

        let verts_a: Vec<_> = a.vertices.iter().map(|(id, v)| (id, *v)).collect();
        let verts_b: Vec<_> = b.vertices.iter().map(|(id, v)| (id, *v)).collect();
        assert_eq!(verts_a, verts_b);

        let half_edges_a: Vec<_> = a.half_edges.iter().map(|(id, h)| (id, *h)).collect();
        let half_edges_b: Vec<_> = b.half_edges.iter().map(|(id, h)| (id, *h)).collect();
        assert_eq!(half_edges_a, half_edges_b);

        let faces_a: Vec<_> = a.faces.iter().map(|(id, f)| (id, *f)).collect();
        let faces_b: Vec<_> = b.faces.iter().map(|(id, f)| (id, *f)).collect();
        assert_eq!(faces_a, faces_b);
    }

    #[test]
    fn from_indexed_triangles_empty_input_is_empty_mesh() {
        let mesh =
            Mesh::from_indexed_triangles(&[], &[], &BuildParams::default()).expect("empty builds");
        assert_eq!(mesh.vertices.len(), 0);
        assert_eq!(mesh.faces.len(), 0);
        assert_eq!(mesh.half_edges.len(), 0);
    }

    #[test]
    fn from_indexed_triangles_sets_expected_vertex_positions() {
        let mesh = Mesh::from_indexed_triangles(
            &[[2.0, 3.0, 5.0], [7.0, 11.0, 13.0], [17.0, 19.0, 23.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle should build");

        let positions: Vec<_> = mesh
            .vertices
            .iter()
            .map(|(id, _)| {
                *mesh
                    .vertex_position(VertexId::from(id))
                    .expect("position set")
            })
            .collect();
        assert_eq!(
            positions,
            vec![[2.0, 3.0, 5.0], [7.0, 11.0, 13.0], [17.0, 19.0, 23.0]]
        );
    }

    #[test]
    fn from_indexed_triangles_has_twin_involution() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("open quad should build");

        for (id, _) in mesh.half_edges.iter() {
            let h = HalfEdgeId::from(id);
            let twin = mesh.twin(h).expect("every half-edge has twin");
            assert_eq!(mesh.twin(twin), Some(h));
        }
    }

    fn count_boundary_loops(mesh: &Mesh) -> usize {
        mesh.boundary_loops()
            .expect("boundary loops should extract")
            .len()
    }

    #[test]
    fn mesh_builder_quad_is_single_ngon_face() {
        let mut builder = MeshBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([1.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([1.0, 1.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 1.0, 0.0]);
        builder.add_face(&[v0, v1, v2, v3]).expect("quad accepted");
        let result = builder.build().expect("quad builds");
        let mesh = result.mesh;

        assert_eq!(mesh.faces.len(), 1);
        let face = result.face_ids[0];
        let record = mesh.faces.get(face.as_id()).expect("face live");
        assert_eq!(record.degree, 4);
        assert_eq!(mesh.face_loop(face).count(), 4);
        assert_eq!(
            mesh.half_edges
                .iter()
                .filter(|(_, h)| h.face == FaceId::OUTSIDE)
                .count(),
            4
        );
        assert_eq!(count_boundary_loops(&mesh), 1);
    }

    #[test]
    fn boundary_loop_accepts_interior_seed_by_normalizing_to_outside() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("open quad should build");
        let face = mesh.faces().next().expect("face should exist");
        let interior_seed = mesh.face_loop(face).next().expect("face loop not empty");

        let boundary_loop = mesh
            .boundary_loop(interior_seed)
            .expect("boundary loop should extract");
        assert_eq!(boundary_loop.len(), 4);
        assert!(
            boundary_loop
                .iter()
                .all(|edge| mesh.face(*edge) == Some(FaceId::OUTSIDE))
        );
    }

    #[test]
    fn boundary_loop_rejects_non_boundary_seed() {
        let mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
            &[&[0, 1, 4, 3], &[1, 2, 5, 4]],
        )
        .expect("mesh build should succeed");
        let interior_edge = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|edge| {
                mesh.twin(*edge)
                    .and_then(|twin| mesh.face(twin))
                    .is_some_and(|adjacent| adjacent != FaceId::OUTSIDE)
            })
            .expect("expected interior edge");

        let error = mesh
            .boundary_loop(interior_edge)
            .expect_err("interior edge should be rejected");
        assert_eq!(error, BoundaryLoopError::NotBoundaryEdge);
    }

    #[test]
    fn selected_face_boundary_loops_extract_single_patch_loop() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("open quad should build");
        let faces = mesh.faces().collect::<Vec<_>>();

        let loops = mesh
            .selected_face_boundary_loops(&faces)
            .expect("selected patch boundary should extract");
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 4);
    }

    #[test]
    fn selected_face_boundary_loops_extract_disjoint_loops_in_stable_order() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.0, 1.0, 0.0],
                [3.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [4, 5, 6]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();

        let loops = mesh
            .selected_face_boundary_loops(&faces)
            .expect("selected patch boundary should extract");
        assert_eq!(loops.len(), 2);
        assert!(
            loops[0][0] < loops[1][0],
            "loop ordering should follow stable seed ordering"
        );
    }

    #[test]
    fn selected_face_boundary_loops_reject_stale_face() {
        let mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad mesh should build");

        let stale_face = FaceId::new(999, NonZeroU32::MIN);
        let error = mesh
            .selected_face_boundary_loops(&[stale_face])
            .expect_err("stale face should be rejected");
        assert_eq!(
            error,
            SelectedFaceBoundaryError::StaleFace { face: stale_face }
        );
    }

    #[test]
    fn selected_face_patch_topology_classifies_shared_edges_and_incident_vertices() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();

        let patch = mesh
            .selected_face_patch_topology(&faces)
            .expect("selected face patch topology should extract");
        assert_eq!(patch.boundary_edges.len(), 4);
        assert_eq!(patch.interior_edges.len(), 1);
        assert_eq!(patch.interior_edges[0].incidences.len(), 2);
        assert_eq!(patch.incident_vertices.len(), 4);
        assert!(patch.boundary_lies_on_mesh_boundary);
    }

    #[test]
    fn selected_face_patch_topology_rejects_outside_face() {
        let (mesh, _) = quad_mesh_open_boundary();
        let error = mesh
            .selected_face_patch_topology(&[FaceId::OUTSIDE])
            .expect_err("outside face should be rejected");
        assert_eq!(error, SelectedFacePatchError::OutsideFaceInSelection);
    }

    #[test]
    fn selected_face_patch_topology_rejects_stale_face() {
        let (mesh, _) = quad_mesh_open_boundary();
        let stale_face = FaceId::new(0, NonZeroU32::new(99).expect("literal should be non-zero"));
        let error = mesh
            .selected_face_patch_topology(&[stale_face])
            .expect_err("stale face should be rejected");
        assert_eq!(
            error,
            SelectedFacePatchError::StaleFace { face: stale_face }
        );
    }

    #[test]
    fn selected_face_patch_topology_canonicalizes_unsorted_duplicate_input() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();

        let patch = mesh
            .selected_face_patch_topology(&[faces[1], faces[0], faces[1]])
            .expect("selected face patch topology should extract");
        assert_eq!(patch.boundary_edges.len(), 4);
        assert!(
            patch.boundary_edges.windows(2).all(|pair| {
                (
                    pair[0].face.index(),
                    pair[0].edge_index,
                    pair[0].edge.index(),
                ) <= (
                    pair[1].face.index(),
                    pair[1].edge_index,
                    pair[1].edge.index(),
                )
            }),
            "boundary edges should be sorted deterministically"
        );
        assert_eq!(patch.incident_vertices.len(), 4);
        assert!(
            patch
                .incident_vertices
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "incident vertices should be deduplicated into deterministic order"
        );
    }

    #[test]
    fn connected_face_region_returns_sorted_connected_component() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 1.0, 0.0],
                [3.0, 1.0, 0.0],
            ],
            &[&[0, 1, 5, 4], &[1, 2, 6, 5], &[2, 3, 7, 6]],
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();

        let mut edit = mesh.edit_with(ChangeSetBuilder::new());
        set_face_region(&mut edit, faces[0], 11).expect("set region on first face");
        set_face_region(&mut edit, faces[1], 11).expect("set region on second face");
        set_face_region(&mut edit, faces[2], 11).expect("set region on third face");
        let _ = edit.finish();

        let connected = mesh
            .connected_face_region(faces[1])
            .expect("connected face region should extract");
        assert_eq!(connected, vec![faces[0], faces[1], faces[2]]);
    }

    #[test]
    fn connected_face_region_respects_region_boundaries() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
            &[&[0, 1, 4, 3], &[1, 2, 5, 4]],
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();

        let mut edit = mesh.edit_with(ChangeSetBuilder::new());
        set_face_region(&mut edit, faces[0], 7).expect("set region on first face");
        set_face_region(&mut edit, faces[1], 9).expect("set region on second face");
        let _ = edit.finish();

        let connected = mesh
            .connected_face_region(faces[1])
            .expect("connected face region should extract");
        assert_eq!(connected, vec![faces[1]]);
    }

    #[test]
    fn connected_face_region_rejects_outside_seed() {
        let (mesh, _) = quad_mesh_open_boundary();
        let error = mesh
            .connected_face_region(FaceId::OUTSIDE)
            .expect_err("outside seed should fail");
        assert_eq!(error, ConnectedFaceRegionError::OutsideSeedFace);
    }

    #[test]
    fn connected_face_region_rejects_stale_seed() {
        let (mesh, _) = quad_mesh_open_boundary();
        let stale_face = FaceId::new(0, NonZeroU32::new(99).expect("literal should be non-zero"));
        let error = mesh
            .connected_face_region(stale_face)
            .expect_err("stale seed should fail");
        assert_eq!(
            error,
            ConnectedFaceRegionError::StaleSeedFace { face: stale_face }
        );
    }

    #[test]
    fn mesh_builder_mixed_degree_faces_build() {
        let mut builder = MeshBuilder::new();
        for position in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
        ] {
            let _ = builder.push_vertex(position);
        }
        builder.add_face(&[0, 1, 2, 3]).expect("quad");
        builder.add_face(&[1, 4, 2]).expect("triangle");
        let result = builder.build().expect("mixed build");
        let mesh = result.mesh;
        assert_eq!(mesh.faces.len(), 2);
        let d0 = mesh
            .faces
            .get(result.face_ids[0].as_id())
            .expect("f0")
            .degree;
        let d1 = mesh
            .faces
            .get(result.face_ids[1].as_id())
            .expect("f1")
            .degree;
        assert_eq!((d0, d1), (4, 3));
    }

    #[test]
    fn from_polygons_empty_input_is_empty_mesh() {
        let mesh = Mesh::from_polygons(&[], &[]).expect("empty polygons build");
        assert_eq!(mesh.vertices.len(), 0);
        assert_eq!(mesh.faces.len(), 0);
        assert_eq!(mesh.half_edges.len(), 0);
    }

    #[test]
    fn mesh_builder_validation_errors_are_structured() {
        let mut builder = MeshBuilder::new();
        let _ = builder.push_vertex([0.0, 0.0, 0.0]);
        let _ = builder.push_vertex([1.0, 0.0, 0.0]);
        let _ = builder.push_vertex([0.0, 1.0, 0.0]);
        assert!(matches!(
            builder.add_face(&[0, 1]),
            Err(BuildError::InvalidFaceLoop {
                kind: FaceLoopErrorKind::TooShort,
                ..
            })
        ));
        assert!(matches!(
            builder.add_face(&[0, 1, 1]),
            Err(BuildError::InvalidFaceLoop {
                kind: FaceLoopErrorKind::ZeroLengthEdge,
                ..
            })
        ));
        assert!(matches!(
            builder.add_face(&[0, 1, 2, 1]),
            Err(BuildError::InvalidFaceLoop {
                kind: FaceLoopErrorKind::RepeatedVertex,
                ..
            })
        ));
        assert!(matches!(
            builder.add_face(&[0, 1, 99]),
            Err(BuildError::InvalidFaceLoop {
                kind: FaceLoopErrorKind::IndexOutOfBounds { index: 99 },
                ..
            })
        ));
    }

    #[test]
    fn triangulate_face_fan_triangle_passthrough() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2])
            .expect("triangle face is valid");
        let built = builder.build().expect("build should succeed");
        let face = built.face_ids[0];

        let triangles = built.mesh.triangulate_face_fan(face);
        assert_eq!(triangles.len(), 1);
        let loop_corners = built.mesh.face_loop(face).collect::<Vec<_>>();
        assert_eq!(
            triangles[0],
            [loop_corners[0], loop_corners[1], loop_corners[2]]
        );
    }

    /// Builds a single-face mesh from an XY polygon.
    fn single_ngon(points: &[[f32; 2]]) -> (Mesh, FaceId) {
        let mut builder = MeshBuilder::new();
        for p in points {
            builder.push_vertex([p[0], p[1], 0.0]);
        }
        let indices: Vec<u32> = (0..).take(points.len()).collect();
        builder.add_face(&indices).expect("ngon face is valid");
        let built = builder.build().expect("build should succeed");
        (built.mesh, built.face_ids[0])
    }

    /// Twice the signed area of a triangle of corners, in the XY plane.
    fn corner_tri_area2(mesh: &Mesh, tri: [CornerId; 3]) -> f32 {
        let p = |c: CornerId| {
            let v = mesh.to_vertex(c).expect("corner has vertex");
            *mesh.vertex_position(v).expect("vertex is live")
        };
        let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }

    const L_POINTS: [[f32; 2]; 6] = [
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 1.0],
        [1.0, 1.0],
        [1.0, 2.0],
        [0.0, 2.0],
    ];

    #[test]
    fn face_triangles_fan_matches_triangulate_face_fan() {
        let (mesh, face) = single_ngon(&L_POINTS);
        assert_eq!(
            mesh.face_triangles(face, FaceTriangulation::Fan),
            mesh.triangulate_face_fan(face),
            "Fan strategy must stay byte-identical to the historical fan"
        );
    }

    #[test]
    fn face_triangles_robust_handles_concave_ngon() {
        let (mesh, face) = single_ngon(&L_POINTS);

        // The fan produces at least one inverted triangle on this L.
        let fan = mesh.face_triangles(face, FaceTriangulation::Fan);
        assert!(
            fan.iter().any(|&t| corner_tri_area2(&mesh, t) < 0.0),
            "fixture must actually defeat the fan"
        );

        // Robust: all triangles positive and the areas sum to the polygon.
        let (robust, fell_back) = mesh.face_triangles_counted(face, FaceTriangulation::Robust);
        assert!(!fell_back, "simple polygon must not fall back");
        assert_eq!(robust.len(), L_POINTS.len() - 2);
        let mut area2 = 0.0;
        for &t in &robust {
            let a2 = corner_tri_area2(&mesh, t);
            assert!(a2 > 0.0, "robust triangle {t:?} must not invert");
            area2 += a2;
        }
        assert!((area2 - 6.0).abs() < 1e-5, "area sum {area2} must be 6");
    }

    #[test]
    fn face_triangles_robust_is_deterministic_and_orientation_stable() {
        let (mesh, face) = single_ngon(&L_POINTS);
        let a = mesh.face_triangles(face, FaceTriangulation::Robust);
        let b = mesh.face_triangles(face, FaceTriangulation::Robust);
        assert_eq!(a, b);

        // A face whose Newell normal points -Z (clockwise in XY) must keep
        // its own winding: build the mirrored L and check triangle
        // orientation matches the (negative) face orientation.
        let reversed: Vec<[f32; 2]> = L_POINTS.iter().rev().copied().collect();
        let (mesh_cw, face_cw) = single_ngon(&reversed);
        let (robust, fell_back) =
            mesh_cw.face_triangles_counted(face_cw, FaceTriangulation::Robust);
        assert!(!fell_back);
        for &t in &robust {
            assert!(
                corner_tri_area2(&mesh_cw, t) < 0.0,
                "triangles must wind with the face loop, not against it"
            );
        }
    }

    #[test]
    fn face_triangles_robust_falls_back_on_nonsimple_faces() {
        // A bowtie ngon: topologically a valid loop, geometrically
        // self-intersecting, so the projected polygon is not simple.
        let (mesh, face) = single_ngon(&[[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]]);
        let (robust, fell_back) = mesh.face_triangles_counted(face, FaceTriangulation::Robust);
        assert!(fell_back, "non-simple projection must fall back to fan");
        assert_eq!(robust, mesh.triangulate_face_fan(face));
    }

    #[test]
    fn triangulate_face_fan_quad() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 1.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder.add_face(&[0, 1, 2, 3]).expect("quad face is valid");
        let built = builder.build().expect("build should succeed");
        let face = built.face_ids[0];

        let triangles = built.mesh.triangulate_face_fan(face);
        assert_eq!(triangles.len(), 2);
        let loop_corners = built.mesh.face_loop(face).collect::<Vec<_>>();
        assert_eq!(
            triangles[0],
            [loop_corners[0], loop_corners[1], loop_corners[2]]
        );
        assert_eq!(
            triangles[1],
            [loop_corners[0], loop_corners[2], loop_corners[3]]
        );
    }

    #[test]
    fn triangulate_face_fan_pentagon_and_hexagon_counts() {
        let mut pent_builder = MeshBuilder::new();
        for i in 0..5 {
            pent_builder.push_vertex([i as f32, 0.0, 0.0]);
        }
        pent_builder
            .add_face(&[0, 1, 2, 3, 4])
            .expect("pentagon is valid");
        let pent = pent_builder.build().expect("build should succeed");
        assert_eq!(pent.mesh.triangulate_face_fan(pent.face_ids[0]).len(), 3);

        let mut hex_builder = MeshBuilder::new();
        for i in 0..6 {
            hex_builder.push_vertex([i as f32, 0.0, 0.0]);
        }
        hex_builder
            .add_face(&[0, 1, 2, 3, 4, 5])
            .expect("hexagon is valid");
        let hex = hex_builder.build().expect("build should succeed");
        assert_eq!(hex.mesh.triangulate_face_fan(hex.face_ids[0]).len(), 4);
    }

    #[test]
    fn triangulate_face_fan_is_deterministic() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([2.0, 0.0, 0.0]);
        builder.push_vertex([3.0, 0.0, 0.0]);
        builder.push_vertex([4.0, 0.0, 0.0]);
        builder
            .add_face(&[0, 1, 2, 3, 4])
            .expect("pentagon is valid");
        let built = builder.build().expect("build should succeed");
        let face = built.face_ids[0];

        let a = built.mesh.triangulate_face_fan(face);
        let b = built.mesh.triangulate_face_fan(face);
        assert_eq!(a, b);
    }

    #[test]
    fn mesh_builder_detects_non_manifold_edge() {
        let mut builder = MeshBuilder::new();
        for position in [
            [0.0, 0.0, 0.0],  // 0
            [1.0, 0.0, 0.0],  // 1
            [0.0, 1.0, 0.0],  // 2
            [0.0, -1.0, 0.0], // 3
            [1.0, 1.0, 0.0],  // 4
        ] {
            let _ = builder.push_vertex(position);
        }
        builder.add_face(&[0, 1, 2]).expect("f0");
        builder.add_face(&[1, 0, 3]).expect("f1");
        builder.add_face(&[0, 1, 4]).expect("f2");
        assert!(matches!(
            builder.build(),
            Err(BuildError::NonManifoldEdge {
                a: 0,
                b: 1,
                count: 3
            })
        ));
    }

    #[test]
    fn mesh_builder_closed_box_has_no_boundary() {
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
        // Outward-oriented quads.
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
        let result = builder.build().expect("box build");
        let mesh = result.mesh;
        assert_eq!(mesh.faces.len(), 6);
        assert_eq!(
            mesh.half_edges
                .iter()
                .filter(|(_, h)| h.face == FaceId::OUTSIDE)
                .count(),
            0
        );
        assert_eq!(count_boundary_loops(&mesh), 0);
    }

    #[test]
    fn mesh_builder_open_cylinder_side_has_two_boundary_loops() {
        let mut builder = MeshBuilder::new();
        // bottom ring 0..4, top ring 4..8
        for position in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [-1.0, 0.0, 1.0],
            [0.0, -1.0, 1.0],
        ] {
            let _ = builder.push_vertex(position);
        }
        for face in [[0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]] {
            builder.add_face(&face).expect("side face");
        }
        let result = builder.build().expect("cylinder side build");
        let mesh = result.mesh;
        assert_eq!(count_boundary_loops(&mesh), 2);
    }

    #[test]
    fn mesh_builder_provenance_maps_builder_indices() {
        let mut builder = MeshBuilder::new();
        let _ = builder.push_vertex([0.0, 0.0, 0.0]);
        let _ = builder.push_vertex([1.0, 0.0, 0.0]);
        let _ = builder.push_vertex([1.0, 1.0, 0.0]);
        let _ = builder.push_vertex([0.0, 1.0, 0.0]);
        builder.add_face(&[0, 1, 2, 3]).expect("quad");
        let result = builder.build().expect("build");
        assert_eq!(result.vertex_ids.len(), 4);
        assert_eq!(result.face_ids.len(), 1);
        assert_eq!(result.face_edge_ids.len(), 1);
        assert_eq!(result.face_edge_ids[0].len(), 4);
        let face = result.face_ids[0];
        let loop_len = result.mesh.face_loop(face).count();
        assert_eq!(loop_len, 4);
    }

    #[test]
    fn validate_fast_and_deep_pass_on_valid_mesh() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("valid mesh builds");
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn validate_fast_reports_twin_and_capacity_issues() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle builds");
        let first = mesh
            .half_edges
            .iter()
            .next()
            .map(|(id, _)| HalfEdgeId::from(id))
            .expect("has half-edge");
        let bad_twin = mesh.next(first).expect("next exists");
        mesh.half_edges.get_mut(first.as_id()).expect("live").twin = bad_twin;
        mesh.attrs.sync_capacities(0, 0, 0);

        let errors = mesh.validate_fast();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::TwinMismatch { half_edge, .. } if *half_edge == first.index()))
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::DenseCapacityMismatch { .. }))
        );
    }

    #[test]
    fn validate_deep_reports_degree_and_boundary_issues() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle builds");
        let face = mesh
            .faces
            .iter()
            .next()
            .map(|(id, _)| FaceId::from(id))
            .expect("face");
        mesh.faces.get_mut(face.as_id()).expect("face live").degree = 99;

        let outside = mesh
            .half_edges
            .iter()
            .find_map(|(id, h)| (h.face == FaceId::OUTSIDE).then_some(HalfEdgeId::from(id)))
            .expect("outside edge exists");
        let outside_twin = mesh.twin(outside).expect("outside has twin");
        mesh.half_edges
            .get_mut(outside.as_id())
            .expect("outside live")
            .twin = outside;
        mesh.half_edges
            .get_mut(outside_twin.as_id())
            .expect("twin live")
            .twin = outside_twin;

        let errors = mesh.validate_deep();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::FaceDegreeMismatch { face: f, .. } if *f == face.index()))
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::BoundaryInvariantBroken { half_edge } if *half_edge == outside.index()))
        );
    }

    #[test]
    fn validate_deep_does_not_duplicate_boundary_error_for_stale_twin() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle builds");
        let outside = mesh
            .half_edges
            .iter()
            .find_map(|(id, h)| (h.face == FaceId::OUTSIDE).then_some(HalfEdgeId::from(id)))
            .expect("outside edge exists");
        mesh.half_edges
            .get_mut(outside.as_id())
            .expect("outside live")
            .twin = HalfEdgeId::INVALID;

        let errors = mesh.validate_deep();
        assert!(errors.iter().any(|e| matches!(
                e,
                ValidationError::InvalidReference {
                    owner: "half_edge",
                    owner_index,
                    field: "twin",
                    ..
                } if *owner_index == outside.index()
        )));
        assert!(!errors.iter().any(|e| matches!(
            e,
            ValidationError::BoundaryInvariantBroken { half_edge } if *half_edge == outside.index()
        )));
    }

    #[test]
    fn sync_attr_capacities_uses_slot_count_after_tombstones() {
        let mut mesh = Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let v2 = mesh.add_vertex([2.0, 0.0, 0.0]);
        let removed = mesh.vertices.remove(v1.as_id());
        assert!(removed.is_some());
        mesh.sync_attr_capacities();

        let custom = AttrKey::<f32>::new(Domain::Vertex, "vertex.test.weight");
        assert_eq!(mesh.attrs_mut().define_dense(custom, 0.0), Ok(()));
        assert_eq!(mesh.attrs().dense(custom).expect("layer exists").len(), 3);
        assert_eq!(
            mesh.attrs()
                .dense(custom)
                .expect("layer exists")
                .get(v0.as_id()),
            Some(&0.0)
        );
        assert_eq!(
            mesh.attrs()
                .dense(custom)
                .expect("layer exists")
                .get(v2.as_id()),
            Some(&0.0)
        );
    }

    #[test]
    fn add_face_with_attrs_applies_region_and_edge_metadata() {
        let mut builder = MeshBuilder::new();
        let _ = builder.push_vertex([0.0, 0.0, 0.0]);
        let _ = builder.push_vertex([1.0, 0.0, 0.0]);
        let _ = builder.push_vertex([1.0, 1.0, 0.0]);
        let _ = builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face_with_attrs(
                &[0, 1, 2, 3],
                &FaceBuildAttrs {
                    region: Some(7),
                    edge_seams: Some(&[true, false, true, false]),
                    edge_sharpness: Some(&[1.0, 0.0, 2.5, 0.5]),
                },
            )
            .expect("face attrs should be valid");

        let result = builder.build().expect("mesh should build");
        let face = result.face_ids[0];
        let region = result
            .mesh
            .attrs()
            .dense(crate::attr::FACE_REGION)
            .expect("FACE_REGION layer must exist")
            .get(face.as_id())
            .copied()
            .expect("face region must exist");
        assert_eq!(region, 7);

        for (edge_index, half_edge) in result.face_edge_ids[0].iter().enumerate() {
            let seam = result
                .mesh
                .edge_seam(*half_edge)
                .expect("edge should be live");
            let sharpness = result
                .mesh
                .edge_sharpness(*half_edge)
                .expect("edge should be live");
            assert_eq!(seam, [true, false, true, false][edge_index]);
            assert!((sharpness - [1.0, 0.0, 2.5, 0.5][edge_index]).abs() <= 1.0e-6);
        }
    }

    #[test]
    fn add_face_with_attrs_rejects_edge_metadata_length_mismatch() {
        let mut builder = MeshBuilder::new();
        let _ = builder.push_vertex([0.0, 0.0, 0.0]);
        let _ = builder.push_vertex([1.0, 0.0, 0.0]);
        let _ = builder.push_vertex([0.0, 1.0, 0.0]);

        let error = builder
            .add_face_with_attrs(
                &[0, 1, 2],
                &FaceBuildAttrs {
                    edge_seams: Some(&[true, false]),
                    ..FaceBuildAttrs::default()
                },
            )
            .expect_err("mismatched seam length should be rejected");
        assert_eq!(
            error,
            BuildError::InvalidFaceAttrs {
                face: 0,
                kind: FaceAttrErrorKind::EdgeSeamLengthMismatch {
                    expected: 3,
                    actual: 2,
                },
            }
        );
    }
}
