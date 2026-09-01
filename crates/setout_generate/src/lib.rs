// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic topology expansion from exact setting-out inputs.
//!
//! `setout_generate` turns resolved values into immutable fragments whose
//! identities do not depend on container positions. Linear generators place
//! either endpoint-inclusive stations or edge-to-edge bays along an exact
//! interval. Station positions and bay edges/centers remain rational numbers
//! of joto iotas because an interval does not necessarily divide into an
//! integral number of iotas.
//!
//! This crate does not evaluate [`setout`] relations, create construction
//! elements, or choose a floating-point lowering. A consumer adapter performs
//! that final translation once.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod delta;
mod key;
mod linear;

pub use delta::{DeltaError, FragmentDelta};
pub use key::{InvocationKey, ItemKey, ItemLabel, KeyError};
pub use linear::{
    GENERATION_SCHEMA_VERSION, GenerationError, ItemOverride, LinearBay, LinearBayDistribution,
    LinearBayFragment, LinearBayGenerator, LinearDistribution, LinearFragment, LinearGenerator,
    LinearStation, MAX_LINEAR_STATIONS, distribute_linear, distribute_linear_bays,
};
