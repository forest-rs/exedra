// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Selection Semantics
//!
//! Cambium uses canonical face/edge sets:
//! - [`FaceSet`](crate::FaceSet)
//! - [`EdgeSet`](crate::EdgeSet)
//!
//! Canonicalization helpers:
//! - [`canonicalize_face_set`](crate::canonicalize_face_set)
//! - [`canonicalize_edge_set`](crate::canonicalize_edge_set)
//!
//! # Why Canonicalize
//!
//! Canonical sets (sorted + deduplicated) provide:
//! - deterministic processing order,
//! - deterministic reporting counters,
//! - stable behavior across equivalent caller inputs.
//!
//! # Operator Guidance
//!
//! - Canonicalize selection inputs at operator boundaries.
//! - Increment `selections_canonicalized` when canonicalization changes input.
//! - Treat stale IDs as precondition failures.
//!
//! # Example
//!
//! ```rust
//! use cambium::{FaceSet, canonicalize_face_set};
//! use exedra::{FaceId, Id};
//! use core::num::NonZeroU32;
//!
//! let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
//! let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
//! let mut faces: FaceSet = vec![f1, f0, f1];
//! let changed = canonicalize_face_set(&mut faces);
//! assert!(changed);
//! assert_eq!(faces, vec![f0, f1]);
//! ```
