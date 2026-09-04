// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Application-facing entry point for the Exedra modeling suite.
//!
//! Exedra brings together a mesh kernel and opt-in modeling heads under stable
//! namespaces. It is deliberately a thin facade: every algorithm, data model,
//! workflow, and conversion remains owned by the crate that defines it.
//!
//! # Choosing a surface
//!
//! - [`mesh`] is always available and contains the mesh kernel API.
//! - [`constructive`] contains immutable recipes and deterministic evaluation.
//! - [`assembly`] contains parts, instances, material bindings, and flattening.
//! - [`ops`] contains workflow-oriented mesh operations and adapters.
//! - [`primitives`] contains deterministic primitive mesh generators (opt-in).
//! - [`analytic`] and [`isosurface`] contain alternative geometry heads (opt-in).
//! - [`gltf`] contains assembly export (opt-in).
//! - `serde` exposes constructive and assembly interchange modules.
//!
//! The root intentionally exposes only the primary anchors: [`Mesh`], and,
//! when enabled, [`Recipe`] and [`Assembly`]. Use the corresponding namespace
//! for all other types and functions. This keeps imports clear about which
//! domain owns a behavior.
//!
//! # Backends and features
//!
//! Exedra requires one numeric backend. The default `std` backend suits host
//! applications; `libm` supports `no_std` applications. The default feature
//! set is `std`, `assembly`, and `ops`; `assembly` also selects
//! `constructive` because its public API admits [`Recipe`]. `analytic`,
//! `isosurface`, `primitives`, and `gltf` remain opt-in. Enabling `analytic`,
//! `constructive`, or `assembly` alongside `ops` also enables the matching
//! workflow adapter in [`ops`]. The `gltf` feature selects `assembly` and
//! `std`; `serde` selects `std` and exposes the constructive and assembly
//! interchange modules.
//!
//! # Example
//!
//! An assembly can mix generated recipe geometry and a directly supplied mesh
//! without making either representation the facade's responsibility:
//!
//! ```
//! use exedra::{Assembly, Mesh, Recipe};
//! use exedra::constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};
//!
//! let mut builder = RecipeBuilder::new();
//! let root = builder.add(NodeKind::Primitive {
//!     spec: PrimitiveSpec::Box {
//!         size: [1.0, 1.0, 1.0],
//!     },
//!     placement: Placement3::IDENTITY,
//! })?;
//! let recipe: Recipe = builder.finish(root)?;
//!
//! let mut assembly = Assembly::new();
//! assembly.add_recipe_part("body", recipe)?;
//! assembly.add_baked_part("detail", Mesh::new(), &[])?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! See the crate [README](https://github.com/forest-rs/exedra/tree/main/crates/exedra)
//! and [facade ADR](https://github.com/forest-rs/exedra/blob/main/crates/exedra/docs/adr-0001-facade-boundary.md)
//! for feature selection and the facade boundary.

#![no_std]

#[cfg(feature = "std")]
extern crate std;

#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra requires either the `std` or `libm` feature");

/// Mesh topology, attributes, construction, editing, and extraction.
///
/// This namespace is always available. It is the direct surface of the mesh
/// kernel; use it when the application needs explicit topology-level control.
pub use exedra_mesh as mesh;

/// Mesh kernel anchor type.
///
/// For construction, attributes, operations, and related types, use [`mesh`].
pub use exedra_mesh::Mesh;

/// Immutable constructive recipes, profiles, evaluation, and tessellation.
#[cfg(feature = "constructive")]
pub use exedra_constructive as constructive;

/// Immutable constructive-recipe anchor type.
///
/// Enabled by the `constructive` feature. For recipe construction and
/// evaluation, use [`constructive`].
#[cfg(feature = "constructive")]
pub use exedra_constructive::ir::Recipe;

/// Parts, instance trees, material binding, compilation, and flattening.
#[cfg(feature = "assembly")]
pub use exedra_assembly as assembly;

/// Assembly anchor type.
///
/// Enabled by the `assembly` feature. For part and instance APIs, use
/// [`assembly`].
#[cfg(feature = "assembly")]
pub use exedra_assembly::Assembly;

/// Workflow-oriented mesh operations and enabled cross-domain adapters.
#[cfg(feature = "ops")]
pub use exedra_ops as ops;

/// Planar analytic topology and tessellation.
#[cfg(feature = "analytic")]
pub use exedra_analytic as analytic;

/// Implicit fields, field composition, and isosurface extraction.
#[cfg(feature = "isosurface")]
pub use exedra_isosurface as isosurface;

/// Deterministic primitive mesh generators and their semantic selections.
#[cfg(feature = "primitives")]
pub use exedra_primitives as primitives;

/// glTF and GLB export from assembly render lists.
///
/// This namespace requires the `gltf` feature, which also selects `std`.
#[cfg(feature = "gltf")]
pub use exedra_gltf as gltf;

#[cfg(test)]
mod feature_contract_tests {
    #[cfg(all(feature = "analytic", feature = "ops"))]
    #[test]
    fn analytic_head_enables_its_ops_adapter() {
        use core::mem::size_of;

        // The facade promises that pairing a native head with `ops` exposes
        // that head's workflow adapter without another feature selection.
        let _ = size_of::<crate::ops::analytic::AnalyticFaceId>();
    }

    #[cfg(all(feature = "assembly", feature = "ops"))]
    #[test]
    fn assembly_head_enables_its_ops_adapter() {
        use core::mem::size_of;

        // The default facade must include the assembly workflow adapter, not
        // merely the assembly data model and the mesh-only operation surface.
        let _ = size_of::<crate::ops::assembly::PlacementSite>();
    }

    #[cfg(all(feature = "constructive", feature = "ops"))]
    #[test]
    fn constructive_head_enables_its_ops_adapter() {
        use core::mem::size_of;

        // Assembly implies constructive, so the default facade must expose
        // the matching recipe workflow adapter as part of that implication.
        let _ = size_of::<crate::ops::constructive::RecipePlan>();
    }

    #[cfg(feature = "primitives")]
    #[test]
    fn primitives_feature_exposes_the_generator_namespace() {
        use core::mem::size_of;

        // Primitive construction is application-facing but opt-in; this pins
        // its direct namespace without folding generators into the mesh core.
        let _ = size_of::<crate::primitives::BoxParams>();
    }
}
