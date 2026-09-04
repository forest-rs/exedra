// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Core half-edge topology records stored in Exedra arenas.

use crate::{FaceId, HalfEdgeId, VertexId};

/// Vertex topology record.
///
/// [`Vertex::out`] references one outgoing half-edge for
/// [`Mesh::vertex_star`](crate::Mesh::vertex_star) traversal.
/// Stored in the mesh vertex arena and exposed mainly for diagnostics/tests.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Vertex {
    /// One outgoing half-edge from this vertex.
    pub out: HalfEdgeId,
}

/// Half-edge topology record.
///
/// Stored in the mesh half-edge arena and exposed mainly for
/// diagnostics/tests.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct HalfEdge {
    /// Destination vertex of this directed half-edge.
    pub to: VertexId,
    /// Owning face (`FaceId::OUTSIDE` for boundary half-edges).
    pub face: FaceId,
    /// Next half-edge in the owning face loop.
    pub next: HalfEdgeId,
    /// Opposite half-edge across the undirected edge.
    pub twin: HalfEdgeId,
}

/// Face topology record.
///
/// Stored in the mesh face arena and exposed mainly for diagnostics/tests.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Face {
    /// One half-edge on this face's loop.
    pub edge: HalfEdgeId,
    /// Cached face degree (loop length).
    pub degree: u32,
}
