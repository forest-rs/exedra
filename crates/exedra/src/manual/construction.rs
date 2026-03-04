// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction Pathways
//!
//! Exedra provides two main construction paths:
//! - indexed triangles: [`Mesh::from_indexed_triangles`](crate::Mesh::from_indexed_triangles)
//! - polygon/ngon builder: [`MeshBuilder`](crate::MeshBuilder)
//!
//! # Indexed Triangles
//!
//! Use `from_indexed_triangles` for triangle input with optional weld tolerance
//! in [`BuildParams`](crate::BuildParams).
//!
//! On success, topology is stitched with explicit OUTSIDE boundaries. On
//! failure, structured [`BuildError`](crate::BuildError) variants report
//! causes such as out-of-bounds indices, degenerate triangles, non-manifold
//! edges, or boundary stitch ambiguity.
//!
//! # MeshBuilder
//!
//! Use [`MeshBuilder`](crate::MeshBuilder) when you need explicit polygon face
//! loops and provenance maps (`vertex_ids`, `face_ids`, `face_edge_ids`) from
//! [`MeshBuildResult`](crate::MeshBuildResult).
//!
//! # Example
//!
//! ```rust
//! use exedra::Mesh;
//!
//! let mesh = Mesh::from_indexed_triangles(
//!     &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
//!     &[[0, 1, 2]],
//!     &Default::default(),
//! )?;
//! assert_eq!(mesh.faces().count(), 1);
//! # Ok::<(), exedra::BuildError>(())
//! ```
