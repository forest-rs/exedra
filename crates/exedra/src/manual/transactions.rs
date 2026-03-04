// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Transactions and Change Sets
//!
//! Exedra edits are performed through [`Txn`](crate::Txn), obtained from
//! [`Mesh::begin`](crate::Mesh::begin).
//!
//! ```rust
//! use exedra::Mesh;
//!
//! let mut mesh = Mesh::new();
//! let mut txn = mesh.begin();
//! let v = txn.add_vertex([0.0, 0.0, 0.0]);
//! let change_set = txn.commit();
//!
//! assert_eq!(change_set.created_vertices, vec![v]);
//! ```
//!
//! # Eager Semantics
//!
//! Transactions are eager in v0.1:
//! - mutating txn calls update the underlying mesh immediately,
//! - dropping/aborting a transaction does not roll back mesh edits,
//! - [`Txn::commit`](crate::Txn::commit) finalizes deterministic bookkeeping
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
//! [`PropagatePolicy`](crate::PropagatePolicy). Configure per-transaction
//! behavior via [`Txn::set_propagate_policy`](crate::Txn::set_propagate_policy)
//! and inspect with [`Txn::propagate_policy`](crate::Txn::propagate_policy).
//!
//! `split_edge` is the first topology-transforming kernel using this policy:
//! [`Txn::split_edge`](crate::Txn::split_edge). It inserts a midpoint vertex,
//! rewires the local half-edge pair, updates dirty/change tracking, and applies
//! propagation defaults/overrides for edge/corner attributes.
//!
//! [`Txn::split_face`](crate::Txn::split_face) inserts a diagonal between two
//! non-adjacent corners of one interior face and creates a second face.
//! Existing corners keep authored values; diagonal corners are populated from
//! policy rules while preserving sparse missingness.
//!
//! [`Txn::add_face`](crate::Txn::add_face) builds a new interior polygon loop
//! from live [`VertexId`](crate::VertexId)s. It reuses compatible OUTSIDE
//! half-edges when filling/opening boundaries and creates new interior+OUTSIDE
//! pairs otherwise.
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
