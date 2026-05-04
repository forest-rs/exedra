// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Built-in attribute key definitions.
//!
//! This module defines canonical key names and domains for layers that Exedra
//! treats as part of core mesh semantics. Keeping these keys centralized avoids
//! string drift across crates and gives callers one stable place to import them.

use crate::attributes::{AttrKey, Domain};

/// Required dense vertex positions.
pub const VERTEX_POSITION: AttrKey<[f32; 3]> = AttrKey::new(Domain::Vertex, "vertex.position");

/// Optional explicit vertex sharpness value.
///
/// Stored sparsely on vertex IDs. Absence means downstream subdivision
/// classifiers should derive the vertex class from incident edge sharpness.
/// `0.0` is an explicit smooth override; positive values mean increasingly
/// sharp, with `f32::INFINITY` representing a pinned corner.
pub const VERTEX_SHARPNESS: AttrKey<f32> = AttrKey::new(Domain::Vertex, "vertex.sharpness");

/// Optional corner UV coordinates.
pub const CORNER_UV: AttrKey<[f32; 2]> = AttrKey::new(Domain::HalfEdge, "corner.uv");

/// Optional authored corner normal overrides.
pub const CORNER_NORMAL_OVERRIDE: AttrKey<[f32; 3]> =
    AttrKey::new(Domain::HalfEdge, "corner.normal_override");

/// Optional explicit edge seam tag.
///
/// Stored sparsely on canonical half-edge IDs (one per undirected edge).
pub const EDGE_SEAM: AttrKey<bool> = AttrKey::new(Domain::HalfEdge, "edge.seam");

/// Optional explicit edge sharpness value.
///
/// Stored sparsely on canonical half-edge IDs (one per undirected edge).
/// `0.0` means smooth; positive values mean increasingly sharp.
pub const EDGE_SHARPNESS: AttrKey<f32> = AttrKey::new(Domain::HalfEdge, "edge.sharpness");

/// Dense face region/material identifier.
pub const FACE_REGION: AttrKey<u32> = AttrKey::new(Domain::Face, "face.region");
