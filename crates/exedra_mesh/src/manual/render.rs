// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Render Extraction
//!
//! Exedra converts polygonal topology into deterministic triangle buffers with
//! [`Mesh::to_trimesh`](crate::Mesh::to_trimesh), returning:
//! - [`TriMesh`](crate::TriMesh)
//! - [`ExtractStats`](crate::ExtractStats)
//!
//! Parameters are passed via [`ExtractParams`](crate::ExtractParams).
//!
//! # Split Semantics
//!
//! Render vertices are keyed by `(VertexId, corner_uv_bits, corner_normal_bits)`
//! plus the bits of every carried attribute value. This means one topology
//! vertex can map to multiple render vertices when corner UVs, corner normals,
//! or carried values differ across incident faces.
//!
//! # Carried Attributes
//!
//! [`ExtractParams::attributes`](crate::ExtractParams::attributes) lists
//! further attribute layers, such as `attr::CORNER_UV1`, `attr::CORNER_COLOR`,
//! or caller-defined vertex, face, and corner layers, to emit as
//! [`TriMesh::attributes`](crate::TriMesh::attributes) streams. Each
//! [`ExtractAttribute`](crate::ExtractAttribute) names the value emitted where
//! the layer has none; [`ExtractStats`](crate::ExtractStats) counts those
//! fallbacks, missing layers, and attribute-driven splits. Write
//! caller-defined layers with `op::set_attribute` inside an edit scope.
//!
//! # Source Policies
//!
//! A corner's UV and normal are whatever the extraction policies resolve for
//! it, and the key sees the resolved values:
//! - [`NormalsSource`](crate::NormalsSource) chooses derived normals, authored
//!   overrides, or authored-with-derived-fallback.
//! - [`UvSource`](crate::UvSource) chooses what a corner without an authored
//!   `CORNER_UV` emits. `CustomOnly` (the default) emits `[0.0, 0.0]`;
//!   `CustomOrBoxProjected { scale }` projects the corner position on its
//!   face's dominant-axis plane using
//!   [`dominant_box_plane`](crate::dominant_box_plane), the same rule as the
//!   `uv.box` operator. Authored UVs are never overwritten.
//!
//! # Determinism
//!
//! Ordering is deterministic for fixed mesh state:
//! - faces in slot order,
//! - per-face fan triangulation order,
//! - first-encounter render vertex append order.
//!
//! # Example
//!
//! ```rust
//! use exedra_mesh::{ExtractParams, Mesh};
//!
//! let mesh = Mesh::from_indexed_triangles(
//!     &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
//!     &[[0, 1, 2]],
//!     &Default::default(),
//! )?;
//! let (tri, stats) = mesh.to_trimesh(&ExtractParams::default());
//! assert_eq!(tri.indices, vec![0, 1, 2]);
//! assert_eq!(stats.triangle_count, 1);
//! # Ok::<(), exedra_mesh::BuildError>(())
//! ```
