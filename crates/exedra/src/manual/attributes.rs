// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Attributes and Domains
//!
//! Exedra attribute storage is typed and domain-scoped:
//! - key type: [`AttrKey<T>`](crate::attributes::AttrKey)
//! - storage: [`Attributes`](crate::attributes::Attributes)
//! - domains: [`Domain`](crate::attributes::Domain)
//!
//! # Dense vs Sparse
//!
//! - Dense layers store one value per slot index in a domain.
//! - Sparse layers store only explicitly-authored values.
//!
//! Typical mapping:
//! - dense: required baseline data (for example vertex positions),
//! - sparse: optional overlays (for example per-corner UV overrides).
//!
//! # Built-ins
//!
//! Exedra registers built-in keys in [`attr`](crate::attr), including:
//! - [`VERTEX_POSITION`](crate::attr::VERTEX_POSITION),
//! - [`VERTEX_SHARPNESS`](crate::attr::VERTEX_SHARPNESS),
//! - [`CORNER_UV`](crate::attr::CORNER_UV),
//! - [`FACE_REGION`](crate::attr::FACE_REGION),
//! - [`EDGE_SEAM`](crate::attr::EDGE_SEAM),
//! - [`EDGE_SHARPNESS`](crate::attr::EDGE_SHARPNESS).
//!
//! Edge sharpness uses sparse `f32` values on canonical undirected edges:
//! - `0.0` means smooth,
//! - values above `0.0` mean authored sharpness.
//!
//! Vertex sharpness uses sparse `f32` values on vertices:
//! - absence means downstream subdivision code derives the class from incident
//!   edge sharpness,
//! - `0.0` is an explicit smooth override,
//! - `f32::INFINITY` represents an authored corner pin.
//!
//! # Capacity Model
//!
//! Dense capacities are based on arena slot counts, not live counts. This
//! keeps stable ID index space addressable even with tombstones.
//!
//! # Example
//!
//! ```rust
//! use exedra::attributes::{AttrKey, Attributes, Domain};
//!
//! let mut attrs = Attributes::new();
//! let key = AttrKey::<u32>::new(Domain::Face, "custom.region");
//! attrs.define_dense(key, 0)?;
//! attrs.sync_capacities(0, 2, 0);
//! assert_eq!(attrs.domain_capacity(Domain::Face), 2);
//! # Ok::<(), exedra::attributes::AttrError>(())
//! ```
