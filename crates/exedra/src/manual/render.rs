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
//! Render vertices are keyed by `(VertexId, corner_uv_bits, corner_normal_bits)`.
//! This means one topology vertex can map to multiple render vertices when
//! corner UVs or corner normals differ across incident faces.
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
//! use exedra::{ExtractParams, Mesh};
//!
//! let mesh = Mesh::from_indexed_triangles(
//!     &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
//!     &[[0, 1, 2]],
//!     &Default::default(),
//! )?;
//! let (tri, stats) = mesh.to_trimesh(&ExtractParams::default());
//! assert_eq!(tri.indices, vec![0, 1, 2]);
//! assert_eq!(stats.triangle_count, 1);
//! # Ok::<(), exedra::BuildError>(())
//! ```
