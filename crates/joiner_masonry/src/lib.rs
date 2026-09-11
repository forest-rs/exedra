// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Masonry coursing and circular opening surrounds for [`joiner`].
//!
//! This crate owns the unit layout and coordinated recipes of a running-bond
//! wall. Site layout, constructive algorithms, material appearance and
//! structural capacity belong to callers and the other layers.
//!
//! [`RunningBondRule`] expands a geometry-free wall through an
//! [`joiner::RelationKind::ElementUnits`] relation. One optional circular
//! opening and its radial surround use the same dimensions. Generated units
//! retain course/column identities; the logical wall emits no duplicate solid.
//! These rules describe geometry and mortar gaps, not a load-bearing design.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod opening;
mod wall;

pub use exedra_measurements::Length;
pub use opening::CircularOpening;
pub use wall::{RunningBondParams, RunningBondRule};

#[cfg(test)]
mod tests;
