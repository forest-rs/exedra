// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Scene-independent geometry compilation, extraction and retained queries.
//!
//! This crate owns content-addressed compilation and immutable geometry snapshots.
//! It owns no scene hierarchy, placements, material bindings or authored identity.
//! Supply an explicit borrowed input list; input-local handles are never persistent
//! identity. Callers own names, structure and appearance. Recipes and baked meshes
//! share extraction, reports, cache retention and geometric queries.
//!
//! Render-only compilation releases topology. Query snapshots retain it and
//! preserve source correspondence. Old snapshots survive cache eviction; new
//! compilations create fresh selection scopes, including cache hits. `no_std`
//! requires `alloc` and pointer-width atomics for shared `Arc` ownership.
//!
//! ```
//! use exedra_compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource};
//! use exedra_mesh::Mesh;
//! let source = PartSource::Baked(Mesh::new());
//! let inputs = [PartInput { key: "detail", source: &source }];
//! let snapshot = PartCompiler::new().compile_snapshot(&inputs, &CompilePolicy::default())?;
//! assert_eq!(snapshot.compiled().part(PartId(0)).unwrap().triangle_count(), 0);
//! # Ok::<(), exedra_compile::CompileError>(())
//! ```

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod compile;
pub use compile::{
    BodyLookupError, CompilationMismatch, CompileCounters, CompileError, CompilePolicy,
    CompiledBody, CompiledPart, CompiledParts, GeometrySnapshot, PartCompiler, PartFingerprint,
    PartSnapshot, PolicyFingerprint, RegionRange, SignedVolume, SnapshotBody,
    SnapshotSurfaceInventory, SnapshotWorkplane, StaleSelection, VolumeError, part_fingerprint,
    policy_fingerprint,
};

/// Index in one explicit compilation input list.
/// Handles may be reused by a later list; content fingerprints establish correspondence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartId(pub u32);

/// Unresolved index into the source geometry's material-slot declarations.
/// Material binding belongs to the caller, independently of geometry compilation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotIndex(pub u32);

/// Independently usable geometry intent or baked topology.
#[derive(Clone, Debug)]
pub enum PartSource {
    /// Immutable construction intent evaluated under an explicit policy.
    Recipe(exedra_constructive::ir::Recipe),
    /// Existing topology with attributes, extracted without recipe evaluation.
    Baked(exedra_mesh::Mesh),
}

/// Borrowed geometry and diagnostic context for one compilation input.
/// The key appears in failures; geometry sharing uses content, independently of names.
#[derive(Clone, Copy, Debug)]
pub struct PartInput<'a> {
    /// Caller-owned diagnostic name, with no imposed scene-path semantics.
    pub key: &'a str,
    /// Recipe or baked mesh; no ownership transfer or geometry cloning is required.
    pub source: &'a PartSource,
}

pub(crate) fn len_u32(n: usize) -> u32 {
    u32::try_from(n).expect("geometry input and extraction counts fit u32")
}

#[cfg(test)]
mod tests;
