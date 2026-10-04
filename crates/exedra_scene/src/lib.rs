// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Optional native scenes over independently usable Exedra construction.
//!
//! `LayerStack` owns authored scene structure, composition, overrides and undo.
//! Exedra owns geometry intent; `exedra_compile` owns compilation, extraction,
//! retained queries and cache lifetimes.
//! This crate lowers typed scene inputs through a caller-owned shared compiler
//! and publishes conventional polygon meshes without an `Assembly` or a second
//! scene tree. The geometry crates have no dependency on this integration.
//!
//! The initial authored vocabulary is [`BOX_SCHEMA`] and [`CYLINDER_SCHEMA`],
//! both derived from USD `Xform`. [`SceneEvaluator::sync`] explicitly samples
//! their composed parameters and updates an exclusively owned local
//! generated layer. Transforms and material bindings remain native stage data;
//! changing them reuses local geometry. Returned snapshots retain recipes,
//! topology, source maps, extraction buffers and reports through shared snapshots.
//!
//! This is an initial scene-authoring surface, not a complete recipe schema or
//! asset importer. Direct callers continue using `Recipe`, `evaluate`, part
//! compilation and extraction without loading a scene implementation.

#![no_std]
extern crate alloc;

mod error;
mod geometry;
mod primitive;
mod runtime;

pub use error::{SceneError, SceneErrorKind};
pub use primitive::{
    BOX_SCHEMA, CYLINDER_SCHEMA, HEIGHT, RADIUS, SEGMENTS, SIZE, define_primitive, register_schemas,
};
pub use runtime::{GENERATED_CHILD, SceneEvaluator, SceneSnapshot, SourceGeometry, SyncStats};

/// Scene-independent compiler, geometry inputs and retained query snapshots.
pub use exedra_compile as compile;

/// Native layers, composition, paths, transactions and change reports.
pub use layerstack as composition;
pub use layerstack::{LiveStage, Stage};
/// Standard geometry and shading schemas, views, edit handles and queries.
pub use layerstack_schemas as schemas;
pub use layerstack_schemas::{Scene, Time};

#[cfg(test)]
mod tests;
