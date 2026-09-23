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
///
/// Each corner belongs to the half-edge's destination vertex within its face;
/// use [`crate::Mesh::to_vertex`] to obtain the corresponding position.
pub const CORNER_UV: AttrKey<[f32; 2]> = AttrKey::new(Domain::HalfEdge, "corner.uv");

/// Optional second corner UV set, for example a lightmap or detail chart.
///
/// Like [`CORNER_UV`], each corner belongs to the half-edge's destination
/// vertex within its face. Render extraction emits it only when carried by an
/// [`crate::ExtractAttribute`]. Topology edits carry it by the rule declared
/// with [`crate::Mesh::set_layer_propagation`] (see
/// [`crate::attributes::Propagation`]).
pub const CORNER_UV1: AttrKey<[f32; 2]> = AttrKey::new(Domain::HalfEdge, "corner.uv1");

/// Optional corner color as linear, straight-alpha RGBA.
///
/// Render extraction emits it only when carried by an
/// [`crate::ExtractAttribute`]. Topology edits carry it by the rule declared
/// with [`crate::Mesh::set_layer_propagation`] (see
/// [`crate::attributes::Propagation`]).
pub const CORNER_COLOR: AttrKey<[f32; 4]> = AttrKey::new(Domain::HalfEdge, "corner.color");

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

/// Built-in keys with dedicated edit operations and invariants.
///
/// Generic attribute operations such as [`crate::op::set_attribute`] and
/// layer registration through [`crate::Mesh::define_dense_layer`] refuse
/// these; use the specific operation instead (for example
/// [`crate::op::set_corner_uv`]). [`CORNER_UV1`] and [`CORNER_COLOR`] are
/// conventions without extra invariants and are written generically.
pub const RESERVED: &[(Domain, &str)] = &[
    (VERTEX_POSITION.domain(), VERTEX_POSITION.name()),
    (VERTEX_SHARPNESS.domain(), VERTEX_SHARPNESS.name()),
    (CORNER_UV.domain(), CORNER_UV.name()),
    (
        CORNER_NORMAL_OVERRIDE.domain(),
        CORNER_NORMAL_OVERRIDE.name(),
    ),
    (EDGE_SEAM.domain(), EDGE_SEAM.name()),
    (EDGE_SHARPNESS.domain(), EDGE_SHARPNESS.name()),
    (FACE_REGION.domain(), FACE_REGION.name()),
];

/// True when `(domain, name)` is listed in [`RESERVED`].
#[must_use]
pub fn is_reserved(domain: Domain, name: &str) -> bool {
    RESERVED
        .iter()
        .any(|(reserved_domain, reserved)| *reserved_domain == domain && *reserved == name)
}
