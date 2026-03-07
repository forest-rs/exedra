// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Edit Scopes and Change Sets
//!
//! Exedra edits are performed through [`EditSession`](crate::EditSession), obtained from
//! [`Mesh::edit`](crate::Mesh::edit) or [`Mesh::edit_with`](crate::Mesh::edit_with).
//!
//! ```rust
//! use exedra::{op, ChangeSetBuilder, Mesh};
//!
//! let mut mesh = Mesh::new();
//! let mut edit = mesh.edit_with(ChangeSetBuilder::new());
//! let v = op::add_vertex(&mut edit, [0.0, 0.0, 0.0]);
//! let v1 = op::add_vertex(&mut edit, [1.0, 0.0, 0.0]);
//! let v2 = op::add_vertex(&mut edit, [0.0, 1.0, 0.0]);
//! let face = op::add_face(&mut edit, &[v, v1, v2]).expect("triangle should be accepted");
//! let change_set = edit.finish();
//!
//! assert_eq!(change_set.created_vertices, vec![v, v1, v2]);
//! assert_eq!(change_set.created_faces, vec![face]);
//! ```
//!
//! [`EditSession`](crate::EditSession) is the eager edit host. The public kernel operation catalog
//! lives in [`crate::op`], which defines explicit topology edits such as
//! [`crate::op::add_face`], [`crate::op::split_edge`], and [`crate::op::delete_faces`].
//! `EditSession` is not a second public mutation catalog.
//!
//! # Eager Semantics
//!
//! Edit scopes are eager in v0.1:
//! - mutating calls update the underlying mesh immediately,
//! - dropping an edit scope does not roll back mesh edits,
//! - [`EditSession::finish`](crate::EditSession::finish) finalizes optional change recording
//!   and increments mesh revision.
//!
//! # Change Summaries
//!
//! [`ChangeSet`](crate::ChangeSet) records:
//! - created/deleted topology IDs,
//! - conservative dirty channels through [`DirtySet`](crate::DirtySet).
//!
//! These summaries are intended for incremental systems (cache invalidation,
//! downstream dependency updates) rather than full recomputation.
//!
//! # Propagation Policy
//!
//! Edit kernels that create/transform topology consume
//! [`PropagatePolicy`](crate::PropagatePolicy) as explicit per-call input.
//!
//! `split_edge` is a topology-transforming kernel available through
//! [`crate::op::split_edge`]. It inserts a midpoint vertex,
//! rewires the local half-edge pair, updates dirty/change tracking, and applies
//! propagation defaults/overrides for edge/corner attributes.
//!
//! [`crate::op::split_face`] inserts a diagonal between two
//! non-adjacent corners of one interior face and creates a second face.
//! Existing corners keep authored values; diagonal corners are populated from
//! policy rules while preserving sparse missingness.
//!
//! [`crate::op::add_face`] builds a new interior polygon loop from live
//! [`VertexId`](crate::VertexId)s. It reuses compatible OUTSIDE half-edges
//! when filling/opening boundaries and creates new interior+OUTSIDE pairs
//! otherwise.
//!
//! For edge sharpness in v0.1:
//! - [`EdgeAttrPropagation::Inherit`](crate::EdgeAttrPropagation::Inherit)
//!   preserves authored sharpness as-is (modeling-friendly default),
//! - [`EdgeAttrPropagation::DecayOnSplit`](crate::EdgeAttrPropagation::DecayOnSplit)
//!   applies subdivision-style decay (`sharpness = max(sharpness - 1.0, 0.0)`),
//! - [`EdgeAttrPropagation::Clear`](crate::EdgeAttrPropagation::Clear) resets
//!   sharpness to `0.0`.
//!
//! For `split_face`, diagonal sharpness is smooth by default (`Inherit`/`Clear`)
//! and only derives from nearby authored sharpness under `DecayOnSplit`.
