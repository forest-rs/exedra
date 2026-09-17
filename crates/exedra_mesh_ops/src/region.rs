// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Region and boundary selection queries over plain meshes.

use crate::selection::{EdgeSet, FaceSet, canonicalize_edge_set, canonicalize_face_set};
use exedra_mesh::{FaceId, HalfEdgeId, Mesh};

/// Work performed by a region or boundary query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionQueryStats {
    /// Faces examined for a global region query, or returned by a flood fill.
    pub faces_processed: u64,
    /// One when the result needed sorting or duplicate removal; otherwise zero.
    pub selections_canonicalized: u64,
}

/// Typed failure from a region or boundary query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionQueryError {
    /// The required dense face-region layer is absent.
    MissingRegionAttribute,
    /// A live seed face has no region value.
    MissingRegionValue {
        /// Seed face.
        face: FaceId,
    },
    /// A traversed boundary edge has no canonical twin pair.
    InvalidCanonicalEdge {
        /// Boundary edge.
        edge: HalfEdgeId,
    },
    /// The kernel refused boundary traversal.
    BoundaryLoop(exedra_mesh::BoundaryLoopError),
    /// The kernel refused region traversal.
    ConnectedFaceRegion(exedra_mesh::ConnectedFaceRegionError),
}
impl core::fmt::Display for SelectionQueryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingRegionAttribute => f.write_str("missing face.region layer"),
            Self::MissingRegionValue { face } => write!(f, "missing region for face {face:?}"),
            Self::InvalidCanonicalEdge { edge } => {
                write!(f, "invalid canonical boundary edge {edge:?}")
            }
            Self::BoundaryLoop(error) => write!(f, "boundary selection: {error}"),
            Self::ConnectedFaceRegion(error) => write!(f, "region selection: {error}"),
        }
    }
}
impl core::error::Error for SelectionQueryError {}

/// Deterministic face-selection query result.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegionSelection {
    /// Canonical face IDs matching the requested region.
    pub faces: FaceSet,
    /// Query counters.
    pub counters: SelectionQueryStats,
}

/// Deterministic boundary-loop edge-selection query result.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EdgeLoopSelection {
    /// Canonical undirected edge IDs in the selected loop.
    pub edges: EdgeSet,
    /// Query counters.
    pub counters: SelectionQueryStats,
}

/// Deterministic region flood-fill query result.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegionFloodSelection {
    /// Region identifier used by the flood fill.
    pub region_id: u32,
    /// Canonical connected face IDs matching `region_id`.
    pub faces: FaceSet,
    /// Query counters.
    pub counters: SelectionQueryStats,
}

/// Returns all faces tagged with `region_id`, sorted and deduplicated.
pub fn select_faces_by_region(
    mesh: &Mesh,
    region_id: u32,
) -> Result<RegionSelection, SelectionQueryError> {
    let layer = mesh
        .attrs()
        .dense(exedra_mesh::attr::FACE_REGION)
        .ok_or(SelectionQueryError::MissingRegionAttribute)?;
    let mut result = RegionSelection::default();
    for face in mesh.faces() {
        result.counters.faces_processed += 1;
        if layer
            .get(face.as_id())
            .is_some_and(|&value| value == region_id)
        {
            result.faces.push(face);
        }
    }
    result.counters.selections_canonicalized = u64::from(canonicalize_face_set(&mut result.faces));
    Ok(result)
}

/// Returns canonical undirected edges of the boundary loop containing the seed.
///
/// Output is sorted by edge ID, not traversal order. Use [`Mesh::boundary_loop`]
/// for an ordered, directed loop. Interior or stale seeds are typed failures.
pub fn select_boundary_edge_loop(
    mesh: &Mesh,
    seed_edge: HalfEdgeId,
) -> Result<EdgeLoopSelection, SelectionQueryError> {
    let mut result = EdgeLoopSelection::default();
    for edge in mesh
        .boundary_loop(seed_edge)
        .map_err(SelectionQueryError::BoundaryLoop)?
    {
        result.edges.push(
            mesh.canonical_edge(edge)
                .ok_or(SelectionQueryError::InvalidCanonicalEdge { edge })?,
        );
    }
    result.counters.selections_canonicalized = u64::from(canonicalize_edge_set(&mut result.edges));
    Ok(result)
}

/// Returns the connected component sharing the seed face's region.
pub fn flood_fill_faces_by_region(
    mesh: &Mesh,
    seed_face: FaceId,
) -> Result<RegionFloodSelection, SelectionQueryError> {
    let mut faces = mesh
        .connected_face_region(seed_face)
        .map_err(SelectionQueryError::ConnectedFaceRegion)?;
    let layer = mesh
        .attrs()
        .dense(exedra_mesh::attr::FACE_REGION)
        .ok_or(SelectionQueryError::MissingRegionAttribute)?;
    let region_id = *layer
        .get(seed_face.as_id())
        .ok_or(SelectionQueryError::MissingRegionValue { face: seed_face })?;
    let counters = SelectionQueryStats {
        faces_processed: faces.len() as u64,
        selections_canonicalized: u64::from(canonicalize_face_set(&mut faces)),
    };
    Ok(RegionFloodSelection {
        region_id,
        faces,
        counters,
    })
}
