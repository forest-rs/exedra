// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Built-in attribute key definitions.
//!
//! This module defines canonical key names and domains for layers that Exedra
//! treats as part of core mesh semantics. Keeping these keys centralized avoids
//! string drift across crates and gives callers one stable place to import them.

use crate::{AttrKey, Domain};

/// Required dense vertex positions.
pub const VERTEX_POSITION: AttrKey<[f32; 3]> = AttrKey::new(Domain::Vertex, "vertex.position");

/// Optional corner UV coordinates.
pub const CORNER_UV: AttrKey<[f32; 2]> = AttrKey::new(Domain::HalfEdge, "corner.uv");

/// Optional explicit edge seam tag.
///
/// Stored sparsely on canonical half-edge IDs (one per undirected edge).
pub const EDGE_SEAM: AttrKey<bool> = AttrKey::new(Domain::HalfEdge, "edge.seam");

/// Optional explicit edge sharpness tag.
///
/// Stored sparsely on canonical half-edge IDs (one per undirected edge).
pub const EDGE_SHARPNESS: AttrKey<bool> = AttrKey::new(Domain::HalfEdge, "edge.sharpness");

/// Dense face region/material identifier.
pub const FACE_REGION: AttrKey<u32> = AttrKey::new(Domain::Face, "face.region");
