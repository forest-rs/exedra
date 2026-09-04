// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Validation Strategy
//!
//! Exedra provides two validation passes on [`Mesh`](crate::Mesh):
//! - [`Mesh::validate_fast`](crate::Mesh::validate_fast): cheap structural checks
//! - [`Mesh::validate_deep`](crate::Mesh::validate_deep): deeper graph-walk checks
//!
//! Both return structured [`ValidationError`](crate::ValidationError) values.
//!
//! # When to Use Which
//!
//! - Use `validate_fast` in hot paths and frequent guardrails.
//! - Use `validate_deep` in debug flows, tests, and post-operation audits.
//!
//! `validate_deep` includes `validate_fast` checks and then adds deeper
//! invariant coverage.
//!
//! # Example
//!
//! ```rust
//! use exedra_mesh::Mesh;
//!
//! let mesh = Mesh::new();
//! assert!(mesh.validate_fast().is_empty());
//! assert!(mesh.validate_deep().is_empty());
//! ```
