// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Structure head: part definitions, instance trees, and material binding
//! over Exedra geometry.
//!
//! This crate owns *cross-part structure*: registering part definitions
//! (constructive recipes or baked meshes), arranging instances of them in
//! a tree under stable frontend-supplied keys, binding material slots to
//! opaque material keys, and flattening the result into an explicit render
//! list. It compiles parts once per distinct content and shares the result
//! across instances.
//!
//! It intentionally does not own: geometry math (that is [`exedra_mesh`] and
//! [`exedra_constructive`]), parameter models, conditional logic, or
//! pricing — those belong to the external runtimes that drive this crate —
//! nor rendering itself, which consumes the flattened output.
//!
//! ## Identity
//!
//! Every instance is addressed by an instance path: the sequence of
//! frontend-supplied string keys from the assembly root down to the
//! instance. Paths are the *stable identity contract*: the same path across
//! re-evaluations denotes the same logical part, regardless of insertion
//! order, sibling count, or geometry changes. Indices are never identity.
//!
//! ## Typical flow
//!
//! Register each distinct part once, place it any number of times, compile
//! part-local geometry, then flatten the instance tree for a renderer or
//! exporter:
//!
//! ```
//! use exedra_assembly::{Assembly, PartCompiler, flatten};
//! use exedra_constructive::{
//!     ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder},
//!     tessellate::EvalPolicy,
//! };
//!
//! let mut builder = RecipeBuilder::new();
//! let root = builder
//!     .add(NodeKind::Primitive {
//!         spec: PrimitiveSpec::Box { size: [1.0; 3] },
//!         placement: Placement3::IDENTITY,
//!     })
//!     .expect("valid box");
//! let recipe = builder.finish(root).expect("valid recipe");
//!
//! let mut assembly = Assembly::new();
//! let part = assembly.add_recipe_part("box", recipe).expect("unique part key");
//! assembly
//!     .add_instance(None, "left", part, Placement3::IDENTITY)
//!     .expect("unique root key");
//! assembly
//!     .add_instance(None, "right", part, Placement3::translate(2.0, 0.0, 0.0))
//!     .expect("unique root key");
//!
//! let compiled = PartCompiler::new()
//!     .compile_parts(&assembly, &EvalPolicy::default())
//!     .expect("part compiles");
//! let render_list = flatten(&assembly, &compiled);
//!
//! assert_eq!(compiled.part(part).unwrap().triangle_count(), 12);
//! assert_eq!(render_list.triangle_count(), 24);
//! ```

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod assembly;
pub mod compile;
pub mod flatten;
pub mod interchange;

pub use assembly::{
    Assembly, AssemblyError, Instance, InstanceId, InstancePath, PartDef, PartId, PartSource,
    SlotIndex,
};
pub use compile::assembly_fingerprint;
pub use compile::{
    CompileCounters, CompileError, CompiledBody, CompiledPart, CompiledParts, PartCompiler,
    PartFingerprint, PolicyFingerprint, RegionRange,
};
pub use flatten::{RenderItem, RenderList, ResolvedRegion, compose, flatten};

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
