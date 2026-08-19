// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Constructive geometry head: an immutable recipe IR with deterministic
//! tessellation into Exedra meshes.
//!
//! This crate is a *compiler target*: external frontends (geometry frontends,
//! parametric evaluators) build 2D profiles and constructive bodies into
//! recipes, and evaluation tessellates them into [`exedra::Mesh`] values
//! with full provenance, semantic regions, and fidelity reporting.
//!
//! The crate owns:
//! - kurbo-backed 2D profiles closed by construction,
//! - the constructive node set (extrude, revolve, loft, sweep, CSG,
//!   transforms, instances) and its content-addressed identity,
//! - deterministic evaluation and tessellation with source maps and reports.
//!
//! It intentionally does not own: mesh topology (that is [`exedra`]),
//! polygon triangulation (that is `exedra_triangulate`), any spec vocabulary
//! (frontends attach opaque source identities), scene assembly across parts,
//! or serialization formats beyond its own canonical encoding.
//!
//! ## Determinism
//!
//! Evaluation is a pure function of the recipe and its policy: identical
//! input bits produce bit-identical meshes on every platform and build mode.
//! Trig always routes through `libm` (never `std`), discretization counts
//! derive from parameters rather than accumulated floats, and iteration
//! never depends on hash order. [`EVAL_SCHEMA_VERSION`] stamps every content
//! hash so an upgrade to kurbo or to any discretization rule invalidates
//! caches and goldens explicitly.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub use kurbo;

pub mod builders;
pub mod discretize;
pub mod evaluate;
#[cfg(test)]
mod goldens;
pub mod ir;
pub mod profile;
pub mod source_map;
pub mod tessellate;

/// Narrows a validated count to `u32`.
///
/// Callers guarantee the `u32` budget structurally (validated collections).
pub(crate) fn len_u32(n: usize) -> u32 {
    debug_assert!(u32::try_from(n).is_ok(), "count budget validated upstream");
    #[expect(
        clippy::cast_possible_truncation,
        reason = "collection sizes are validated against the u32 budget at construction"
    )]
    {
        n as u32
    }
}

/// Version stamp folded into every content hash.
///
/// Bump on any change that can alter evaluation output for an unchanged
/// recipe: a kurbo upgrade, a discretization-rule change, a canonical
/// encoding change. Caches and goldens keyed on hashes then invalidate
/// explicitly instead of silently drifting.
pub const EVAL_SCHEMA_VERSION: u32 = 1;
