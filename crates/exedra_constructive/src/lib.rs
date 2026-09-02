// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Constructive geometry head: an immutable recipe IR with deterministic
//! tessellation into Exedra meshes.
//!
//! This crate is a *compiler target*: external geometry frontends build 2D
//! profiles and constructive bodies into recipes, and evaluation tessellates
//! them into [`exedra::Mesh`] values with full provenance, semantic regions,
//! and fidelity reporting.
//!
//! The crate owns:
//! - kurbo-backed 2D profiles closed by construction,
//! - the constructive node set (primitives, extrude, revolve, loft, sweep,
//!   planar faces, grids, stretch, CSG, transforms, instances) and its
//!   content-addressed identity,
//! - deterministic evaluation and tessellation with source maps and reports.
//!
//! It intentionally does not own: mesh topology (that is [`exedra`]),
//! polygon triangulation (that is `exedra_triangulate`), any source vocabulary
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
//!
//! ## Integrating an external compiler
//!
//! The full walkthrough lives in `docs/integration-guide.md`. The core
//! loop is: intern your opaque identities, build validated profiles and
//! nodes, freeze, evaluate:
//!
//! ```
//! use exedra_constructive::builders;
//! use exedra_constructive::evaluate::{Fidelity, evaluate};
//! use exedra_constructive::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};
//! use exedra_constructive::tessellate::EvalPolicy;
//!
//! let mut b = RecipeBuilder::new();
//! let source = b.source_ref("yourspec:part/7#body");
//! let front = b.material_slot("front");
//! let profile = b.add_profile(builders::rounded_rect(600.0, 400.0, 40.0).unwrap());
//! let node = b
//!     .with_source(source)
//!     .with_material(front)
//!     .add(NodeKind::Extrude {
//!         profile,
//!         placement: Placement3::IDENTITY,
//!         height: 720.0,
//!         caps: CapMode::Both,
//!     })
//!     .unwrap();
//! let recipe = b.finish(node).unwrap();
//!
//! let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
//! assert_eq!(result.bodies.len(), 1);
//! assert_eq!(result.report.fidelity_of(node), Some(Fidelity::Exact));
//! assert!(result.bodies[0].body.mesh.validate_deep().is_empty());
//! ```

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub use kurbo;

pub mod builders;
pub mod cache;
pub mod discretize;
pub mod evaluate;
#[cfg(test)]
mod goldens;
#[cfg(test)]
mod hostile;
pub mod interchange;
pub mod ir;
pub mod offset;
pub mod profile;
pub mod source_map;
mod stretch;
pub mod tessellate;
pub mod text;

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
pub const EVAL_SCHEMA_VERSION: u32 = 6;
