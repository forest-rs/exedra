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
