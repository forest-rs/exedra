// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Topology Model
//!
//! Exedra uses a half-edge topology with stable generational IDs:
//! - vertices: [`VertexId`](crate::VertexId)
//! - directed edges/corners: [`HalfEdgeId`](crate::HalfEdgeId) /
//!   [`CornerId`](crate::CornerId)
//! - faces: [`FaceId`](crate::FaceId)
//!
//! Core modeling rules:
//! - interior faces are closed `next` cycles,
//! - every half-edge has a twin,
//! - boundary twins are explicit half-edges with face
//!   [`FaceId::OUTSIDE`](crate::FaceId::OUTSIDE).
//!
//! # Manifold Assumptions
//!
//! Exedra targets orientable 2-manifold surface topology with explicit
//! boundaries:
//! - each undirected edge is represented by exactly two half-edges,
//! - interior edges have two interior incident faces,
//! - boundary edges have one interior face and one
//!   [`FaceId::OUTSIDE`](crate::FaceId::OUTSIDE) twin.
//!
//! Construction APIs reject non-manifold edge multiplicity, and validation
//! reports it via [`ValidationError::EdgeMultiplicity`](crate::ValidationError::EdgeMultiplicity).
//!
//! # Corners
//!
//! Corners are represented by half-edge IDs in Exedra
//! ([`CornerId`](crate::CornerId) == [`HalfEdgeId`](crate::HalfEdgeId)).
//! A face of degree `N` has `N` half-edges and therefore `N` corners. This
//! enables per-corner data (for example UVs) where values can differ per face
//! even when a topology vertex is shared.
//!
//! # Traversal
//!
//! Typical traversal APIs on [`Mesh`](crate::Mesh):
//! - face loop: [`Mesh::face_loop`](crate::Mesh::face_loop)
//! - vertex star: [`Mesh::vertex_star`](crate::Mesh::vertex_star)
//! - fan triangulation: [`Mesh::triangulate_face_fan`](crate::Mesh::triangulate_face_fan)
