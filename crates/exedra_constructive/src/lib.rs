// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Constructive geometry head: an immutable recipe IR with deterministic
//! tessellation into Exedra meshes.
//!
//! This crate is a *compiler target*: external geometry frontends build 2D
//! profiles and constructive bodies into recipes, and evaluation tessellates
//! them into [`exedra_mesh::Mesh`] values with full provenance, semantic regions,
//! and fidelity reporting.
//!
//! The crate owns:
//! - kurbo-backed 2D profiles closed by construction,
//! - the constructive node set (primitives, extrude, revolve, loft, sweep,
//!   planar faces, grids, implicit fields, stretch, CSG, transforms,
//!   instances) and its content-addressed identity,
//! - deterministic evaluation and tessellation with source maps and reports,
//! - evaluated-body plane sections and capped cuts preserving surface attribution,
//! - revision-checked planar workplanes with authored origin and in-plane axes.
//!
//! It intentionally does not own: mesh topology (that is [`exedra_mesh`]),
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
//! ## Recommended imports
//!
//! Most construction needs [`ir::RecipeBuilder`] and [`ir::Node`], the
//! [`ops`] records ([`ops::Extrude`], [`ops::Revolve`], [`ops::Csg`],
//! [`ops::Primitive`]), profiles from [`builders`] or drawn with
//! [`profile::LoopCursor`], and [`evaluate::evaluate`] with a
//! [`tessellate::EvalPolicy`]. Every [`ir::NodeKind`] remains available
//! directly, and the other modules own their advanced operations.
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
//! use exedra_constructive::ir::{Node, RecipeBuilder};
//! use exedra_constructive::ops::Extrude;
//! use exedra_constructive::tessellate::EvalPolicy;
//!
//! let mut b = RecipeBuilder::new();
//! let source = b.source_ref("yourspec:part/7#body");
//! let front = b.material_slot("front");
//! let profile = b.add_profile(builders::rounded_rect_from_corner(600.0, 400.0, 40.0).unwrap());
//! let body = Extrude::forward(profile, 720.0);
//! let node = b
//!     .add_node(Node::new(body).with_source(source).with_material(front))
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
#[cfg(test)]
mod cap_tests;
pub mod chart;
pub mod clearance;
pub mod discretize;
#[cfg(test)]
mod door_boolean_tests;
pub mod edge_finish;
pub mod evaluate;
pub mod extrude;
#[cfg(test)]
mod goldens;
#[cfg(test)]
mod hostile;
pub mod implicit;
mod import_mesh;
pub mod interchange;
pub mod ir;
pub mod loft;
#[cfg(test)]
mod material_tests;
pub mod measure;
pub mod offset;
pub mod ops;
#[cfg(test)]
mod panel_boolean_tests;
pub mod path;
#[cfg(test)]
mod pocket_tests;
pub mod profile;
pub mod profile_section;
pub mod section;
pub mod section_law;
pub mod source_map;
mod stretch;
pub mod tessellate;
pub mod text;
pub mod workplane;

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
///
/// Schema 42 samples curves through `exedra_curves`: profile cubics and
/// smooth-loft trajectories evaluate as its cubic spans and count their
/// edges by its chord bound, sweep-path cubics take its adaptive intervals,
/// and sweeps frame through its rotation-minimizing frames and the shared
/// double reflection. Cubic points, path stations and sweep frames may
/// change; lines and arcs are unchanged.
///
/// Schema 43 resolves every bulge arc's circle through
/// `exedra_curves::profile::Arc::from_bulge`: offset arcs' centers and radii
/// may change in their last bits; discretized arcs are unchanged.
pub const EVAL_SCHEMA_VERSION: u32 = 43;

#[cfg(test)]
mod diagonal_boolean_tests;
#[cfg(test)]
mod uv_mapping_tests;

#[cfg(test)]
mod rounded_post_tests;

#[cfg(test)]
mod retained_plane_tests;

#[cfg(test)]
mod attachment_tests;

#[cfg(test)]
mod surface_origin_tests;
