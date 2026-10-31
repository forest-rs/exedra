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
//! - `scene` integrates caller-owned native Layerstack stages with retained geometry.
//! - `mesh_ops` contains direct mesh modeling and geometric queries.
//! - `edit` contains opt-in command execution, preview and reports for editors.
//! - `primitives` contains deterministic primitive mesh generators (opt-in).
//! - `analytic` contains planar analytic topology (opt-in).
//! - `implicit` contains implicit fields written as formulas, and
//!   `isosurface` meshes them (opt-in; `isosurface` selects `implicit`).
//! - `curves` contains curve evaluation, sampling and frames, and in
//!   `curves::profile` the line and arc profiles both `constructive` and
//!   `analytic` accept; either feature enables it.
//! - `step` exchanges supported retained analytic boundaries (opt-in).
//! - `gltf` exports compiled geometry and caller-prepared structure (opt-in).
//! - `serde` exposes recipe interchange and forwards to optional Assembly interchange.
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
//! `edit`, `analytic`, `implicit`, `isosurface`, `primitives`, and `gltf`
//! remain opt-in.
//! `gltf` selects `compile` and `std`; `serde` selects `constructive` and `std`.
//! Neither selects `assembly`. Select `assembly,gltf` for the optional Assembly
//! export adapter, or `assembly,serde` for Assembly interchange. Standalone
//! construction, compilation, extraction and export do not depend on structural
//! composition. Use `compile` for independent geometry inputs and snapshots.
//!
//! # Example
//!
//! With `compile` enabled, compile recipe geometry and a baked mesh without
//! creating a scene tree:
//!
//! ```
//! # #[cfg(feature = "compile")]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use exedra::{Mesh, Recipe};
//! use exedra::compile::{CompilePolicy, PartCompiler, PartInput, PartSource};
//! use exedra::constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};
//!
//! let mut builder = RecipeBuilder::new();
//! let root = builder.add_node(NodeKind::Primitive {
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
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "compile"))]
//! # fn main() {}
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

/// Optional caller-owned Layerstack scene evaluation and mesh publication.
/// Geometry heads and default facade builds remain independent of USD composition.
#[cfg(feature = "scene")]
pub use exedra_scene as scene;

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

/// Analytic extrusions and planar shells with explicit mesh realization.
#[cfg(feature = "analytic")]
pub use exedra_analytic as analytic;

/// Curves and the line and circular-arc profiles shared by the constructive
/// and analytic heads.
#[cfg(any(feature = "analytic", feature = "constructive"))]
pub use exedra_curves as curves;

/// Implicit fields written once as formulas, and their composition.
#[cfg(feature = "implicit")]
pub use exedra_implicit as implicit;

/// Surface meshes extracted from implicit fields.
#[cfg(feature = "isosurface")]
pub use exedra_isosurface as isosurface;

/// Deterministic primitive mesh generators and their semantic selections.
#[cfg(feature = "primitives")]
pub use exedra_primitives as primitives;

/// glTF and GLB export from compiled geometry and caller-prepared structure.
///
/// This namespace requires the `gltf` feature, which also selects `std`.
#[cfg(feature = "gltf")]
pub use exedra_gltf as gltf;

/// STEP exchange from retained native boundaries, independent of display meshes.
///
/// This namespace requires `step`, which also selects `analytic`.
#[cfg(feature = "step")]
pub use exedra_step as step;

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
