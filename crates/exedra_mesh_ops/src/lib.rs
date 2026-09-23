// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reusable mesh modeling and geometric queries.
//!
//! Operations act directly on [`exedra_mesh::Mesh`] with explicit geometric
//! parameters, policies, scratch storage and typed outcomes. They do not require
//! a retained recipe, operation runner, or application context. Construction
//! provenance and application execution belong to callers.
//!
//! The mesh kernel owns topology and primitive edits; this crate composes them
//! into modeling algorithms. See [`boolean`] and [`round`] for their supported
//! geometry and failure guarantees.
//!
//! # Direct editing
//!
//! ```rust
//! use exedra_mesh::{Mesh, PropagatePolicy};
//! use exedra_mesh_ops::{
//!     face_edit::{extrude_faces, ExtrudeFacesParams, ExtrudeMode},
//!     normal_edit::{edit_normals, NormalEdit, NormalEditParams},
//! };
//!
//! let mut mesh = Mesh::from_polygons(
//!     &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
//!     &[&[0, 1, 2, 3]],
//! )?;
//! let faces = mesh.faces().collect();
//! let mut edit = mesh.edit();
//! let (_, extruded) = extrude_faces(&mut edit, &ExtrudeFacesParams {
//!     faces, mode: ExtrudeMode::KeepSource, distance: 0.5,
//! }, &PropagatePolicy::default())?;
//! edit_normals(&mut edit, &NormalEditParams {
//!     faces: extruded.cap_faces, mode: NormalEdit::Face,
//! })?;
//! let _: () = edit.finish();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Kernel edit sessions apply changes eagerly. Prepared operations check their
//! source before editing; later kernel failures may leave partial edits. See each
//! operation's failure contract and finish the caller's change sink on error too.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra_mesh_ops requires either the `std` or `libm` feature");

pub mod boolean;
pub mod bridge;
pub mod clearance;
pub mod components;
pub mod face_edit;
pub mod inspect;
pub mod junction;
#[cfg(test)]
mod layer_tests;
mod layers;
mod math;
pub mod measure;
pub mod normal_edit;
mod patch;
pub mod planar;
pub mod poke;
pub mod polygon;
pub mod region;
pub mod round;
pub mod section;
pub mod selection;
pub mod stretch;
pub mod transform;
pub mod uv;
pub mod workplane;

pub use boolean::{
    Aabb, BooleanBroadPhaseStats, BooleanBvh, BooleanCandidatePair, BooleanScratch,
    BooleanTriangleRef,
};
pub use round::{
    RoundError, RoundFaceSource, RoundKind, RoundPolicy, RoundResult, RoundStats, round_edges,
    round_sharp_edges,
};
