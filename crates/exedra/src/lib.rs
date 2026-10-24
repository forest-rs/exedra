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
//! - `constructive` contains immutable recipes and deterministic evaluation.
//! - `assembly` contains parts, instances, material bindings, and flattening.
//! - `compile` contains standalone compilation, extraction and retained queries.
//! - `mesh_ops` contains direct mesh modeling and geometric queries.
//! - `edit` contains opt-in command execution, preview and reports for editors.
//! - `primitives` contains deterministic primitive mesh generators (opt-in).
//! - `analytic` and `isosurface` contain alternative geometry heads (opt-in).
//! - `gltf` contains assembly export (opt-in).
//! - `serde` exposes constructive and assembly interchange modules.
//!
//! The root intentionally exposes only the primary anchors: [`Mesh`], and,
//! when enabled, `Recipe` and `Assembly`. Use the corresponding namespace
//! for all other types and functions. This keeps imports clear about which
//! domain owns a behavior.
//!
//! # Backends and features
//!
//! Exedra requires one numeric backend. The default `std` backend suits host
//! applications; `libm` supports `no_std` applications. The default feature
//! set is `std`, `compile`, and `mesh_ops`; `compile` also selects
//! `constructive`. Assembly is an optional structural surface above the same
//! geometry service, so `assembly` selects `compile`.
//! `edit`, `analytic`, `isosurface`, `primitives`, and `gltf` remain opt-in.
//! `gltf` selects `assembly` and `std`; `serde` selects `assembly` and `std`.
//! Standalone construction, compilation and extraction do not depend on
//! structural composition. Use `compile` for independent geometry inputs and
//! snapshots.
//!
//! # Example
//!
//! Compile recipe geometry and a baked mesh without creating a scene tree:
//!
//! ```
//! use exedra::{Mesh, Recipe};
//! use exedra::compile::{CompilePolicy, PartCompiler, PartInput, PartSource};
//! use exedra::constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};
//!
//! let mut builder = RecipeBuilder::new();
//! let root = builder.add(NodeKind::Primitive {
//!     spec: PrimitiveSpec::Box { size: [1.0; 3] },
//!     placement: Placement3::IDENTITY,
//! })?;
//! let recipe: Recipe = builder.finish(root)?;
//! let source = PartSource::Recipe(recipe);
//! let detail = PartSource::Baked(Mesh::new());
//! let inputs = [
//!     PartInput::new("body", &source),
//!     PartInput::new("detail", &detail),
//! ];
//! let geometry = PartCompiler::new().compile_snapshot(&inputs, &CompilePolicy::default())?;
//! assert_eq!(geometry.compiled().parts().len(), 2);
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

/// Standalone compilation, retained query snapshots and extraction buffers.
///
/// Enabled by `compile`, and also selected by `assembly`.
/// Standalone callers supply explicit geometry inputs without a scene tree.
#[cfg(feature = "compile")]
pub use exedra_compile as compile;

/// Parts, instance trees, material binding, compilation adapters, and flattening.
#[cfg(feature = "assembly")]
pub use exedra_assembly as assembly;

/// Assembly anchor type.
///
/// Enabled by the `assembly` feature. For part and instance APIs, use
/// [`assembly`].
#[cfg(feature = "assembly")]
pub use exedra_assembly::Assembly;

/// Direct mesh modeling, geometric queries and source correspondence.
#[cfg(feature = "mesh_ops")]
pub use exedra_mesh_ops as mesh_ops;

/// Opt-in mesh command execution, preview and reporting for editor applications.
#[cfg(feature = "edit")]
pub use exedra_edit as edit;

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
    #[cfg(feature = "primitives")]
    #[test]
    fn primitives_feature_exposes_the_generator_namespace() {
        use core::mem::size_of;

        // Primitive construction is application-facing but opt-in; this pins
        // its direct namespace without folding generators into the mesh core.
        let _ = size_of::<crate::primitives::BoxParams>();
    }
}
